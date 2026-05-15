use anyhow::Result;
use polymarket_weather::config::Config;
use polymarket_weather::engine::streaming::{print_sim_report, run_sim_monitor};
use rust_decimal::Decimal;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    fmt()
        .with_env_filter(EnvFilter::new("info,warn"))
        .with_target(false)
        .init();

    dotenvy::dotenv().ok();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--report") {
        return print_sim_report().await;
    }

    let interval_secs: Option<u64> = get_arg(&args, "--interval");
    let min_conf = get_arg(&args, "--min-confidence");
    let max_pos: Option<f64> = get_arg(&args, "--max-position");

    let mut cfg = Config::from_env_without_private_key()?.with_runtime_overrides(
        min_conf,
        max_pos.and_then(|v| Decimal::try_from(v).ok()),
        interval_secs,
    );

    if let Some(interval_secs) = interval_secs {
        cfg.discovery_refresh_secs = interval_secs;
        cfg.resolution_poll_secs = interval_secs.min(300);
        cfg.weather_poll_d3_secs = interval_secs.max(cfg.weather_poll_d3_secs.min(interval_secs));
        cfg.weather_poll_d2_secs = interval_secs.min(cfg.weather_poll_d2_secs);
        cfg.weather_poll_d1_secs = interval_secs.min(cfg.weather_poll_d1_secs);
        cfg.weather_intraday_poll_secs = (interval_secs / 2).max(60);
    }

    run_sim_monitor(cfg).await
}

fn get_arg<T: std::str::FromStr>(args: &[String], flag: &str) -> Option<T> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .and_then(|w| w[1].parse().ok())
}
