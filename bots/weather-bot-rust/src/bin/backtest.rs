// src/bin/backtest.rs  (binário: backtest)
//! Análise histórica de resolução do Polymarket Weather Bot.
//!
//! Como funciona:
//!   1. Para cada cidade, busca eventos FECHADOS (já resolvidos) na Gamma API
//!   2. Extrai ICAO da descrição → resolve coordenadas da estação
//!   3. Identifica o mercado vencedor (YES price ≈ 1.0 após resolução)
//!   4. Busca a temperatura REAL daquele dia via Open-Meteo Archive
//!   5. Simula a decisão do bot com a temperatura REAL já conhecida
//!   6. Calcula P&L usando o preço do mercado na abertura
//!   7. Gera relatório por cidade e total
//!
//! IMPORTANTE:
//!   Este binário não é um backtest preditivo. Ele usa hindsight/perfect foresight
//!   para medir a qualidade estrutural dos ranges e do pricing histórico.
//!
//! Open-Meteo Archive API:
//!   https://archive-api.open-meteo.com/v1/archive
//!   Dados reais de temperatura registrada na estação — mesma fonte que o
//!   Wunderground usa para resolver os mercados Polymarket.
//!
//! Uso:
//!   cargo run --bin backtest
//!   cargo run --bin backtest -- --days 30
//!   cargo run --bin backtest -- --days 60 --min-confidence 0.95
//!   cargo run --bin backtest -- --days 14 --min-confidence 0.90 --max-position 5.0

#[path = "../cities.rs"]
mod cities;
#[path = "../config.rs"]
mod config;
#[path = "../markets.rs"]
mod markets;
#[path = "../strategy.rs"]
mod strategy;
#[path = "../types.rs"]
mod types;
#[path = "../weather.rs"]
mod weather;

use anyhow::{Context, Result};
use chrono::{Duration, Local, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, EnvFilter};

use cities::all_slugs;
use config::Config;
use markets::{
    extract_icao, extract_wunderground_history_url, parse_temp_range, TempMarket, TempUnit,
};
use strategy::{evaluate, Decision};
use types::Forecast;
use weather::WeatherClient;

// ── Estruturas internas do backtest ──────────────────────────

/// Um mercado resolvido com o vencedor identificado
#[derive(Debug)]
struct ResolvedMarket {
    market: TempMarket,
    target_date: NaiveDate,
    resolution_source: String,
    /// question do mercado que resolveu YES (preço → 1.0)
    winning_question: String,
}

/// Resultado de uma simulação para um mercado num dia
#[derive(Debug)]
struct BtRow {
    city: String,
    date: NaiveDate,
    icao: String,
    question: String,
    actual_temp: f64,
    unit: TempUnit,
    winner: String,
    decision: &'static str,
    size: Decimal,
    price: f64,
    correct: bool,
    pnl: Decimal,
}

// ── Gamma API (para buscar eventos fechados) ──────────────────

#[derive(Deserialize)]
struct GammaEvent {
    slug: String,
    description: Option<String>,
    markets: Option<Vec<GammaMkt>>,
}

fn deser_str_or_vec<'de, D>(d: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let raw: Option<serde_json::Value> = Option::deserialize(d)?;
    match raw {
        None => Ok(None),
        Some(serde_json::Value::Array(arr)) => arr
            .into_iter()
            .map(|v| match v {
                serde_json::Value::String(s) => Ok(s),
                other => Ok(other.to_string()),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(serde_json::Value::String(s)) => serde_json::from_str::<Vec<String>>(&s)
            .map(Some)
            .map_err(D::Error::custom),
        Some(other) => Err(D::Error::custom(format!(
            "expected array or string, got {}",
            other
        ))),
    }
}

#[derive(Deserialize)]
struct GammaMkt {
    question: String,
    active: Option<bool>,
    #[serde(
        default,
        rename = "clobTokenIds",
        deserialize_with = "deser_str_or_vec"
    )]
    clob_token_ids: Option<Vec<String>>,
    #[serde(
        default,
        rename = "outcomePrices",
        deserialize_with = "deser_str_or_vec"
    )]
    outcome_prices: Option<Vec<String>>,
    #[serde(rename = "negRisk")]
    neg_risk: Option<bool>,
}

// ── Geocoding (usa tabela estática ICAO → coords) ────────────

/// Retorna coordenadas do aeroporto ICAO consultando a tabela estática.
/// Substitui a chamada ao Open-Meteo Geocoding que buscava por nome de cidade
/// (não por código ICAO), retornando coordenadas completamente erradas.
fn geocode(icao: &str) -> Option<(f64, f64)> {
    markets::icao_static_coords(icao)
}

// ── Parser de data do slug ────────────────────────────────────

fn parse_date(slug: &str) -> Option<NaiveDate> {
    let lower = slug.to_lowercase();
    let pos = lower.rfind("-on-")?;
    let parts: Vec<&str> = lower[pos + 4..].splitn(3, '-').collect();
    if parts.len() < 3 {
        return None;
    }
    let month = match parts[0] {
        "january" | "jan" => 1u32,
        "february" | "feb" => 2,
        "march" | "mar" => 3,
        "april" | "apr" => 4,
        "may" => 5,
        "june" | "jun" => 6,
        "july" | "jul" => 7,
        "august" | "aug" => 8,
        "september" | "sep" => 9,
        "october" | "oct" => 10,
        "november" | "nov" => 11,
        "december" | "dec" => 12,
        _ => return None,
    };
    let day: u32 = parts[1].parse().ok()?;
    let year: i32 = parts[2].parse().ok()?;
    NaiveDate::from_ymd_opt(year, month, day)
}

fn detect_unit(desc: &str) -> TempUnit {
    if desc.to_lowercase().contains("degrees fahrenheit") {
        TempUnit::Fahrenheit
    } else {
        TempUnit::Celsius
    }
}

// ── Busca mercados resolvidos ─────────────────────────────────

async fn fetch_resolved(
    http: &reqwest::Client,
    slug_keyword: &str,
    days_back: u32,
) -> Result<Vec<ResolvedMarket>> {
    let url = format!(
        "https://gamma-api.polymarket.com/events?slug={}&closed=true&limit={}",
        slug_keyword,
        days_back + 15
    );

    let events: Vec<GammaEvent> = http
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("Falha ao parsear eventos resolvidos")?;

    let cutoff = Local::now().date_naive() - Duration::days(days_back as i64);
    let today = Local::now().date_naive();
    let mut out = Vec::new();

    for ev in &events {
        if !ev.slug.contains(slug_keyword) {
            continue;
        }

        let date = match parse_date(&ev.slug) {
            Some(d) => d,
            None => {
                warn!("Data não parseável: {}", ev.slug);
                continue;
            }
        };

        // Só mercados passados (não hoje nem futuro) dentro da janela
        if date < cutoff || date >= today {
            continue;
        }

        let desc = ev.description.as_deref().unwrap_or("");
        let icao = match extract_icao(desc) {
            Some(c) => c,
            None => {
                warn!("[{}] ICAO não encontrado", ev.slug);
                continue;
            }
        };
        let resolution_source = match extract_wunderground_history_url(desc) {
            Some(url) => url,
            None => {
                warn!("[{}] URL Wunderground nao encontrada", ev.slug);
                continue;
            }
        };
        let unit = detect_unit(desc);

        let (lat, lon) = match geocode(&icao) {
            Some(c) => c,
            None => {
                warn!(
                    "[ICAO={}] Coordenadas não resolvidas (ICAO não está na tabela estática)",
                    icao
                );
                continue;
            }
        };

        let markets = match &ev.markets {
            Some(m) if !m.is_empty() => m,
            _ => continue,
        };

        // Identifica o vencedor: YES price ≈ 1.0 após resolução
        let winner = markets
            .iter()
            .find(|m| {
                m.outcome_prices
                    .as_deref()
                    .and_then(|p| p.first())
                    .and_then(|p| p.parse::<f64>().ok())
                    .map(|p| p > 0.98)
                    .unwrap_or(false)
            })
            .map(|m| m.question.clone());

        let winning_question = match winner {
            Some(q) => q,
            None => {
                warn!(
                    "[{}] Sem vencedor claro (mercado pode não ter resolvido ainda)",
                    ev.slug
                );
                continue;
            }
        };

        for mkt in markets {
            if mkt.active == Some(false) {
                continue;
            }

            let tokens = match &mkt.clob_token_ids {
                Some(t) if t.len() >= 2 => t,
                _ => continue,
            };

            let yes_p = mkt
                .outcome_prices
                .as_deref()
                .and_then(|p| p.first())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.5);
            let no_p = mkt
                .outcome_prices
                .as_deref()
                .and_then(|p| p.get(1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.5);
            let (rmin, rmax) = parse_temp_range(&mkt.question);

            out.push(ResolvedMarket {
                market: TempMarket {
                    yes_token_id: tokens[0].clone(),
                    no_token_id: tokens[1].clone(),
                    question: mkt.question.clone(),
                    event_slug: ev.slug.clone(),
                    yes_price: yes_p,
                    no_price: no_p,
                    range_min: rmin,
                    range_max: rmax,
                    tick_size: "0.01".into(),
                    neg_risk: mkt.neg_risk.unwrap_or(false),
                    icao: icao.clone(),
                    station_lat: lat,
                    station_lon: lon,
                    unit,
                    target_date: Some(date),
                },
                target_date: date,
                resolution_source: resolution_source.clone(),
                winning_question: winning_question.clone(),
            });
        }
    }

    Ok(out)
}

// ── Helpers ───────────────────────────────────────────────────

fn d2f(d: Decimal) -> f64 {
    d.to_string().parse().unwrap_or(0.0)
}
fn get_arg<T: std::str::FromStr>(args: &[String], flag: &str) -> Option<T> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .and_then(|w| w[1].parse().ok())
}

// ── Main ──────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::new("backtest=info,info"))
        .with_target(false)
        .pretty()
        .init();

    dotenvy::dotenv().ok();

    let args: Vec<String> = std::env::args().collect();
    let days: u32 = get_arg(&args, "--days").unwrap_or(30);
    let min_conf: Option<f64> = get_arg(&args, "--min-confidence");
    let max_pos: Option<f64> = get_arg(&args, "--max-position");
    let watch: bool = args.iter().any(|a| a == "--watch");
    // --interval em segundos. Padrão: 86400 (24h) em --watch, irrelevante sem --watch
    let interval_secs: u64 = get_arg(&args, "--interval").unwrap_or(86400);

    info!("╔══════════════════════════════════════════════════════════╗");
    info!("║  Polymarket Weather Bot — Historical Resolution Review   ║");
    info!("╚══════════════════════════════════════════════════════════╝");
    info!("  Janela:           últimos {} dias", days);
    let cfg = match Config::from_env_without_private_key() {
        Ok(cfg) => cfg.with_runtime_overrides(
            min_conf,
            max_pos.and_then(|v| Decimal::try_from(v).ok()),
            None,
        ),
        Err(_) => Config {
            private_key: String::new(),
            min_confidence: min_conf.unwrap_or(0.98),
            max_position_size_usdc: max_pos
                .and_then(|v| Decimal::try_from(v).ok())
                .unwrap_or(dec!(10)),
            min_order_size_usdc: dec!(1),
            run_interval_secs: 3600,
            dry_run: true,
            allow_live_trading: false,
            penny_shares: 0,
            penny_max_price: 0.02,
            penny_min_confidence: 0.40,
            extended_horizon_days: 3,
            change_poll_secs: 300,
            temp_change_threshold: 0.3,
            price_change_threshold: 0.03,
            use_intraday_trend: true,
            anticipatory_min_confidence: 0.88,
            discovery_refresh_secs: 300,
            resolution_poll_secs: 180,
            weather_poll_d3_secs: 3600,
            weather_poll_d2_secs: 1800,
            weather_poll_d1_secs: 900,
            weather_intraday_poll_secs: 180,
            edge_min: 0.02,
            max_spread_cents: 5.0,
            forecast_change_trigger_degrees: 0.4,
            implied_move_trigger_cents: 2.0,
        },
    };

    info!("  Confiança mínima: {:.0}%", cfg.min_confidence * 100.0);
    info!("  Posição máxima:   {:.1} USDC", cfg.max_position_size_usdc);
    info!(
        "  Modo:             {}",
        if watch {
            format!(
                "--watch (re-executa a cada {}s / {:.1}h)",
                interval_secs,
                interval_secs as f64 / 3600.0
            )
        } else {
            "uma execução".into()
        }
    );
    info!("  Fonte de dados:   Wunderground (temperatura oficial de resolucao)");
    info!("  Modo:             hindsight / perfect-foresight\n");

    if watch {
        info!("👁️  Modo watch ativo — Ctrl+C para parar\n");
    }

    let http = reqwest::Client::builder()
        .user_agent("polymarket-weather-bot-backtest/1.0")
        .timeout(std::time::Duration::from_secs(20))
        .build()?;

    let weather = WeatherClient::new()?;

    // ── Loop principal ────────────────────────────────────────
    let mut ciclo: u32 = 1;
    loop {
        if watch {
            info!("{}", "═".repeat(60));
            info!(
                "🔄 Ciclo #{} — {}",
                ciclo,
                Local::now().format("%Y-%m-%d %H:%M:%S")
            );
            info!("{}\n", "═".repeat(60));
        }

        run_once(&http, &weather, &cfg, days).await;

        if !watch {
            break;
        }

        ciclo += 1;
        info!(
            "\n⏳ Próxima execução em {}s ({:.1}h) — {}\n",
            interval_secs,
            interval_secs as f64 / 3600.0,
            (Local::now() + chrono::Duration::seconds(interval_secs as i64))
                .format("%Y-%m-%d %H:%M:%S")
        );
        tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
    }

    Ok(())
}

/// Executa um ciclo completo de backtest e imprime o relatório.
async fn run_once(http: &reqwest::Client, weather: &WeatherClient, cfg: &Config, days: u32) {
    let slugs = all_slugs();
    let min_conf = cfg.min_confidence;

    let mut rows: Vec<BtRow> = Vec::new();
    let mut stats: HashMap<String, [Decimal; 4]> = HashMap::new();

    for slug in &slugs {
        info!("📍 {} — buscando histórico...", slug.name);

        let resolved = match fetch_resolved(http, slug.keyword, days).await {
            Ok(r) if r.is_empty() => {
                warn!("  Nenhum mercado resolvido encontrado\n");
                continue;
            }
            Ok(r) => r,
            Err(e) => {
                error!("  Erro: {}\n", e);
                continue;
            }
        };

        info!("  {} registro(s) de mercado encontrado(s)", resolved.len());

        let mut cache: HashMap<(NaiveDate, String, String), Option<f64>> = HashMap::new();

        for rm in &resolved {
            let key = (
                rm.target_date,
                rm.market.icao.clone(),
                rm.market.unit.open_meteo_str().to_string(),
            );

            if !cache.contains_key(&key) {
                let temp = weather
                    .fetch_wunderground_historical(
                        &rm.resolution_source,
                        rm.target_date,
                        rm.market.unit,
                        &rm.market.icao,
                    )
                    .await
                    .ok()
                    .flatten()
                    .map(|f| f.max_temp);
                cache.insert(key.clone(), temp);
            }

            let actual_temp = match cache[&key] {
                Some(t) => t,
                None => continue,
            };

            let forecast = Forecast {
                icao: rm.market.icao.clone(),
                max_temp: actual_temp,
                unit: rm.market.unit,
                confidence: min_conf + 0.005,
            };

            let decision = evaluate(&rm.market, &forecast, cfg);

            let (label, size, price): (&'static str, Decimal, f64) = match &decision {
                Decision::BuyYes {
                    size_usdc, price, ..
                } => ("BUY_YES", *size_usdc, *price),
                Decision::BuyNo {
                    size_usdc, price, ..
                } => ("BUY_NO", *size_usdc, *price),
                Decision::Skip(_) => ("SKIP", dec!(0), 0.0),
            };

            if label == "SKIP" {
                rows.push(BtRow {
                    city: slug.name.into(),
                    date: rm.target_date,
                    icao: rm.market.icao.clone(),
                    question: rm.market.question.clone(),
                    actual_temp,
                    unit: rm.market.unit,
                    winner: rm.winning_question.clone(),
                    decision: "SKIP",
                    size: dec!(0),
                    price: 0.0,
                    correct: false,
                    pnl: dec!(0),
                });
                continue;
            }

            let correct = match &decision {
                Decision::BuyYes { .. } => rm.winning_question == rm.market.question,
                Decision::BuyNo { .. } => rm.winning_question != rm.market.question,
                Decision::Skip(_) => false,
            };

            let pnl = if correct {
                Decimal::try_from(d2f(size) * (1.0 / price - 1.0)).unwrap_or(dec!(0))
            } else {
                -size
            };

            info!(
                "  [{} ICAO={}] {} | Temp={:.1}{} | '{}' | {} | PnL={:+.2}",
                rm.target_date,
                rm.market.icao,
                label,
                actual_temp,
                rm.market.unit.symbol(),
                rm.market
                    .question
                    .trim()
                    .chars()
                    .take(35)
                    .collect::<String>(),
                if correct { "✅" } else { "❌" },
                pnl
            );

            let s = stats.entry(slug.name.to_string()).or_insert([dec!(0); 4]);
            s[0] += dec!(1);
            if correct {
                s[1] += dec!(1);
            }
            s[2] += size;
            s[3] += pnl;

            rows.push(BtRow {
                city: slug.name.into(),
                date: rm.target_date,
                icao: rm.market.icao.clone(),
                question: rm.market.question.clone(),
                actual_temp,
                unit: rm.market.unit,
                winner: rm.winning_question.clone(),
                decision: label,
                size,
                price,
                correct,
                pnl,
            });
        }
        info!("");
    }

    print_report(&rows, &stats);
}

fn print_report(rows: &[BtRow], stats: &HashMap<String, [Decimal; 4]>) {
    let sep = "═".repeat(80);
    let mid = "─".repeat(80);

    println!("\n{sep}");
    println!("  📊  RELATÓRIO HISTÓRICO DE RESOLUÇÃO — Polymarket Weather Bot");
    println!("{sep}");
    println!(
        "{:<22} {:>7} {:>9} {:>12} {:>12} {:>9}",
        "Cidade", "Trades", "Acurácia", "Apostado $", "P&L $", "ROI"
    );
    println!("{mid}");

    let mut g = [dec!(0); 4]; // [trades, correct, bet, pnl]

    let mut cities: Vec<&String> = stats.keys().collect();
    cities.sort();

    for city in &cities {
        let s = &stats[*city];
        let n = d2f(s[0]) as u32;
        let ok = d2f(s[1]) as u32;
        if n == 0 {
            continue;
        }
        let acc = ok as f64 / n as f64 * 100.0;
        let roi = if s[2] > dec!(0) {
            d2f(s[3] / s[2]) * 100.0
        } else {
            0.0
        };
        println!(
            "{:<22} {:>7} {:>8.1}% {:>11.2} {:>+11.2} {:>+8.1}%",
            city, n, acc, s[2], s[3], roi
        );
        for i in 0..4 {
            g[i] += s[i];
        }
    }

    println!("{mid}");
    let gn = d2f(g[0]) as u32;
    let gok = d2f(g[1]) as u32;
    let gacc = if gn > 0 {
        gok as f64 / gn as f64 * 100.0
    } else {
        0.0
    };
    let groi = if g[2] > dec!(0) {
        d2f(g[3] / g[2]) * 100.0
    } else {
        0.0
    };
    println!(
        "{:<22} {:>7} {:>8.1}% {:>11.2} {:>+11.2} {:>+8.1}%",
        "TOTAL", gn, gacc, g[2], g[3], groi
    );
    println!("{sep}");

    // Top 5 melhores e piores
    let mut traded: Vec<&BtRow> = rows.iter().filter(|r| r.decision != "SKIP").collect();
    if traded.is_empty() {
        return;
    }
    traded.sort_by(|a, b| {
        b.pnl
            .partial_cmp(&a.pnl)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    println!("\n  🏆 Top 5 Melhores Operações:");
    for r in traded.iter().take(5) {
        println!(
            "    {} | {} | {} | Temp={:.1}{} | PnL={:+.2}",
            r.date,
            r.city,
            r.decision,
            r.actual_temp,
            r.unit.symbol(),
            r.pnl
        );
        println!(
            "       '{}'\n       Vencedor: '{}'",
            r.question.trim().chars().take(60).collect::<String>(),
            r.winner.trim().chars().take(60).collect::<String>()
        );
    }

    println!("\n  💸 Top 5 Piores Operações:");
    for r in traded.iter().rev().take(5) {
        println!(
            "    {} | {} | {} | Temp={:.1}{} | PnL={:+.2}",
            r.date,
            r.city,
            r.decision,
            r.actual_temp,
            r.unit.symbol(),
            r.pnl
        );
        println!(
            "       '{}'\n       Vencedor: '{}'",
            r.question.trim().chars().take(60).collect::<String>(),
            r.winner.trim().chars().take(60).collect::<String>()
        );
    }

    println!("\n  ℹ️  Metodologia:");
    println!("     • Temperatura REAL do dia via Wunderground (fonte oficial da Polymarket)");
    println!("     • Preços usados = preços históricos retornados pela Gamma API");
    println!("     • P&L = ganho líquido (sem taxas) | Quarter-Kelly para sizing");
    println!("     • Este relatório usa hindsight: a decisão vê a temperatura resolvida");
    println!("     • Não interprete este binário como backtest preditivo do bot real");
    println!("{sep}\n");
}
