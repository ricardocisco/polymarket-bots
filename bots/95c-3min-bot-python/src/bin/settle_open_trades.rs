use anyhow::Result;
use polymarket_sniper_95c::config::Config;
use polymarket_sniper_95c::feed::{BinanceFeed, GammaMarketsFeed};
use polymarket_sniper_95c::settle::reconcile_due_trades;
use polymarket_sniper_95c::storage::PaperTradeStore;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_target(false)
        .pretty()
        .init();

    let cfg = Config::from_env()?;
    let store = PaperTradeStore::connect(&cfg.paper_trades_path).await?;
    let gamma = GammaMarketsFeed::new(&cfg.gamma_base_url)?;
    let binance = BinanceFeed::new(&cfg.binance_base_url)?;
    let settled = reconcile_due_trades(&store, &gamma, &binance).await?;
    println!("settled_open_trades={settled}");
    Ok(())
}
