use anyhow::Result;
use btc_6am_bot::backtest::run_backtest;
use btc_6am_bot::config::Config;
use btc_6am_bot::feed::{ClobOrderbookFeed, GammaMarketsFeed};
use btc_6am_bot::types::TradeDirection;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_target(false)
        .pretty()
        .init();

    let cfg = Config::from_env_without_private_key()?;
    let days = get_arg::<i64>("--days").unwrap_or(7);
    let gamma = GammaMarketsFeed::new(&cfg.gamma_base_url)?;
    let clob = ClobOrderbookFeed::new(&cfg.clob_base_url)?;

    println!("{}", "═".repeat(88));
    println!("Sweep BTC {:02}:00 UTC", cfg.target_hour_utc);
    println!("{}", "═".repeat(88));

    for direction in [TradeDirection::Up, TradeDirection::Down] {
        for entry in [0.45, 0.50, 0.55, 0.60] {
            let local_cfg = cfg
                .clone()
                .with_trade_direction(direction)
                .with_max_entry_price(entry);
            let result = run_backtest(&local_cfg, days, &gamma, &clob).await?;
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

            println!(
                "side={} | max_entry={:.2} | trades={} | win_rate={:.2}% | pnl={:+.2} | roi={:+.2}%",
                direction, entry, result.total_trades, win_rate, result.pnl, roi
            );
        }
    }

    println!("{}", "═".repeat(88));
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
