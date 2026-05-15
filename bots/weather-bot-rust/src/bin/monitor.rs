// src/bin/monitor.rs  (binário: monitor)
//! Painel de monitoramento em tempo real dos mercados de temperatura.
//!
//! Mostra a cada ciclo, para TODOS os mercados abertos (hoje + amanhã + futuros):
//!   - Data alvo do mercado
//!   - Cidade e código ICAO da estação
//!   - Previsão atual de temperatura máxima (Open-Meteo)
//!   - Range do mercado e onde a previsão está em relação a ele
//!   - Confiança do modelo meteorológico
//!   - Preços atuais YES/NO
//!   - Decisão que o bot tomaria AGORA
//!   - Barra visual mostrando a previsão dentro/fora do range
//!
//! OBJETIVO: rodar de hora em hora para acompanhar como a previsão
//! evolui ao longo do dia e identificar o melhor momento de entrada.
//!
//! USO:
//!   cargo run --release --bin monitor                    # roda uma vez
//!   cargo run --release --bin monitor -- --watch         # loop a cada 1h
//!   cargo run --release --bin monitor -- --watch --interval 1800  # a cada 30min
//!   cargo run --release --bin monitor -- --min-confidence 0.95    # threshold custom

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
use serde::Deserialize;
use std::collections::HashMap;
use tracing::warn;
use tracing_subscriber::{fmt, EnvFilter};

use config::Config;
use markets::TempUnit;
use strategy::{evaluate, Decision};
use types::Forecast;
use weather::WeatherClient;

// ── Gamma API structs ─────────────────────────────────────────

#[derive(Deserialize)]
struct GammaTag {
    id: String,
    slug: String,
}

#[derive(Deserialize)]
struct KeysetEventsResponse {
    #[serde(default)]
    events: Vec<GammaEvent>,
    #[serde(default)]
    next_cursor: Option<String>,
}

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
    closed: Option<bool>,
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

// ── Dados de um mercado enriquecido com previsão ──────────────

#[derive(Debug)]
struct MarketSnapshot {
    // Identificação
    city: String,
    target_date: NaiveDate,
    icao: String,
    question: String,
    // Range do mercado
    range_min: Option<f64>,
    range_max: Option<f64>,
    unit: TempUnit,
    // Previsão atual
    forecast: Option<Forecast>,
    // Preços atuais
    yes_price: f64,
    no_price: f64,
    // Decisão do bot
    decision: String,
    decision_size: f64,
    decision_price: f64,
}

// ── Busca todos os mercados ativos ────────────────────────────

async fn fetch_tag_id(http: &reqwest::Client, slug: &str) -> Result<String> {
    let url = format!("https://gamma-api.polymarket.com/tags/slug/{slug}");
    let tag: GammaTag = http
        .get(&url)
        .send()
        .await
        .context("Falha buscando tag no Gamma")?
        .error_for_status()?
        .json()
        .await
        .context("Falha parsear tag do Gamma")?;
    Ok(tag.id)
}

async fn fetch_events_by_tag(http: &reqwest::Client, tag_id: &str) -> Result<Vec<GammaEvent>> {
    let mut all = Vec::new();
    let mut after_cursor: Option<String> = None;

    loop {
        let url = if let Some(cursor) = &after_cursor {
            format!(
                "https://gamma-api.polymarket.com/events/keyset?tag_id={tag_id}&active=true&closed=false&limit=200&after_cursor={cursor}"
            )
        } else {
            format!(
                "https://gamma-api.polymarket.com/events/keyset?tag_id={tag_id}&active=true&closed=false&limit=200"
            )
        };

        let body = http
            .get(&url)
            .send()
            .await
            .context("Falha na Gamma API keyset")?
            .error_for_status()
            .context("Gamma API erro na keyset")?
            .text()
            .await
            .context("Falha lendo resposta keyset")?;

        if let Ok(parsed) = serde_json::from_str::<KeysetEventsResponse>(&body) {
            let count = parsed.events.len();
            all.extend(parsed.events);
            after_cursor = parsed.next_cursor.filter(|c| !c.is_empty());
            if count == 0 || after_cursor.is_none() {
                break;
            }
        } else if let Ok(parsed) = serde_json::from_str::<Vec<GammaEvent>>(&body) {
            all.extend(parsed);
            break;
        } else {
            break;
        }
    }
    Ok(all)
}

async fn fetch_all_markets(
    http: &reqwest::Client,
    horizon_days: i64,
) -> Result<Vec<markets::TempMarket>> {
    let tag_id = fetch_tag_id(http, "temperature").await?;
    let events = fetch_events_by_tag(http, &tag_id).await?;

    let today = Local::now().date_naive();
    let horizon = today + Duration::days(horizon_days);

    let mut result = Vec::new();
    let mut geo_cache: HashMap<String, Option<(f64, f64)>> = HashMap::new();

    for event in &events {
        let target_date = match parse_date_from_slug(&event.slug) {
            Some(d) => d,
            None => continue,
        };
        if target_date < today || target_date > horizon {
            continue;
        }

        let desc = event.description.as_deref().unwrap_or("");
        let icao = match markets::extract_icao(desc) {
            Some(c) => c,
            None => {
                warn!("[{}] ICAO não encontrado", event.slug);
                continue;
            }
        };

        let unit = detect_unit(desc, event.markets.as_deref().unwrap_or(&[]));

        if !geo_cache.contains_key(&icao) {
            geo_cache.insert(icao.clone(), geocode_icao(http, &icao));
        }
        let (lat, lon) = match geo_cache[&icao] {
            Some(c) => c,
            None => continue,
        };

        let mkt_list = match &event.markets {
            Some(m) if !m.is_empty() => m,
            _ => continue,
        };

        for mkt in mkt_list {
            if mkt.active == Some(false) || mkt.closed == Some(true) {
                continue;
            }
            let tokens = match &mkt.clob_token_ids {
                Some(t) if t.len() >= 2 => t,
                _ => continue,
            };
            let yes_p = price_at(&mkt.outcome_prices, 0);
            let no_p = price_at(&mkt.outcome_prices, 1);
            let (rmin, rmax) = markets::parse_temp_range(&mkt.question);

            result.push(markets::TempMarket {
                yes_token_id: tokens[0].clone(),
                no_token_id: tokens[1].clone(),
                question: mkt.question.clone(),
                event_slug: event.slug.clone(),
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
                target_date: parse_date_from_slug(&event.slug),
            });
        }
    }

    Ok(result)
}

// ── Painel principal ──────────────────────────────────────────

async fn run_monitor(http: &reqwest::Client, weather: &WeatherClient, cfg: &Config) -> Result<()> {
    let now = Local::now();
    let today = now.date_naive();

    // Header
    println!();
    println!("{}", "═".repeat(90));
    println!(
        "  🌡️  POLYMARKET WEATHER MONITOR  |  {}",
        now.format("%Y-%m-%d  %H:%M:%S")
    );
    println!(
        "  Threshold: {:.0}% confiança  |  Posição max: {:.0} USDC  |  Horizonte: hoje + 3 dias",
        cfg.min_confidence * 100.0,
        cfg.max_position_size_usdc
    );
    println!("{}", "═".repeat(90));

    // Busca mercados
    print!("  Buscando mercados...");
    let all_markets = fetch_all_markets(http, 3).await?;
    println!(" {} encontrados\n", all_markets.len());

    // Cache de previsão por ICAO
    let mut fcache: HashMap<String, Option<Forecast>> = HashMap::new();

    // Constrói snapshots
    let mut snapshots: Vec<MarketSnapshot> = Vec::new();

    for mkt in &all_markets {
        if !fcache.contains_key(&mkt.icao) {
            let f = weather.fetch_for_market(mkt).await.ok().flatten();
            fcache.insert(mkt.icao.clone(), f);
        }

        let forecast = fcache[&mkt.icao].clone();

        // Decisão do bot
        let (decision_label, decision_size, decision_price) = match &forecast {
            Some(f) => match evaluate(mkt, f, cfg) {
                Decision::BuyYes {
                    size_usdc, price, ..
                } => ("BUY YES".into(), d2f(size_usdc), price),
                Decision::BuyNo {
                    size_usdc, price, ..
                } => ("BUY NO".into(), d2f(size_usdc), price),
                Decision::Skip(r) => (format!("SKIP ({})", truncate(&r, 30)), 0.0, 0.0),
            },
            None => ("SEM PREVISÃO".into(), 0.0, 0.0),
        };

        let target_date = parse_date_from_slug(&mkt.event_slug).unwrap_or(today);
        let city = city_from_slug(&mkt.event_slug);

        snapshots.push(MarketSnapshot {
            city,
            target_date,
            icao: mkt.icao.clone(),
            question: mkt.question.clone(),
            range_min: mkt.range_min,
            range_max: mkt.range_max,
            unit: mkt.unit,
            forecast,
            yes_price: mkt.yes_price,
            no_price: mkt.no_price,
            decision: decision_label,
            decision_size,
            decision_price,
        });
    }

    // Ordena: hoje primeiro, depois amanhã, etc.; dentro do dia por cidade
    snapshots.sort_by(|a, b| a.target_date.cmp(&b.target_date).then(a.city.cmp(&b.city)));

    // Agrupa por data para imprimir seções separadas
    let mut current_date: Option<NaiveDate> = None;
    let mut current_city: Option<String> = None;

    for snap in &snapshots {
        // Cabeçalho de data
        if current_date != Some(snap.target_date) {
            current_date = Some(snap.target_date);
            current_city = None;

            let days_away = (snap.target_date - today).num_days();
            let label = match days_away {
                0 => "HOJE".to_string(),
                1 => "AMANHÃ".to_string(),
                n => format!("DAQUI A {} DIAS", n),
            };
            println!();
            println!(
                "  ┌─ {} — {} ─────────────────────────────────────────────",
                label,
                snap.target_date.format("%d/%m/%Y")
            );
        }

        // Cabeçalho de cidade (agrupa múltiplos ranges da mesma cidade)
        if current_city.as_deref() != Some(&snap.city) {
            current_city = Some(snap.city.clone());

            let (temp_str, conf_str, bar_str) = match &snap.forecast {
                Some(f) => {
                    let temp = format!("{:.1}{}", f.max_temp, snap.unit.symbol());
                    let conf = format!("{:.1}%", f.confidence * 100.0);
                    let bar = temp_bar(f.max_temp, snap.range_min, snap.range_max, snap.unit);
                    (temp, conf, bar)
                }
                None => ("N/A".into(), "N/A".into(), "  [sem dados]".into()),
            };

            println!("  │");
            println!("  │  📍 {}  [ICAO: {}]", snap.city, snap.icao);
            println!(
                "  │     Previsão agora: {}  |  Confiança: {}",
                temp_str, conf_str
            );
            println!("  │     {}", bar_str);
        }

        // Linha do mercado
        let range_str = format_range(snap.range_min, snap.range_max, snap.unit);
        let decision_display =
            format_decision(&snap.decision, snap.decision_size, snap.decision_price);
        let price_str = format!(
            "YES {:.0}%  NO {:.0}%",
            snap.yes_price * 100.0,
            snap.no_price * 100.0
        );

        println!(
            "  │     {:>10}  {:>18}  {:>18}  {}",
            range_str,
            price_str,
            decision_display,
            truncate(&snap.question, 40)
        );
    }

    println!("  │");
    println!("  └{}", "─".repeat(88));

    // Resumo de decisões
    println!();
    println!("  📊 RESUMO DE DECISÕES");
    println!("  {}", "─".repeat(88));

    let buy_yes: Vec<&MarketSnapshot> = snapshots
        .iter()
        .filter(|s| s.decision.starts_with("BUY YES"))
        .collect();
    let buy_no: Vec<&MarketSnapshot> = snapshots
        .iter()
        .filter(|s| s.decision.starts_with("BUY NO"))
        .collect();
    let skips = snapshots.len() - buy_yes.len() - buy_no.len();

    if buy_yes.is_empty() && buy_no.is_empty() {
        println!(
            "  Nenhuma entrada recomendada neste ciclo (threshold {:.0}%)",
            cfg.min_confidence * 100.0
        );
    }

    for snap in &buy_yes {
        println!(
            "  ✅ BUY YES  {:.0} USDC @ {:.2}  |  {:10}  {}  |  {}  [ICAO:{}] {}",
            snap.decision_size,
            snap.decision_price,
            snap.target_date.format("%d/%m/%Y"),
            snap.city,
            format_range(snap.range_min, snap.range_max, snap.unit),
            snap.icao,
            snap.forecast
                .as_ref()
                .map(|f| format!("Prev={:.1}{}", f.max_temp, snap.unit.symbol()))
                .unwrap_or_default()
        );
    }

    for snap in &buy_no {
        println!(
            "  ❌ BUY NO   {:.0} USDC @ {:.2}  |  {:10}  {}  |  {}  [ICAO:{}] {}",
            snap.decision_size,
            snap.decision_price,
            snap.target_date.format("%d/%m/%Y"),
            snap.city,
            format_range(snap.range_min, snap.range_max, snap.unit),
            snap.icao,
            snap.forecast
                .as_ref()
                .map(|f| format!("Prev={:.1}{}", f.max_temp, snap.unit.symbol()))
                .unwrap_or_default()
        );
    }

    println!(
        "\n  Total: {} BUY YES  |  {} BUY NO  |  {} SKIP",
        buy_yes.len(),
        buy_no.len(),
        skips
    );

    println!("{}", "═".repeat(90));
    println!();

    Ok(())
}

// ── Main ──────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Silencia tracing para não poluir o painel visual
    fmt()
        .with_env_filter(EnvFilter::new("warn"))
        .with_target(false)
        .init();

    dotenvy::dotenv().ok();

    let args: Vec<String> = std::env::args().collect();
    let watch = args.iter().any(|a| a == "--watch");
    let interval_secs: u64 = get_arg(&args, "--interval").unwrap_or(3600);
    let min_conf = get_arg(&args, "--min-confidence");
    let max_pos: Option<f64> = get_arg(&args, "--max-position");

    let cfg = Config::from_env_without_private_key()?.with_runtime_overrides(
        min_conf,
        max_pos.and_then(|v| Decimal::try_from(v).ok()),
        Some(interval_secs),
    );

    let http = reqwest::Client::builder()
        .user_agent("polymarket-weather-bot-monitor/1.0")
        .timeout(std::time::Duration::from_secs(20))
        .build()?;

    let weather = WeatherClient::new()?;

    if watch {
        println!(
            "Monitor ativo — atualizando a cada {}min | Ctrl+C para parar",
            interval_secs / 60
        );
    }

    let mut cycle = 1u32;
    loop {
        if watch && cycle > 1 {
            println!("  [ciclo #{}]", cycle);
        }

        if let Err(e) = run_monitor(&http, &weather, &cfg).await {
            eprintln!("Erro no ciclo {}: {}", cycle, e);
        }

        if !watch {
            break;
        }

        cycle += 1;
        let next = Local::now() + Duration::seconds(interval_secs as i64);
        println!(
            "  ⏳ Próxima atualização: {}  (em {}min)\n",
            next.format("%H:%M:%S"),
            interval_secs / 60
        );
        tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
    }

    Ok(())
}

// ── Formatação visual ─────────────────────────────────────────

/// Barra visual mostrando onde a temperatura prevista cai em relação ao range.
///
/// Exemplo:  |-----|[===PREV===]-----|
///           min        ^          max
fn temp_bar(temp: f64, range_min: Option<f64>, range_max: Option<f64>, unit: TempUnit) -> String {
    let sym = unit.symbol();

    let (rmin, rmax) = match (range_min, range_max) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (a, temp.max(a + 4.0)),
        (None, Some(b)) => (temp.min(b - 4.0), b),
        (None, None) => return format!("  Prev: {:.1}{}", temp, sym),
    };

    let width = 40usize;
    let span = (rmax - rmin).max(0.01);
    let padded_min = rmin - span * 0.3;
    let padded_max = rmax + span * 0.3;
    let total = padded_max - padded_min;

    let range_start = (((rmin - padded_min) / total) * width as f64) as usize;
    let range_end = (((rmax - padded_min) / total) * width as f64) as usize;
    let temp_pos =
        (((temp - padded_min) / total) * width as f64).clamp(0.0, width as f64 - 1.0) as usize;

    let mut bar: Vec<char> = vec!['-'; width];

    // Marca o range com '='
    for i in range_start..=range_end.min(width - 1) {
        bar[i] = '=';
    }

    // Marca a temperatura com '*' (ou 'X' se fora do range)
    let marker = if temp >= rmin && temp <= rmax {
        '●'
    } else {
        '✗'
    };
    if temp_pos < width {
        bar[temp_pos] = marker;
    }

    let status = if temp >= rmin && temp <= rmax {
        format!(
            "DENTRO  +{:.1}{} margem",
            (temp - rmin).min(rmax - temp),
            sym
        )
    } else if temp < rmin {
        format!("ABAIXO  {:.1}{} do range", rmin - temp, sym)
    } else {
        format!("ACIMA   {:.1}{} do range", temp - rmax, sym)
    };

    format!(
        "  {:.1}{}  [{}]  {:.1}{}   {}  {}",
        padded_min,
        sym,
        bar.into_iter().collect::<String>(),
        padded_max,
        sym,
        format!("Prev={:.1}{}", temp, sym),
        status
    )
}

fn format_range(min: Option<f64>, max: Option<f64>, unit: TempUnit) -> String {
    let s = unit.symbol();
    match (min, max) {
        (Some(a), Some(b)) if (b - a - 1.0).abs() < 0.1 => format!("{:.0}-{:.0}{}", a, b, s), // "60-61°F"
        (Some(a), Some(b)) if (b - a - 1.0).abs() >= 0.1 => format!("{:.0}–{:.0}{}", a, b, s),
        (Some(a), None) => format!(">={:.0}{}", a, s),
        (None, Some(b)) => format!("<={:.0}{}", b, s),
        (Some(a), Some(b)) => format!("{:.0}-{:.0}{}", a, b, s),
        _ => "???".into(),
    }
}

fn format_decision(label: &str, size: f64, price: f64) -> String {
    if size > 0.0 {
        format!("{} {:.1}U@{:.2}", label, size, price)
    } else {
        label.chars().take(18).collect()
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

// ── Helpers comuns ────────────────────────────────────────────

fn d2f(d: Decimal) -> f64 {
    d.to_string().parse().unwrap_or(0.0)
}

fn get_arg<T: std::str::FromStr>(args: &[String], flag: &str) -> Option<T> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .and_then(|w| w[1].parse().ok())
}

fn parse_date_from_slug(slug: &str) -> Option<NaiveDate> {
    let lower = slug.to_lowercase();
    let pos = lower.rfind("-on-")?;
    let parts: Vec<&str> = lower[pos + 4..].splitn(3, '-').collect();
    if parts.len() < 3 {
        return None;
    }
    let month: u32 = match parts[0] {
        "january" | "jan" => 1,
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
    NaiveDate::from_ymd_opt(parts[2].parse().ok()?, month, parts[1].parse().ok()?)
}

fn city_from_slug(slug: &str) -> String {
    let lower = slug.to_lowercase();
    let prefix = "highest-temperature-in-";
    let start = lower.find(prefix).map(|p| p + prefix.len()).unwrap_or(0);
    let after = &lower[start..];
    let end = after.rfind("-on-").unwrap_or(after.len());
    after[..end]
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().to_string() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn detect_unit(desc: &str, mkts: &[GammaMkt]) -> TempUnit {
    let lo = desc.to_lowercase();
    if lo.contains("degrees fahrenheit") {
        return TempUnit::Fahrenheit;
    }
    if lo.contains("degrees celsius") {
        return TempUnit::Celsius;
    }
    for m in mkts {
        if m.question.contains("°F") {
            return TempUnit::Fahrenheit;
        }
        if m.question.contains("°C") {
            return TempUnit::Celsius;
        }
    }
    TempUnit::Celsius
}

fn price_at(prices: &Option<Vec<String>>, idx: usize) -> f64 {
    prices
        .as_deref()
        .and_then(|p| p.get(idx))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.5)
}

fn geocode_icao(_http: &reqwest::Client, icao: &str) -> Option<(f64, f64)> {
    markets::icao_static_coords(icao)
}
