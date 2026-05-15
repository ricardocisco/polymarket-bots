use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::{Duration, Utc};

use crate::config::Config;
use crate::feed::{ClobOrderbookFeed, GammaMarketsFeed};
use crate::strategy::{build_strategy_input, SixAmStrategy, Strategy};
use crate::types::{Action, BacktestResult, BacktestTrade, QuoteSnapshot, StrategyMarket};

pub async fn run_backtest(
    cfg: &Config,
    days: i64,
    gamma: &GammaMarketsFeed,
    clob: &ClobOrderbookFeed,
) -> Result<BacktestResult> {
    let start_min = Utc::now() - Duration::days(days);
    let end_max = Utc::now();
    let markets = gamma
        .fetch_markets_between(start_min, end_max, true)
        .await
        .context("falha ao buscar mercados fechados")?;

    run_backtest_with_markets(cfg, &markets, clob).await
}

pub async fn run_backtest_with_markets(
    cfg: &Config,
    markets: &[StrategyMarket],
    clob: &ClobOrderbookFeed,
) -> Result<BacktestResult> {
    let mut strategy = SixAmStrategy::new(cfg.clone());
    let candidates = markets
        .iter()
        .filter(|market| market.is_strategy_candidate(cfg.target_hour_utc))
        .cloned()
        .collect::<Vec<_>>();

    let mut result = BacktestResult {
        total_candidates: 0,
        total_trades: 0,
        winners: 0,
        pnl: 0.0,
        stake_total: 0.0,
        skip_reasons: BTreeMap::new(),
        trades: Vec::new(),
    };

    for market in candidates {
        result.total_candidates += 1;

        let Some(token_id) = market.token_id_for_direction(cfg.trade_direction) else {
            *result
                .skip_reasons
                .entry("sem token up/down".into())
                .or_default() += 1;
            continue;
        };

        let start_ts = market.start_date.timestamp() + cfg.entry_delay_secs;
        let end_ts = market.start_date.timestamp() + cfg.entry_window_secs;
        let history = match clob.price_history(&token_id, start_ts, end_ts, 1).await {
            Ok(history) => history,
            Err(err) => {
                let reason = if err.to_string().to_ascii_lowercase().contains("timed out") {
                    "timeout no historico de precos"
                } else {
                    "falha no historico de precos"
                };
                *result.skip_reasons.entry(reason.into()).or_default() += 1;
                continue;
            }
        };
        let Some(entry) = history.iter().min_by_key(|point| point.t) else {
            *result
                .skip_reasons
                .entry("sem preço de entrada".into())
                .or_default() += 1;
            continue;
        };

        let quote = QuoteSnapshot {
            best_bid: None,
            best_ask: Some(entry.p),
            last_price: Some(entry.p),
        };
        let input = build_strategy_input(&market, quote, market.start_date, cfg);
        let decision = strategy.decide(&input);

        if decision.action != Action::Buy {
            *result
                .skip_reasons
                .entry(decision.reason.clone())
                .or_default() += 1;
            continue;
        }

        let Some(final_price) = market.resolved_price_for_direction(cfg.trade_direction) else {
            *result
                .skip_reasons
                .entry("sem preço final".into())
                .or_default() += 1;
            continue;
        };

        result.total_trades += 1;
        let stake = cfg
            .position_size_usdc
            .to_string()
            .parse::<f64>()
            .unwrap_or_default();
        let shares = stake / entry.p;
        let trade_pnl = shares * final_price - stake;
        let won = final_price > 0.5;

        if won {
            result.winners += 1;
        }
        result.pnl += trade_pnl;
        result.stake_total += stake;
        result.trades.push(BacktestTrade {
            market_label: market.display_label(),
            entry_price: entry.p,
            final_price,
            pnl: trade_pnl,
            won,
        });
    }

    Ok(result)
}
