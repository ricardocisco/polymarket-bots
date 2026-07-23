use anyhow::{Context, Result};
use polymarket_weather::config::Config;
use polymarket_weather::engine::streaming::run_bot;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,warn")),
        )
        .with_target(false)
        .init();

    let cfg = Config::from_env_without_private_key()
        .context("falha ao carregar configuracao do weather bot")?;
    if cfg.live_trading_enabled() && cfg.private_key.trim().is_empty() {
        anyhow::bail!("POLYMARKET_PRIVATE_KEY nao definida para live trading");
    }
    run_bot(cfg).await
}
