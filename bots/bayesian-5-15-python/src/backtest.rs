use anyhow::{Context, Result};
use chrono::{Duration, TimeZone, Utc};

use crate::config::Config;
use crate::feed::{BinanceFeed, ClobOrderbookFeed, GammaMarketsFeed};
use crate::strategy::BayesianStrategy;
use crate::types::{
    Action, BacktestResult, BacktestTrade, Candle, OrderBookPrice, PricePoint, StrategyMarket,
    TradeSide,
};

pub async fn run_backtest(
    cfg: &Config,
    days: i64,
    asset_filter: Option<&str>,
    interval_filter: Option<u32>,
    stake_override: Option<f64>,
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

    run_backtest_with_markets(
        cfg,
        &markets,
        asset_filter,
        interval_filter,
        stake_override,
        clob,
        binance,
    )
    .await
}

pub async fn run_backtest_with_markets(
    cfg: &Config,
    markets: &[StrategyMarket],
    asset_filter: Option<&str>,
    interval_filter: Option<u32>,
    stake_override: Option<f64>,
    clob: &ClobOrderbookFeed,
    binance: &BinanceFeed,
) -> Result<BacktestResult> {
    let strategy = BayesianStrategy::new(cfg.clone());
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

        let candles = match binance
            .candles_between(
                &market.config.binance_symbol,
                market.start_time.timestamp() - 3600,
                market.end_time.timestamp(),
            )
            .await
        {
            Ok(candles) if candles.len() >= 35 => candles,
            _ => {
                add_skip(&mut result, "candles insuficientes");
                continue;
            }
        };

        let up_history = match clob
            .price_history(
                &market.up_token_id,
                market.start_time.timestamp(),
                market.end_time.timestamp(),
                1,
            )
            .await
        {
            Ok(history) if !history.is_empty() => history,
            _ => {
                add_skip(&mut result, "sem historico UP");
                continue;
            }
        };
        let down_history = match clob
            .price_history(
                &market.down_token_id,
                market.start_time.timestamp(),
                market.end_time.timestamp(),
                1,
            )
            .await
        {
            Ok(history) if !history.is_empty() => history,
            _ => {
                add_skip(&mut result, "sem historico DOWN");
                continue;
            }
        };

        let mut final_skip = "sem entrada".to_string();
        if let Some(trade) = replay_market(
            cfg,
            &strategy,
            &market,
            &candles,
            &up_history,
            &down_history,
            stake_override,
            &mut final_skip,
        ) {
            result.total_trades += 1;
            result.stake_total += trade.stake_usdc;
            result.pnl += trade.pnl;
            if trade.won {
                result.winners += 1;
            }
            result.trades.push(trade);
        } else {
            add_skip(&mut result, final_skip);
        }
    }

    Ok(result)
}

fn replay_market(
    cfg: &Config,
    strategy: &BayesianStrategy,
    market: &StrategyMarket,
    candles: &[Candle],
    up_history: &[PricePoint],
    down_history: &[PricePoint],
    stake_override: Option<f64>,
    final_skip: &mut String,
) -> Option<BacktestTrade> {
    let latest_entry_ts = market.end_time.timestamp() - 120;

    for (idx, candle) in candles.iter().enumerate() {
        let ts = candle.open_time + 60;
        if ts <= market.start_time.timestamp() || ts >= latest_entry_ts || idx < 30 {
            continue;
        }
        let recent = &candles[idx.saturating_sub(99)..=idx];
        let up_price = nearest_price(up_history, ts)?;
        let down_price = nearest_price(down_history, ts)?;
        let up_book = synthetic_book(&market.up_token_id, up_price);
        let down_book = synthetic_book(&market.down_token_id, down_price);
        let now = Utc.timestamp_opt(ts, 0).single()?;

        let decision = strategy.decide(market, recent, &up_book, &down_book, 0, now);
        if decision.action != Action::Buy {
            *final_skip = decision.reason;
            continue;
        }

        let side = decision.side?;
        let entry_price = match side {
            TradeSide::Up => up_price,
            TradeSide::Down => down_price,
        };
        if !(cfg.min_buy_price..=cfg.max_buy_price).contains(&entry_price) {
            *final_skip = "preco fora da faixa".into();
            continue;
        }

        let final_underlying = candles.last().map(|c| c.close).unwrap_or(candle.close);
        let winner = market.winner_from_outcome_prices().or_else(|| {
            if final_underlying > market.strike_price {
                Some(TradeSide::Up)
            } else if final_underlying < market.strike_price {
                Some(TradeSide::Down)
            } else {
                None
            }
        });

        let stake = stake_override
            .or(decision.stake_usdc)
            .or(cfg.flat_stake_usdc)
            .unwrap_or(1.0);
        let shares = stake / entry_price;
        let payout = match winner {
            Some(winner) if winner == side => shares,
            Some(_) => 0.0,
            None => shares * 0.5,
        };
        let pnl = payout - stake;
        let won = winner == Some(side);
        let diag = decision.diagnostics.as_ref();

        return Some(BacktestTrade {
            market_slug: market.slug.clone(),
            asset: market.config.asset.clone(),
            interval_minutes: market.config.duration_minutes,
            entry_ts: ts,
            side,
            entry_price,
            stake_usdc: stake,
            final_price: final_underlying,
            confidence: diag.and_then(|d| d.confidence).unwrap_or_default(),
            edge: diag.and_then(|d| d.edge).unwrap_or_default(),
            pnl,
            won,
            reason: decision.reason,
        });
    }

    None
}

async fn ensure_strike(mut market: StrategyMarket, binance: &BinanceFeed) -> StrategyMarket {
    if market.strike_price > 0.0 {
        return market;
    }
    if let Ok(Some(price)) = binance
        .price_at(&market.config.binance_symbol, market.start_time.timestamp())
        .await
    {
        market.strike_price = price;
    }
    market
}

fn synthetic_book(token_id: &str, price: f64) -> OrderBookPrice {
    OrderBookPrice {
        token_id: token_id.into(),
        best_ask: price,
        best_bid: price,
        ask_size: 10_000.0,
        bid_size: 10_000.0,
        tick_size: "0.01".into(),
        neg_risk: false,
    }
}

fn nearest_price(points: &[PricePoint], ts: i64) -> Option<f64> {
    points
        .iter()
        .filter(|point| point.t <= ts && ts - point.t <= 90)
        .max_by_key(|point| point.t)
        .map(|point| point.p)
}

fn add_skip(result: &mut BacktestResult, reason: impl Into<String>) {
    *result.skip_reasons.entry(reason.into()).or_default() += 1;
}
