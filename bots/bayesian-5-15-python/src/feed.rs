use anyhow::{Context, Result};
use chrono::{DateTime, Duration, SecondsFormat, TimeZone, Timelike, Utc};
use regex::Regex;
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;
use tokio::time::sleep;

use crate::types::{Candle, MarketConfig, OrderBookPrice, PricePoint, StrategyMarket};

pub struct GammaMarketsFeed {
    http: reqwest::Client,
    base_url: String,
}

impl GammaMarketsFeed {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("polymarket-bayesian-rust/0.1")
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .context("falha ao criar cliente HTTP da Gamma")?;
        Ok(Self {
            http,
            base_url: base_url.into(),
        })
    }

    pub async fn find_active_market(
        &self,
        config: &MarketConfig,
    ) -> Result<Option<StrategyMarket>> {
        for ts in interval_timestamps(config.duration_minutes) {
            let slug = format!("{}-{ts}", config.slug_prefix());
            let mut url = Url::parse(&format!("{}/markets", self.base_url.trim_end_matches('/')))?;
            url.query_pairs_mut().append_pair("slug", &slug);

            let payload: Value = match self.http.get(url).send().await {
                Ok(resp) => match resp.error_for_status() {
                    Ok(resp) => resp
                        .json()
                        .await
                        .context("falha ao parsear Gamma /markets")?,
                    Err(_) => continue,
                },
                Err(_) => continue,
            };

            let market_value = payload
                .as_array()
                .and_then(|items| items.first())
                .cloned()
                .unwrap_or(payload);

            let Some(market) = parse_market(&market_value, config.clone()) else {
                continue;
            };
            if market.closed || !market.active || !market.accepting_orders {
                continue;
            }
            return Ok(Some(market));
        }
        Ok(None)
    }

    pub async fn fetch_closed_markets(
        &self,
        start_min: DateTime<Utc>,
        start_max: DateTime<Utc>,
        configs: &[MarketConfig],
    ) -> Result<Vec<StrategyMarket>> {
        let mut out = Vec::new();
        let mut window_start = normalize_datetime(start_min);
        let start_max = normalize_datetime(start_max);

        while window_start < start_max {
            let window_end = std::cmp::min(window_start + Duration::hours(24), start_max);
            out.extend(
                self.fetch_events_window(window_start, window_end, true, configs)
                    .await?,
            );
            window_start = window_end;
        }

        out.sort_by(|left, right| left.slug.cmp(&right.slug));
        out.dedup_by(|left, right| left.slug == right.slug);
        Ok(out)
    }

    pub async fn fetch_market_by_slug(
        &self,
        slug: &str,
        config: MarketConfig,
    ) -> Result<Option<StrategyMarket>> {
        let mut url = Url::parse(&format!("{}/markets", self.base_url.trim_end_matches('/')))?;
        url.query_pairs_mut().append_pair("slug", slug);
        let payload: Value = self
            .http
            .get(url)
            .send()
            .await
            .context("falha ao consultar market por slug")?
            .error_for_status()
            .context("Gamma retornou erro HTTP")?
            .json()
            .await
            .context("falha ao parsear Gamma /markets")?;

        let market_value = payload
            .as_array()
            .and_then(|items| items.first())
            .cloned()
            .unwrap_or(payload);
        Ok(parse_market(&market_value, config))
    }

    async fn fetch_events_window(
        &self,
        start_min: DateTime<Utc>,
        start_max: DateTime<Utc>,
        closed: bool,
        configs: &[MarketConfig],
    ) -> Result<Vec<StrategyMarket>> {
        let mut out = Vec::new();
        let mut after_cursor: Option<String> = None;
        let limit = 500usize;

        loop {
            let mut url = Url::parse(&format!(
                "{}/events/keyset",
                self.base_url.trim_end_matches('/')
            ))?;
            url.query_pairs_mut()
                .append_pair("closed", if closed { "true" } else { "false" })
                .append_pair("limit", &limit.to_string())
                .append_pair("start_time_min", &format_datetime_for_gamma(start_min))
                .append_pair("start_time_max", &format_datetime_for_gamma(start_max));
            if let Some(cursor) = after_cursor.as_deref() {
                url.query_pairs_mut().append_pair("after_cursor", cursor);
            }

            let payload: Value = self
                .http
                .get(url)
                .send()
                .await
                .context("falha ao consultar events/keyset")?
                .error_for_status()
                .context("Gamma retornou erro HTTP em events/keyset")?
                .json()
                .await
                .context("falha ao parsear events/keyset")?;

            let events = payload
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let page_len = events.len();

            for event in events {
                let Some(markets) = event.get("markets").and_then(Value::as_array) else {
                    continue;
                };
                for value in markets {
                    let slug = value.get("slug").and_then(as_string).unwrap_or_default();
                    let Some(config) = configs
                        .iter()
                        .find(|cfg| slug.starts_with(&cfg.slug_prefix()))
                        .cloned()
                    else {
                        continue;
                    };
                    if let Some(market) = parse_market(value, config) {
                        out.push(market);
                    }
                }
            }

            after_cursor = payload
                .get("next_cursor")
                .and_then(as_string)
                .map(ToOwned::to_owned);
            if page_len < limit || after_cursor.is_none() {
                break;
            }
        }

        Ok(out)
    }
}

pub struct ClobOrderbookFeed {
    http: reqwest::Client,
    base_url: String,
}

impl ClobOrderbookFeed {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("polymarket-bayesian-rust/0.1")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("falha ao criar cliente HTTP do CLOB")?;
        Ok(Self {
            http,
            base_url: base_url.into(),
        })
    }

    pub async fn buy_price(&self, token_id: &str) -> Result<f64> {
        let mut url = Url::parse(&format!("{}/price", self.base_url.trim_end_matches('/')))?;
        url.query_pairs_mut()
            .append_pair("token_id", token_id)
            .append_pair("side", "BUY");
        let resp: PriceResponse = self
            .http
            .get(url)
            .send()
            .await
            .context("falha ao consultar CLOB /price")?
            .error_for_status()
            .context("erro HTTP em CLOB /price")?
            .json()
            .await
            .context("falha ao parsear CLOB /price")?;
        Ok(resp.price)
    }

    pub async fn order_book_price(&self, token_id: &str) -> Result<OrderBookPrice> {
        let mut url = Url::parse(&format!("{}/book", self.base_url.trim_end_matches('/')))?;
        url.query_pairs_mut().append_pair("token_id", token_id);
        let resp: OrderBookResponse = self
            .http
            .get(url)
            .send()
            .await
            .context("falha ao consultar CLOB /book")?
            .error_for_status()
            .context("erro HTTP em CLOB /book")?
            .json()
            .await
            .context("falha ao parsear CLOB /book")?;

        let best_ask = resp.asks.first().context("order book sem asks")?;
        let best_bid = resp.bids.first().unwrap_or(best_ask);
        Ok(OrderBookPrice {
            token_id: token_id.to_owned(),
            best_ask: best_ask.price,
            best_bid: best_bid.price,
            ask_size: best_ask.size,
            bid_size: best_bid.size,
            tick_size: resp.tick_size.unwrap_or_else(|| "0.01".into()),
            neg_risk: resp.neg_risk.unwrap_or(false),
        })
    }

    pub async fn refresh_prices(&self, market: &mut StrategyMarket) -> Result<()> {
        let (up, down) = tokio::try_join!(
            self.buy_price(&market.up_token_id),
            self.buy_price(&market.down_token_id)
        )?;
        market.up_price = up;
        market.down_price = down;
        Ok(())
    }

    pub async fn price_history(
        &self,
        token_id: &str,
        start_ts: i64,
        end_ts: i64,
        fidelity_minutes: u32,
    ) -> Result<Vec<PricePoint>> {
        let mut url = Url::parse(&format!(
            "{}/prices-history",
            self.base_url.trim_end_matches('/')
        ))?;
        url.query_pairs_mut()
            .append_pair("market", token_id)
            .append_pair("startTs", &start_ts.to_string())
            .append_pair("endTs", &end_ts.to_string())
            .append_pair("fidelity", &fidelity_minutes.to_string());

        let mut last_err = None;
        for attempt in 1..=3 {
            let result = self
                .http
                .get(url.clone())
                .send()
                .await
                .context("falha ao consultar prices-history")
                .and_then(|resp| {
                    resp.error_for_status()
                        .context("erro HTTP em prices-history")
                });

            match result {
                Ok(resp) => {
                    let parsed: PriceHistoryResponse = resp
                        .json()
                        .await
                        .context("falha ao parsear prices-history")?;
                    return Ok(parsed
                        .history
                        .into_iter()
                        .map(|point| PricePoint {
                            t: point.t,
                            p: point.p,
                        })
                        .collect());
                }
                Err(err) => {
                    last_err = Some(err);
                    if attempt < 3 {
                        sleep(std::time::Duration::from_millis(400 * attempt)).await;
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("falha desconhecida em prices-history")))
    }
}

pub struct BinanceFeed {
    http: reqwest::Client,
    base_url: String,
}

impl BinanceFeed {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("polymarket-bayesian-rust/0.1")
            .timeout(std::time::Duration::from_secs(12))
            .build()
            .context("falha ao criar cliente HTTP da Binance")?;
        Ok(Self {
            http,
            base_url: base_url.into(),
        })
    }

    pub async fn recent_candles(&self, symbol: &str, limit: u32) -> Result<Vec<Candle>> {
        self.candles_until(symbol, Utc::now().timestamp(), limit)
            .await
    }

    pub async fn candles_until(
        &self,
        symbol: &str,
        end_ts: i64,
        limit: u32,
    ) -> Result<Vec<Candle>> {
        let mut url = Url::parse(&format!(
            "{}/api/v3/klines",
            self.base_url.trim_end_matches('/')
        ))?;
        url.query_pairs_mut()
            .append_pair("symbol", symbol)
            .append_pair("interval", "1m")
            .append_pair("endTime", &(end_ts * 1000).to_string())
            .append_pair("limit", &limit.to_string());
        let mut candles = self.fetch_candles(url).await?;
        candles.retain(|candle| candle.open_time + 60 <= end_ts);
        Ok(candles)
    }

    pub async fn candles_between(
        &self,
        symbol: &str,
        start_ts: i64,
        end_ts: i64,
    ) -> Result<Vec<Candle>> {
        let mut out = Vec::new();
        let mut cursor = start_ts;
        while cursor < end_ts {
            let mut url = Url::parse(&format!(
                "{}/api/v3/klines",
                self.base_url.trim_end_matches('/')
            ))?;
            url.query_pairs_mut()
                .append_pair("symbol", symbol)
                .append_pair("interval", "1m")
                .append_pair("startTime", &(cursor * 1000).to_string())
                .append_pair("endTime", &(end_ts * 1000).to_string())
                .append_pair("limit", "1000");
            let mut chunk = self.fetch_candles(url).await?;
            chunk.retain(|candle| candle.open_time + 60 <= end_ts);
            if chunk.is_empty() {
                break;
            }
            cursor = chunk
                .last()
                .map(|candle| candle.open_time + 60)
                .unwrap_or(cursor + 60);
            let chunk_len = chunk.len();
            out.extend(chunk);
            if chunk_len < 1000 {
                break;
            }
        }
        Ok(out)
    }

    pub async fn price_at(&self, symbol: &str, ts: i64) -> Result<Option<f64>> {
        let mut url = Url::parse(&format!(
            "{}/api/v3/klines",
            self.base_url.trim_end_matches('/')
        ))?;
        url.query_pairs_mut()
            .append_pair("symbol", symbol)
            .append_pair("interval", "1m")
            .append_pair("startTime", &(ts * 1000).to_string())
            .append_pair("limit", "1");
        Ok(self.fetch_candles(url).await?.first().map(|c| c.open))
    }

    async fn fetch_candles(&self, url: Url) -> Result<Vec<Candle>> {
        let raw: Vec<Value> = self
            .http
            .get(url)
            .send()
            .await
            .context("falha ao consultar Binance klines")?
            .error_for_status()
            .context("Binance retornou erro HTTP")?
            .json()
            .await
            .context("falha ao parsear Binance klines")?;

        Ok(raw
            .into_iter()
            .filter_map(|row| {
                let arr = row.as_array()?;
                Some(Candle {
                    open_time: arr.first()?.as_i64()? / 1000,
                    open: parse_value_f64(arr.get(1)?)?,
                    high: parse_value_f64(arr.get(2)?)?,
                    low: parse_value_f64(arr.get(3)?)?,
                    close: parse_value_f64(arr.get(4)?)?,
                    volume: parse_value_f64(arr.get(5)?)?,
                })
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct PriceResponse {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    price: f64,
}

#[derive(Debug, Deserialize)]
struct OrderBookResponse {
    #[serde(default)]
    asks: Vec<OrderBookLevel>,
    #[serde(default)]
    bids: Vec<OrderBookLevel>,
    tick_size: Option<String>,
    neg_risk: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct OrderBookLevel {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    price: f64,
    #[serde(deserialize_with = "deserialize_string_or_number")]
    size: f64,
}

#[derive(Debug, Deserialize)]
struct PriceHistoryResponse {
    #[serde(default)]
    history: Vec<PricePointWire>,
}

#[derive(Debug, Deserialize)]
struct PricePointWire {
    t: i64,
    #[serde(deserialize_with = "deserialize_string_or_number")]
    p: f64,
}

fn parse_market(value: &Value, config: MarketConfig) -> Option<StrategyMarket> {
    let slug = value.get("slug").and_then(as_string)?.to_owned();
    let token_ids = parse_token_ids(value);
    if token_ids.len() < 2 {
        return None;
    }

    let outcomes = value
        .get("outcomes")
        .map(parse_jsonish_vec_string)
        .unwrap_or_else(|| vec!["Up".into(), "Down".into()]);
    let up_idx = outcomes
        .iter()
        .position(|outcome| outcome.to_ascii_lowercase().contains("up"))
        .unwrap_or(0);
    let down_idx = outcomes
        .iter()
        .position(|outcome| outcome.to_ascii_lowercase().contains("down"))
        .unwrap_or(1);

    let start_time = value
        .get("eventStartTime")
        .and_then(as_string)
        .and_then(parse_datetime)
        .or_else(|| {
            value
                .get("startDate")
                .and_then(as_string)
                .and_then(parse_datetime)
        })
        .or_else(|| parse_slug_start(&slug))?;
    let end_time = value
        .get("endDate")
        .and_then(as_string)
        .and_then(parse_datetime)?;
    let description = value
        .get("description")
        .and_then(as_string)
        .map(ToOwned::to_owned);
    let question = value
        .get("question")
        .and_then(as_string)
        .unwrap_or_default()
        .to_owned();
    let strike_price = parse_strike(description.as_deref().unwrap_or(""), &question);
    let outcome_prices = value
        .get("outcomePrices")
        .map(parse_jsonish_vec_f64)
        .unwrap_or_default();

    Some(StrategyMarket {
        id: value
            .get("id")
            .and_then(as_string)
            .or_else(|| value.get("conditionId").and_then(as_string))
            .unwrap_or(&slug)
            .to_owned(),
        slug,
        question,
        description,
        up_token_id: token_ids
            .get(up_idx)
            .cloned()
            .unwrap_or_else(|| token_ids[0].clone()),
        down_token_id: token_ids
            .get(down_idx)
            .cloned()
            .unwrap_or_else(|| token_ids[1].clone()),
        strike_price,
        start_time,
        end_time,
        up_price: outcome_prices.get(up_idx).copied().unwrap_or(0.5),
        down_price: outcome_prices.get(down_idx).copied().unwrap_or(0.5),
        active: value.get("active").and_then(as_bool).unwrap_or(false),
        closed: value.get("closed").and_then(as_bool).unwrap_or(false),
        accepting_orders: value
            .get("acceptingOrders")
            .and_then(as_bool)
            .unwrap_or(true),
        outcome_prices: vec![
            outcome_prices.get(up_idx).copied().unwrap_or(0.5),
            outcome_prices.get(down_idx).copied().unwrap_or(0.5),
        ],
        config,
    })
}

fn interval_timestamps(duration_minutes: u32) -> Vec<i64> {
    let now = Utc::now();
    let minute = (now.minute() / duration_minutes) * duration_minutes;
    let current_start = now
        .with_minute(minute)
        .and_then(|d| d.with_second(0))
        .and_then(|d| d.with_nanosecond(0))
        .unwrap_or(now);
    [-1, 0, 1, 2]
        .into_iter()
        .map(|offset| current_start + Duration::minutes(offset * duration_minutes as i64))
        .map(|dt| dt.timestamp())
        .collect()
}

fn parse_token_ids(value: &Value) -> Vec<String> {
    let from_clob = value
        .get("clobTokenIds")
        .map(parse_jsonish_vec_string)
        .unwrap_or_default();
    if !from_clob.is_empty() {
        return from_clob;
    }

    match value.get("tokens") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(v) => Some(v.clone()),
                Value::Object(_) => item
                    .get("token_id")
                    .and_then(as_string)
                    .or_else(|| item.get("tokenId").and_then(as_string))
                    .map(ToOwned::to_owned),
                _ => None,
            })
            .collect(),
        Some(Value::String(raw)) => serde_json::from_str::<Vec<String>>(raw).unwrap_or_default(),
        _ => vec![],
    }
}

fn parse_strike(description: &str, question: &str) -> f64 {
    let text = format!("{description} {question}");
    let dollar_re = Regex::new(r"\$[\d,]+(?:\.\d+)?").expect("regex valida");
    if let Some(found) = dollar_re.find_iter(&text).last() {
        if let Ok(value) = found.as_str().replace(['$', ','], "").parse::<f64>() {
            return value;
        }
    }
    let clean_re = Regex::new(r"(?i)(?:Strike|Price|Reference|Beat)[:\s]+([\d,]+(?:\.\d+)?)")
        .expect("regex valida");
    clean_re
        .captures_iter(&text)
        .last()
        .and_then(|cap| cap.get(1))
        .and_then(|m| m.as_str().replace(',', "").parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn parse_slug_start(slug: &str) -> Option<DateTime<Utc>> {
    let ts = slug.rsplit('-').next()?.parse::<i64>().ok()?;
    Utc.timestamp_opt(ts, 0).single()
}

fn parse_datetime(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn normalize_datetime(value: DateTime<Utc>) -> DateTime<Utc> {
    value
        .with_nanosecond(0)
        .expect("with_nanosecond(0) deve funcionar")
}

fn format_datetime_for_gamma(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn as_string(value: &Value) -> Option<&str> {
    match value {
        Value::String(v) => Some(v),
        _ => None,
    }
}

fn as_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(v) => Some(*v),
        Value::String(v) => v.parse().ok(),
        _ => None,
    }
}

fn parse_value_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(v) => v.as_f64(),
        Value::String(v) => v.parse().ok(),
        _ => None,
    }
}

fn parse_jsonish_vec_string(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(v) => Some(v.clone()),
                Value::Number(v) => Some(v.to_string()),
                _ => None,
            })
            .collect(),
        Value::String(raw) => serde_json::from_str::<Vec<String>>(raw).unwrap_or_default(),
        _ => vec![],
    }
}

fn parse_jsonish_vec_f64(value: &Value) -> Vec<f64> {
    match value {
        Value::Array(items) => items.iter().filter_map(parse_value_f64).collect(),
        Value::String(raw) => serde_json::from_str::<Vec<f64>>(raw).unwrap_or_else(|_| {
            serde_json::from_str::<Vec<String>>(raw)
                .map(|items| {
                    items
                        .into_iter()
                        .filter_map(|item| item.parse::<f64>().ok())
                        .collect()
                })
                .unwrap_or_default()
        }),
        _ => vec![],
    }
}

pub fn deserialize_string_or_number<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    parse_value_f64(&value)
        .ok_or_else(|| serde::de::Error::custom(format!("valor numerico invalido: {value}")))
}
