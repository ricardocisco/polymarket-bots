use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::Utc;
use polymarket_bayesian::config::Config;
use polymarket_bayesian::execution::{ExchangeExecutor, OrderIntent};
use polymarket_bayesian::feed::{BinanceFeed, ClobOrderbookFeed, GammaMarketsFeed};
use polymarket_bayesian::settle::reconcile_due_trades;
use polymarket_bayesian::storage::PaperTradeStore;
use polymarket_bayesian::strategy::BayesianStrategy;
use polymarket_bayesian::types::Action;
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

    let cfg = Config::from_env().context("falha ao carregar config")?;
    info!(
        "bayesian rust | mode={} | dry_run={} | live={} | bankroll={:.2} | band {:.2}-{:.2}",
        cfg.mode.as_str(),
        cfg.dry_run,
        cfg.live_trading_enabled(),
        cfg.bankroll,
        cfg.min_buy_price,
        cfg.max_buy_price
    );

    let gamma = GammaMarketsFeed::new(&cfg.gamma_base_url)?;
    let clob = ClobOrderbookFeed::new(&cfg.clob_base_url)?;
    let binance = BinanceFeed::new(&cfg.binance_base_url)?;
    let executor = ExchangeExecutor::from_config(&cfg).await?;
    let store = PaperTradeStore::connect(&cfg.paper_trades_path).await?;
    let strategy = BayesianStrategy::new(cfg.clone());
    let mut traded_slugs = HashSet::<String>::new();

    loop {
        let settled = reconcile_due_trades(&store, &gamma, &binance).await?;
        if settled > 0 {
            info!("settle automatico liquidou {} trade(s)", settled);
        }

        let now = Utc::now();
        let open_positions = store.list_open_trades().await?.len();
        let mut active_found = false;

        for market_cfg in &cfg.markets {
            let Some(mut market) = gamma.find_active_market(market_cfg).await? else {
                continue;
            };
            active_found = true;

            if traded_slugs.contains(&market.slug) || store.has_open_trade(&market.slug).await? {
                continue;
            }

            if market.strike_price <= 0.0 {
                if let Ok(Some(price)) = binance
                    .price_at(&market.config.binance_symbol, market.start_time.timestamp())
                    .await
                {
                    market.strike_price = price;
                }
            }
            if market.strike_price <= 0.0 {
                warn!("{} | sem strike", market.slug);
                continue;
            }

            let candles = match binance
                .recent_candles(&market.config.binance_symbol, 100)
                .await
            {
                Ok(candles) if candles.len() >= 30 => candles,
                _ => {
                    warn!("{} | sem candles Binance", market.slug);
                    continue;
                }
            };

            let up_book = match clob.order_book_price(&market.up_token_id).await {
                Ok(book) => book,
                Err(err) => {
                    warn!("{} | sem book UP: {err}", market.slug);
                    continue;
                }
            };
            let down_book = match clob.order_book_price(&market.down_token_id).await {
                Ok(book) => book,
                Err(err) => {
                    warn!("{} | sem book DOWN: {err}", market.slug);
                    continue;
                }
            };

            let decision =
                strategy.decide(&market, &candles, &up_book, &down_book, open_positions, now);
            if decision.action != Action::Buy {
                info!("skip | {} | {}", market.display_label(), decision.reason);
                continue;
            }

            let side = decision.side.context("decisao de compra sem side")?;
            let price = decision.limit_price_cents.unwrap_or_default() / 100.0;
            let intent = OrderIntent {
                market_slug: market.slug.clone(),
                token_id: market.token_id(side).to_owned(),
                price,
                stake_usdc: decision.stake_usdc.unwrap_or(1.0),
                reason: decision.reason.clone(),
            };
            let update = executor.submit(&intent).await?;

            store
                .insert_open_trade(
                    &market,
                    now.timestamp(),
                    side,
                    (price * 100.0).round() as u32,
                    candles.last().map(|c| c.close).unwrap_or_default(),
                    market.minutes_left_at(now) * 60.0,
                    &decision,
                    cfg.dry_run,
                    update.order_id.clone(),
                    cfg.strategy_name(),
                    &cfg.strategy_version,
                )
                .await?;

            info!(
                "{} | {} | {} @ {:.3} | stake=${:.2} | simulated={}",
                if cfg.live_trading_enabled() {
                    "ordem enviada"
                } else {
                    "paper trade gravado"
                },
                market.slug,
                side,
                price,
                intent.stake_usdc,
                update.simulated
            );
            traded_slugs.insert(market.slug.clone());
            if traded_slugs.len() > 100 {
                traded_slugs.clear();
            }
        }

        if !active_found {
            info!("nenhum mercado ativo na janela monitorada");
        }

        tokio::time::sleep(std::time::Duration::from_secs(cfg.loop_interval_secs)).await;
    }
}
