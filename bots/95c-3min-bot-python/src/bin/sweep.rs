use anyhow::Result;
use polymarket_sniper_95c::backtest::run_backtest;
use polymarket_sniper_95c::config::Config;
use polymarket_sniper_95c::feed::{BinanceFeed, ClobOrderbookFeed, GammaMarketsFeed};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::new("warn"))
        .with_target(false)
        .pretty()
        .init();

    let base_cfg = Config::from_env()?;
    let days = get_arg::<i64>("--days").unwrap_or(3);
    let asset = get_arg::<String>("--asset");
    let interval = get_arg::<u32>("--interval");
    let gamma = GammaMarketsFeed::new(&base_cfg.gamma_base_url)?;
    let clob = ClobOrderbookFeed::new(&base_cfg.clob_base_url)?;
    let binance = BinanceFeed::new(&base_cfg.binance_base_url)?;

    println!("{}", "=".repeat(78));
    println!("Sweep sniper_95c | days={days}");
    println!("{}", "=".repeat(78));
    println!(
        "{:<12} {:>10} {:>10} {:>9} {:>10} {:>9}",
        "Band", "Trades", "WR", "PnL", "Stake", "ROI"
    );
    println!("{}", "-".repeat(78));

    for (min_price, max_price) in [
        (0.93, 0.995),
        (0.94, 0.995),
        (0.95, 0.995),
        (0.96, 0.995),
        (0.97, 0.995),
    ] {
        let cfg = base_cfg.clone().with_entry_band(min_price, max_price);
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
        let wr = if result.total_trades > 0 {
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
            "{:.3}-{:.3} {:>10} {:>9.2}% {:>+9.2} {:>10.2} {:>+8.2}%",
            min_price, max_price, result.total_trades, wr, result.pnl, result.stake_total, roi
        );
    }
    println!("{}", "=".repeat(78));
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
