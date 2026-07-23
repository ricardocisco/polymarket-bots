use anyhow::{Context, Result};
use btc_6am_bot::backtest::run_backtest_with_markets;
use btc_6am_bot::config::Config;
use btc_6am_bot::feed::{ClobOrderbookFeed, GammaMarketsFeed};
use chrono::{Duration, Utc};
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

struct HourSummary {
    hour: u32,
    total_candidates: u64,
    total_trades: u64,
    raw_win_rate: f64,
    traded_win_rate: f64,
    pnl: f64,
    roi: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_target(false)
        .pretty()
        .init();

    let cfg = Config::from_env_without_private_key()?;
    let days = get_arg::<i64>("--days").unwrap_or(30);
    let gamma = GammaMarketsFeed::new(&cfg.gamma_base_url)?;
    let clob = ClobOrderbookFeed::new(&cfg.clob_base_url)?;

    info!(
        "BTC 5m | scan automatico de 00:00 a 23:00 UTC | lado={} | hit-rate esperado={:.1}% | entrada<= {:.3} | edge min={:.1}pp | stake={} USDC",
        cfg.trade_direction,
        cfg.expected_win_rate * 100.0,
        cfg.max_entry_price,
        cfg.min_edge * 100.0,
        cfg.position_size_usdc
    );
    info!("janela historica = ultimos {days} dias");

    let start_min = Utc::now() - Duration::days(days);
    let end_max = Utc::now();
    let mut markets = gamma
        .fetch_markets_between(start_min, end_max, true)
        .await
        .context("falha ao buscar mercados fechados")?;

    if !std::env::args().any(|arg| arg == "--scan-hours") {
        let result = run_backtest_with_markets(&cfg, &markets, &clob).await?;
        let roi = if result.stake_total > 0.0 {
            result.pnl / result.stake_total * 100.0
        } else {
            0.0
        };
        println!(
            "Hora {:02}:00 UTC | candidatos={} trades={} wins={} pnl={:+.2} roi={:+.2}%",
            cfg.target_hour_utc,
            result.total_candidates,
            result.total_trades,
            result.winners,
            result.pnl,
            roi
        );
        println!("Use --scan-hours para selecao walk-forward de horario.");
        return Ok(());
    }

    markets.sort_by_key(|market| market.start_date);
    if markets.len() < 2 {
        anyhow::bail!("mercados insuficientes para divisao treino/holdout");
    }
    let split_at = (markets.len() * 70 / 100).clamp(1, markets.len().saturating_sub(1));
    let (training_markets, holdout_markets) = markets.split_at(split_at);

    let mut summaries = Vec::with_capacity(24);
    for hour in 0..24 {
        let local_cfg = cfg.clone().with_target_hour(hour);
        let result = run_backtest_with_markets(&local_cfg, training_markets, &clob).await?;
        let raw_win_rate = if result.total_candidates > 0 {
            result.winners as f64 / result.total_candidates as f64 * 100.0
        } else {
            0.0
        };
        let traded_win_rate = if result.total_trades > 0 {
            result.winners as f64 / result.total_trades as f64 * 100.0
        } else {
            0.0
        };
        let roi = if result.stake_total > 0.0 {
            result.pnl / result.stake_total * 100.0
        } else {
            0.0
        };

        summaries.push(HourSummary {
            hour,
            total_candidates: result.total_candidates,
            total_trades: result.total_trades,
            raw_win_rate,
            traded_win_rate,
            pnl: result.pnl,
            roi,
        });
    }

    let best = summaries
        .iter()
        .max_by(|left, right| left.roi.total_cmp(&right.roi))
        .context("nenhum resultado de hora produzido")?;
    let selected_cfg = cfg.clone().with_target_hour(best.hour);
    let holdout = run_backtest_with_markets(&selected_cfg, holdout_markets, &clob).await?;
    let holdout_roi = if holdout.stake_total > 0.0 {
        holdout.pnl / holdout.stake_total * 100.0
    } else {
        0.0
    };

    println!();
    println!("{}", "=".repeat(88));
    println!("Backtest BTC 5m por hora UTC (70% treino / 30% holdout)");
    println!("{}", "=".repeat(88));
    println!(
        "{:<8} {:>12} {:>10} {:>11} {:>11} {:>10} {:>9}",
        "Hora", "Candidatos", "Trades", "WR bruto", "WR exec", "P&L", "ROI"
    );
    println!("{}", "-".repeat(88));
    for summary in &summaries {
        println!(
            "{:02}:00    {:>12} {:>10} {:>10.2}% {:>10.2}% {:>+10.2} {:>+8.2}%",
            summary.hour,
            summary.total_candidates,
            summary.total_trades,
            summary.raw_win_rate,
            summary.traded_win_rate,
            summary.pnl,
            summary.roi
        );
    }
    println!("{}", "-".repeat(88));
    println!(
        "Melhor hora por ROI: {:02}:00 UTC | trades={} | pnl={:+.2} | roi={:+.2}%",
        best.hour, best.total_trades, best.pnl, best.roi
    );
    println!(
        "Holdout da hora selecionada: trades={} | wins={} | pnl={:+.2} | roi={:+.2}%",
        holdout.total_trades, holdout.winners, holdout.pnl, holdout_roi
    );
    println!(
        "TARGET_HOUR_UTC atual no config: {:02}:00 UTC",
        cfg.target_hour_utc
    );
    println!("{}", "=".repeat(88));

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
