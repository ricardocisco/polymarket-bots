use anyhow::{Context, Result};
use polymarket_sniper_95c::backtest::run_backtest;
use polymarket_sniper_95c::config::Config;
use polymarket_sniper_95c::feed::{BinanceFeed, ClobOrderbookFeed, GammaMarketsFeed};
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_target(false)
        .pretty()
        .init();

    let cfg = Config::from_env()?;
    let days = get_arg::<i64>("--days").unwrap_or(3);
    let asset = get_arg::<String>("--asset");
    let interval = get_arg::<u32>("--interval");
    let show_trades = std::env::args().any(|arg| arg == "--trades");

    let gamma = GammaMarketsFeed::new(&cfg.gamma_base_url)?;
    let clob = ClobOrderbookFeed::new(&cfg.clob_base_url)?;
    let binance = BinanceFeed::new(&cfg.binance_base_url)?;

    info!(
        "backtest sniper_95c | days={} | asset={} | interval={} | band {:.3}-{:.3}",
        days,
        asset.as_deref().unwrap_or("ALL"),
        interval
            .map(|v| format!("{v}m"))
            .unwrap_or_else(|| "ALL".into()),
        cfg.min_entry_price,
        cfg.max_entry_price
    );

    let result = run_backtest(
        &cfg,
        days,
        asset.as_deref(),
        interval,
        &gamma,
        &clob,
        &binance,
    )
    .await?;

    let win_rate = if result.total_trades > 0 {
        result.winners as f64 / result.total_trades as f64 * 100.0
    } else {
        0.0
    };
    let roi = if result.stake_total > 0.0 {
        result.pnl / result.stake_total * 100.0
    } else {
        0.0
    };

    println!();
    println!("{}", "=".repeat(86));
    println!("Backtest Polymarket Sniper 95c");
    println!("{}", "=".repeat(86));
    println!("Candidates : {}", result.total_candidates);
    println!("Trades     : {}", result.total_trades);
    println!("Wins       : {}", result.winners);
    println!("Win rate   : {:.2}%", win_rate);
    println!("Stake      : ${:.2}", result.stake_total);
    println!("PnL        : {:+.2}", result.pnl);
    println!("ROI        : {:+.2}%", roi);
    println!("{}", "-".repeat(86));
    println!("Skip reasons:");
    for (reason, count) in &result.skip_reasons {
        println!("  {:>5}  {}", count, reason);
    }

    if show_trades {
        println!("{}", "-".repeat(86));
        println!(
            "{:<9} {:<5} {:<4} {:>7} {:>9} {:>8}  {}",
            "Asset", "Int", "Side", "Entry", "PnL", "Result", "Slug"
        );
        for trade in &result.trades {
            println!(
                "{:<9} {:<5} {:<4} {:>7.3} {:>+9.2} {:>8}  {}",
                trade.asset,
                format!("{}m", trade.interval_minutes),
                trade.side,
                trade.entry_price,
                trade.pnl,
                if trade.won { "WIN" } else { "LOSS" },
                trade.market_slug
            );
        }
    }
    println!("{}", "=".repeat(86));

    Ok(())
}

fn get_arg<T>(flag: &str) -> Option<T>
where
    T: std::str::FromStr,
{
    let args = std::env::args().collect::<Vec<_>>();
    args.windows(2)
        .find(|window| window[0] == flag)
        .and_then(|window| window[1].parse::<T>().ok())
}

#[allow(dead_code)]
fn require_arg<T>(flag: &str) -> Result<T>
where
    T: std::str::FromStr,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    get_arg(flag).with_context(|| format!("{flag} ausente"))
}
