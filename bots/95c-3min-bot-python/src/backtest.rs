use anyhow::{Context, Result};
use chrono::{Duration, TimeZone, Utc};

use crate::config::Config;
use crate::feed::{BinanceFeed, ClobOrderbookFeed, GammaMarketsFeed};
use crate::strategy::{analyze_candles, SniperStrategy};
use crate::types::{Action, BacktestResult, BacktestTrade, PricePoint, StrategyMarket, TradeSide};

pub async fn run_backtest(
    cfg: &Config,
    days: i64,
    asset_filter: Option<&str>,
    interval_filter: Option<u32>,
    gamma: &GammaMarketsFeed,
    clob: &ClobOrderbookFeed,
    binance: &BinanceFeed,
) -> Result<BacktestResult> {
    let start_min = Utc::now() - Duration::days(days);
    let end_max = Utc::now();
    let markets = gamma
        .fetch_closed_markets(start_min, end_max, &cfg.markets)
        .await
        .context("falha ao buscar mercados fechados para backtest")?;

    run_backtest_with_markets(cfg, &markets, asset_filter, interval_filter, clob, binance).await
}

pub async fn run_backtest_with_markets(
    cfg: &Config,
    markets: &[StrategyMarket],
    asset_filter: Option<&str>,
    interval_filter: Option<u32>,
    clob: &ClobOrderbookFeed,
    binance: &BinanceFeed,
) -> Result<BacktestResult> {
    let strategy = SniperStrategy::new(cfg.clone());
    let mut result = BacktestResult {
        total_candidates: 0,
        total_trades: 0,
        winners: 0,
        pnl: 0.0,
        stake_total: 0.0,
        skip_reasons: Default::default(),
        trades: Vec::new(),
    };

    let asset_filter = asset_filter.map(|v| v.to_ascii_uppercase());

    for market in markets {
        if let Some(asset) = asset_filter.as_deref() {
            if market.config.asset.to_ascii_uppercase() != asset {
                continue;
            }
        }
        if let Some(interval) = interval_filter {
            if market.config.duration_minutes != interval {
                continue;
            }
        }
        if market.up_token_id.is_empty() || market.down_token_id.is_empty() {
            add_skip(&mut result, "sem token up/down");
            continue;
        }

        result.total_candidates += 1;
        let market = ensure_strike(market.clone(), binance).await;
        if market.strike_price <= 0.0 {
            add_skip(&mut result, "sem strike");
            continue;
        }

        let window_start = market.end_time.timestamp() - (cfg.max_minutes_left * 60.0) as i64;
        let window_end = market.end_time.timestamp() - (cfg.min_minutes_left * 60.0) as i64;
        let up_history = match clob
            .price_history(&market.up_token_id, window_start, window_end, 1)
            .await
        {
            Ok(history) => history,
            Err(_) => {
                add_skip(&mut result, "falha no historico UP");
                continue;
            }
        };
        let down_history = match clob
            .price_history(&market.down_token_id, window_start, window_end, 1)
            .await
        {
            Ok(history) => history,
            Err(_) => {
                add_skip(&mut result, "falha no historico DOWN");
                continue;
            }
        };

        let mut final_skip = "sem sinal na janela".to_string();
        let mut trade_done = false;

        for up_point in up_history
            .iter()
            .filter(|p| p.t >= window_start && p.t <= window_end)
        {
            let Some(down_price) = nearest_price(&down_history, up_point.t) else {
                continue;
            };
            let Some(now) = Utc.timestamp_opt(up_point.t, 0).single() else {
                continue;
            };

            let mut synthetic = market.clone();
            synthetic.up_price = up_point.p;
            synthetic.down_price = down_price;

            let candles = match binance
                .candles_until(&synthetic.config.binance_symbol, up_point.t, 50)
                .await
            {
                Ok(candles) if !candles.is_empty() => candles,
                _ => {
                    final_skip = "sem candles Binance".into();
                    continue;
                }
            };

            let Some(signal) = analyze_candles(
                &synthetic.config.binance_symbol,
                synthetic.strike_price,
                synthetic.minutes_left_at(now),
                &candles,
            ) else {
                final_skip = "analise sem sinal".into();
                continue;
            };

            let decision = strategy.decide(&synthetic, &signal, now);
            if decision.action != Action::Buy {
                final_skip = decision.reason;
                continue;
            }

            let side = decision.side.context("decisao de compra sem side")?;
            let stake = decision
                .stake_usdc
                .unwrap_or(cfg.position_size_usdc.min(cfg.bankroll));
            let entry_price = synthetic.price(side);
            if !(0.0..1.0).contains(&entry_price) {
                final_skip = "preco de entrada invalido".into();
                continue;
            }

            let final_underlying = binance
                .price_at(
                    &synthetic.config.binance_symbol,
                    synthetic.end_time.timestamp(),
                )
                .await
                .ok()
                .flatten()
                .unwrap_or(signal.current_price);
            let winner = synthetic.winner_from_outcome_prices().or_else(|| {
                if final_underlying > synthetic.strike_price {
                    Some(TradeSide::Up)
                } else if final_underlying < synthetic.strike_price {
                    Some(TradeSide::Down)
                } else {
                    None
                }
            });

            let shares = stake / entry_price;
            let payout = match winner {
                Some(winner) if winner == side => shares,
                Some(_) => 0.0,
                None => shares * 0.5,
            };
            let pnl = payout - stake;
            let won = winner == Some(side);

            result.total_trades += 1;
            result.stake_total += stake;
            result.pnl += pnl;
            if won {
                result.winners += 1;
            }
            result.trades.push(BacktestTrade {
                market_slug: synthetic.slug.clone(),
                asset: synthetic.config.asset.clone(),
                interval_minutes: synthetic.config.duration_minutes,
                entry_ts: up_point.t,
                side,
                entry_price,
                final_price: final_underlying,
                pnl,
                won,
                reason: decision.reason,
            });
            trade_done = true;
            break;
        }

        if !trade_done {
            add_skip(&mut result, final_skip);
        }
    }

    Ok(result)
}

async fn ensure_strike(mut market: StrategyMarket, binance: &BinanceFeed) -> StrategyMarket {
    if market.strike_price > 0.0 {
        return market;
    }
    let start_ts = market.start_time.timestamp();
    if let Ok(Some(price)) = binance
        .price_at(&market.config.binance_symbol, start_ts)
        .await
    {
        market.strike_price = price;
    }
    market
}

fn nearest_price(points: &[PricePoint], ts: i64) -> Option<f64> {
    points
        .iter()
        .filter(|point| (point.t - ts).abs() <= 90)
        .min_by_key(|point| (point.t - ts).abs())
        .map(|point| point.p)
}

fn add_skip(result: &mut BacktestResult, reason: impl Into<String>) {
    *result.skip_reasons.entry(reason.into()).or_default() += 1;
}
