use anyhow::Result;
use btc_6am_bot::config::Config;
use btc_6am_bot::feed::GammaMarketsFeed;
use btc_6am_bot::storage::{reconcile_due_trades, PaperTradeStore};
use chrono::Utc;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_target(false)
        .pretty()
        .init();

    let cfg = Config::from_env_without_private_key()?;
    let gamma = GammaMarketsFeed::new(&cfg.gamma_base_url)?;
    let store = PaperTradeStore::connect(&cfg.paper_trades_path).await?;

    let settled = reconcile_due_trades(&store, &gamma, Utc::now().timestamp()).await?;
    info!("settle_open_trades liquidou {} trade(s)", settled);

    Ok(())
}
