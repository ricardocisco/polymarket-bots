use anyhow::{Context, Result};
use chrono::{Duration, Local, NaiveDate};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use tracing::{info, warn};

use crate::markets::{self, TempMarket, TempUnit};

#[derive(Debug, Clone)]
pub struct DiscoveredMarket {
    pub market: TempMarket,
    pub city: String,
    pub resolution_source: String,
    pub market_key: String,
}

#[derive(Deserialize)]
struct GammaEvent {
    slug: String,
    description: Option<String>,
    markets: Option<Vec<GammaMkt>>,
}

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

pub async fn fetch_temperature_markets(
    http: &reqwest::Client,
    horizon_days: i64,
) -> Result<Vec<DiscoveredMarket>> {
    let tag = fetch_tag_by_slug(http, "temperature").await?;
    let events = fetch_events_by_tag_id(http, &tag.id).await?;
    if events.is_empty() {
        info!(
            "[discovery] tag '{}' (id={}) sem eventos ativos no Gamma",
            tag.slug, tag.id
        );
    }

    let today = Local::now().date_naive();
    let horizon = today + Duration::days(horizon_days);
    let mut result = Vec::new();
    let mut geo_cache: HashMap<String, Option<(f64, f64)>> = HashMap::new();

    for event in &events {
        let Some(target_date) = parse_date_from_slug(&event.slug) else {
            continue;
        };
        if target_date < today || target_date > horizon {
            continue;
        }

        let desc = event.description.as_deref().unwrap_or("");
        let Some((icao, resolution_source)) = markets::extract_icao_and_source(desc) else {
            warn!("[{}] sem ICAO/fonte de resolucao", event.slug);
            continue;
        };

        let unit = detect_unit_local(desc, event.markets.as_deref().unwrap_or(&[]));
        let city = city_from_slug(&event.slug);

        if !geo_cache.contains_key(&icao) {
            geo_cache.insert(icao.clone(), markets::icao_static_coords(&icao));
        }
        let (lat, lon) = match geo_cache[&icao] {
            Some(coords) => coords,
            None => {
                warn!("[{}] coordenadas ausentes para ICAO={}", event.slug, icao);
                continue;
            }
        };

        let Some(mkts) = &event.markets else {
            continue;
        };

        for mkt in mkts {
            if mkt.active == Some(false) || mkt.closed == Some(true) {
                continue;
            }
            let Some(tokens) = &mkt.clob_token_ids else {
                continue;
            };
            if tokens.len() < 2 {
                continue;
            }
            let yes_price = price_at(&mkt.outcome_prices, 0);
            let no_price = price_at(&mkt.outcome_prices, 1);
            let (range_min, range_max) = markets::parse_temp_range(&mkt.question);
            let market = TempMarket {
                yes_token_id: tokens[0].clone(),
                no_token_id: tokens[1].clone(),
                question: mkt.question.clone(),
                event_slug: event.slug.clone(),
                yes_price,
                no_price,
                range_min,
                range_max,
                tick_size: "0.01".into(),
                neg_risk: mkt.neg_risk.unwrap_or(false),
                icao: icao.clone(),
                station_lat: lat,
                station_lon: lon,
                unit,
                target_date: Some(target_date),
            };
            result.push(DiscoveredMarket {
                city: city.clone(),
                resolution_source: resolution_source.clone(),
                market_key: format!("{}:{}", event.slug, market.yes_token_id),
                market,
            });
        }
    }

    Ok(result)
}

async fn fetch_tag_by_slug(http: &reqwest::Client, slug: &str) -> Result<GammaTag> {
    let url = format!("https://gamma-api.polymarket.com/tags/slug/{slug}");
    http.get(url)
        .send()
        .await
        .context("falha consultando tag por slug no Gamma")?
        .error_for_status()
        .context("Gamma retornou erro ao buscar tag por slug")?
        .json::<GammaTag>()
        .await
        .context("falha ao parsear tag por slug do Gamma")
}

async fn fetch_events_by_tag_id(http: &reqwest::Client, tag_id: &str) -> Result<Vec<GammaEvent>> {
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
            .context("falha na Gamma API durante discovery")?
            .error_for_status()
            .context("Gamma API status na discovery")?
            .text()
            .await
            .context("falha ao ler resposta de discovery")?;

        if let Ok(parsed) = serde_json::from_str::<KeysetEventsResponse>(&body) {
            let count = parsed.events.len();
            all.extend(parsed.events);
            after_cursor = parsed.next_cursor.filter(|cursor| !cursor.is_empty());
            if count == 0 || after_cursor.is_none() {
                break;
            }
            continue;
        }

        if let Ok(parsed) = serde_json::from_str::<Vec<GammaEvent>>(&body) {
            all.extend(parsed);
            break;
        }

        let raw: Value =
            serde_json::from_str(&body).context("falha ao parsear discovery de mercados")?;
        if let Some(events) = raw.get("events").and_then(|v| v.as_array()) {
            let parsed: Vec<GammaEvent> = serde_json::from_value(Value::Array(events.clone()))
                .context("falha ao desserializar eventos do keyset")?;
            let count = parsed.len();
            all.extend(parsed);
            after_cursor = raw
                .get("next_cursor")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned)
                .filter(|cursor| !cursor.is_empty());
            if count == 0 || after_cursor.is_none() {
                break;
            }
            continue;
        }

        return Err(anyhow::anyhow!(
            "resposta inesperada do Gamma em discovery por tag_id"
        ));
    }

    Ok(all)
}

pub async fn winner_of_closed_market(
    http: &reqwest::Client,
    event_slug: &str,
) -> Result<Option<String>> {
    let url = format!(
        "https://gamma-api.polymarket.com/events?slug={}&closed=true&limit=1",
        event_slug
    );

    #[derive(Deserialize)]
    struct Ev {
        markets: Option<Vec<GammaMkt>>,
    }

    let events: Vec<Ev> = http
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let mkts = match events.into_iter().next().and_then(|e| e.markets) {
        Some(m) => m,
        None => return Ok(None),
    };

    Ok(mkts
        .iter()
        .find(|m| price_at(&m.outcome_prices, 0) > 0.98)
        .map(|m| m.question.clone()))
}

pub fn parse_date_from_slug(slug: &str) -> Option<NaiveDate> {
    let lower = slug.to_lowercase();
    let pos = lower.rfind("-on-")?;
    let parts: Vec<&str> = lower[pos + 4..].splitn(3, '-').collect();
    if parts.len() < 3 {
        return None;
    }
    let month = match parts[0] {
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

pub fn city_from_slug(slug: &str) -> String {
    let lower = slug.to_lowercase();
    const PREFIX: &str = "highest-temperature-in-";
    let start = match lower.find(PREFIX) {
        Some(p) => p + PREFIX.len(),
        None => return slug.to_string(),
    };
    let end = match lower[start..].rfind("-on-") {
        Some(p) => start + p,
        None => lower.len(),
    };
    slug[start..end]
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn detect_unit_local(desc: &str, mkts: &[GammaMkt]) -> TempUnit {
    let lower = desc.to_lowercase();
    if lower.contains("degrees celsius") {
        return TempUnit::Celsius;
    }
    if lower.contains("degrees fahrenheit") {
        return TempUnit::Fahrenheit;
    }
    for mkt in mkts {
        if mkt.question.contains("°C") || mkt.question.contains("Â°C") {
            return TempUnit::Celsius;
        }
        if mkt.question.contains("°F") || mkt.question.contains("Â°F") {
            return TempUnit::Fahrenheit;
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
