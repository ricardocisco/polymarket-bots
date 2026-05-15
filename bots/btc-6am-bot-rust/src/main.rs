use std::collections::HashSet;

use anyhow::{Context, Result};
use btc_6am_bot::config::Config;
use btc_6am_bot::execution::{ExchangeExecutor, OrderIntent};
use btc_6am_bot::feed::{ClobOrderbookFeed, GammaMarketsFeed};
use btc_6am_bot::storage::{reconcile_due_trades, PaperTradeStore};
use btc_6am_bot::strategy::{
    build_strategy_input, is_entry_window_open, strategy_summary, SixAmStrategy, Strategy,
};
use btc_6am_bot::types::{Action, SignalDecision};
use chrono::{Duration, Utc};
use tracing::{info, warn};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .pretty()
        .init();

    let cfg = Config::from_env_without_private_key().context("falha ao carregar configuração")?;
    if cfg.live_trading_enabled() && cfg.private_key.trim().is_empty() {
        anyhow::bail!("POLYMARKET_PRIVATE_KEY não definida para live trading");
    }
    info!("{}", strategy_summary(&cfg));
    info!(
        "modo={} | dry_run={} | live_trading={}",
        if cfg.live_trading_enabled() {
            "real"
        } else {
            "paper"
        },
        cfg.dry_run,
        cfg.live_trading_enabled()
    );

    let gamma = GammaMarketsFeed::new(&cfg.gamma_base_url)?;
    let clob = ClobOrderbookFeed::new(&cfg.clob_base_url)?;
    let executor = ExchangeExecutor::from_config(&cfg).await?;
    let store = PaperTradeStore::connect(&cfg.paper_trades_path).await?;
    let mut strategy = SixAmStrategy::new(cfg.clone());

    let mut traded_today: HashSet<String> = HashSet::new();
    let mut current_day = Utc::now().date_naive();

    loop {
        let now = Utc::now();
        if now.date_naive() != current_day {
            current_day = now.date_naive();
            traded_today.clear();
        }

        let settled = reconcile_due_trades(&store, &gamma, now.timestamp()).await?;
        if settled > 0 {
            info!("reconciliação liquidou {} trade(s)", settled);
        }

        let target_start = current_day
            .and_hms_opt(cfg.target_hour_utc, 0, 0)
            .context("hora alvo inválida")?
            .and_utc();
        let target_end = target_start + Duration::hours(1);
        let active_window_start = target_start - Duration::minutes(1);
        let active_window_end = target_end + Duration::minutes(1);

        let active_cycle = now >= active_window_start && now <= active_window_end;
        let sleep_secs = if active_cycle {
            cfg.poll_interval_active_secs
        } else {
            cfg.poll_interval_idle_secs
        };

        if active_cycle {
            let markets = gamma
                .fetch_markets_between(target_start, target_end, false)
                .await
                .context("falha ao descobrir mercados ativos")?;

            for market in markets
                .into_iter()
                .filter(|market| market.is_strategy_candidate(cfg.target_hour_utc))
            {
                if traded_today.contains(&market.id) {
                    continue;
                }
                if traded_today.len() as u32 >= cfg.max_daily_trades {
                    warn!("limite diário de trades atingido");
                    break;
                }
                if !market.accepting_orders || !is_entry_window_open(&market, now, &cfg) {
                    continue;
                }
                if store.has_open_trade(&market.id).await? {
                    continue;
                }

                let Some(token_id) = market.token_id_for_direction(cfg.trade_direction) else {
                    continue;
                };

                let quote = match clob.quote_for_token(&token_id).await {
                    Ok(quote) => quote,
                    Err(err) => {
                        warn!("{} | erro no quote: {err}", market.display_label());
                        continue;
                    }
                };
                let input = build_strategy_input(&market, quote.clone(), now, &cfg);
                let decision = strategy.decide(&input);

                if decision.action != Action::Buy {
                    info!("skip | {} | {}", market.display_label(), decision.reason);
                    continue;
                }

                let Some(price_cents) = decision.limit_price_cents else {
                    continue;
                };
                let price = price_cents / 100.0;
                let intent = OrderIntent {
                    market_id: market.id.clone(),
                    market_ticker: market.display_label(),
                    token_id: token_id.clone(),
                    price,
                    stake_usdc: cfg.position_size_usdc,
                    reason: decision.reason.clone(),
                };
                let update = executor.submit(&intent).await?;
                strategy.on_order_submitted(&update, &quote);

                persist_trade(
                    &store,
                    &market,
                    &token_id,
                    now.timestamp(),
                    &decision,
                    &cfg,
                    input.seconds_to_close,
                )
                .await?;

                info!(
                    "{} | {} | price={:.3} | simulated={}",
                    if cfg.live_trading_enabled() {
                        "ordem enviada"
                    } else {
                        "paper trade gravado"
                    },
                    intent.reason,
                    price,
                    update.simulated
                );
                traded_today.insert(market.id);
            }
        } else {
            info!(
                "fora da janela ativa | agora={} UTC | hora alvo={:02}:00",
                now.format("%Y-%m-%d %H:%M:%S"),
                cfg.target_hour_utc
            );
        }

        tokio::time::sleep(std::time::Duration::from_secs(sleep_secs)).await;
    }
}

async fn persist_trade(
    store: &PaperTradeStore,
    market: &btc_6am_bot::types::StrategyMarket,
    token_id: &str,
    submitted_at: i64,
    decision: &SignalDecision,
    cfg: &Config,
    seconds_to_close: f64,
) -> Result<()> {
    let side = decision
        .side
        .context("decision side ausente em ação de compra")?;
    let entry_price_cents = decision.limit_price_cents.unwrap_or_default().round() as u32;

    store
        .insert_open_trade(
            market,
            token_id,
            submitted_at,
            side,
            entry_price_cents,
            seconds_to_close,
            decision,
            cfg.dry_run,
            "btc_6am",
            &cfg.strategy_version,
        )
        .await
}
