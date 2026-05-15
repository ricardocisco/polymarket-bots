use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, SecondsFormat, Timelike, Utc};
use reqwest::Url;
use tracing::warn;

use crate::types::StrategyMarket;

pub struct GammaMarketsFeed {
    http: reqwest::Client,
    base_url: String,
}

impl GammaMarketsFeed {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("btc-6am-bot-rust/0.1")
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .context("falha ao criar cliente HTTP da Gamma")?;
        Ok(Self {
            http,
            base_url: base_url.into(),
        })
    }

    pub async fn fetch_markets_between(
        &self,
        start_min: DateTime<Utc>,
        start_max: DateTime<Utc>,
        closed: bool,
    ) -> Result<Vec<StrategyMarket>> {
        let start_min = normalize_datetime(start_min);
        let start_max = normalize_datetime(start_max);

        if closed && start_max - start_min > Duration::hours(24) {
            return self
                .fetch_markets_between_chunked(start_min, start_max, closed, Duration::hours(24))
                .await;
        }

        self.fetch_markets_between_single_range(start_min, start_max, closed)
            .await
    }

    async fn fetch_markets_between_single_range(
        &self,
        start_min: DateTime<Utc>,
        start_max: DateTime<Utc>,
        closed: bool,
    ) -> Result<Vec<StrategyMarket>> {
        match self
            .fetch_markets_between_single_range_impl(start_min, start_max, closed, true)
            .await
        {
            Ok(markets) => Ok(markets),
            Err(err) => {
                warn!(
                    "Gamma falhou com title_search=bitcoin no range {}..{}; tentando sem filtro server-side",
                    format_datetime_for_gamma(start_min),
                    format_datetime_for_gamma(start_max)
                );
                self.fetch_markets_between_single_range_impl(start_min, start_max, closed, false)
                    .await
                    .with_context(|| {
                        format!(
                            "Gamma tambem falhou sem title_search no range {}..{}",
                            format_datetime_for_gamma(start_min),
                            format_datetime_for_gamma(start_max)
                        )
                    })
                    .map_err(|fallback_err| fallback_err.context(err.to_string()))
            }
        }
    }

    async fn fetch_markets_between_single_range_impl(
        &self,
        start_min: DateTime<Utc>,
        start_max: DateTime<Utc>,
        closed: bool,
        use_title_search: bool,
    ) -> Result<Vec<StrategyMarket>> {
        let limit = 500usize;
        let mut out = Vec::new();
        let mut after_cursor: Option<String> = None;

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
            if use_title_search {
                url.query_pairs_mut().append_pair("title_search", "bitcoin");
            }
            if let Some(cursor) = after_cursor.as_deref() {
                url.query_pairs_mut().append_pair("after_cursor", cursor);
            }

            let payload: serde_json::Value = self
                .http
                .get(url)
                .send()
                .await
                .context("falha ao consultar events da Gamma")?
                .error_for_status()
                .context("Gamma retornou erro HTTP")?
                .json()
                .await
                .context("falha ao parsear resposta de events")?;

            let events = payload
                .get("events")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();

            let page_len = events.len();
            for event in events {
                let event_title = event
                    .get("title")
                    .and_then(as_string)
                    .map(ToOwned::to_owned);
                let event_slug = event.get("slug").and_then(as_string).map(ToOwned::to_owned);

                let Some(markets) = event.get("markets").and_then(|value| value.as_array()) else {
                    continue;
                };

                for market in markets {
                    if let Some(parsed) =
                        parse_strategy_market(market, event_title.clone(), event_slug.clone())
                    {
                        out.push(parsed);
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

    async fn fetch_markets_between_chunked(
        &self,
        start_min: DateTime<Utc>,
        start_max: DateTime<Utc>,
        closed: bool,
        chunk_size: Duration,
    ) -> Result<Vec<StrategyMarket>> {
        let mut dedup = BTreeMap::<String, StrategyMarket>::new();
        let mut window_start = start_min;

        while window_start < start_max {
            let window_end = std::cmp::min(window_start + chunk_size, start_max);
            match self
                .fetch_markets_between_single_range(window_start, window_end, closed)
                .await
            {
                Ok(markets) => {
                    for market in markets {
                        dedup.insert(market.id.clone(), market);
                    }
                }
                Err(err) => {
                    warn!(
                        "Gamma falhou no range {}..{}; tentando janelas menores",
                        format_datetime_for_gamma(window_start),
                        format_datetime_for_gamma(window_end)
                    );
                    for market in self
                        .fetch_markets_between_in_small_windows(window_start, window_end, closed)
                        .await
                        .with_context(|| {
                            format!(
                                "falha ao buscar eventos da Gamma no range {}..{}",
                                format_datetime_for_gamma(window_start),
                                format_datetime_for_gamma(window_end)
                            )
                        })?
                    {
                        dedup.insert(market.id.clone(), market);
                    }
                    let _ = err;
                }
            }
            window_start = window_end;
        }

        Ok(dedup.into_values().collect())
    }

    async fn fetch_markets_between_in_small_windows(
        &self,
        start_min: DateTime<Utc>,
        start_max: DateTime<Utc>,
        closed: bool,
    ) -> Result<Vec<StrategyMarket>> {
        let mut dedup = BTreeMap::<String, StrategyMarket>::new();
        let mut pending = vec![(start_min, start_max)];

        while let Some((window_start, window_end)) = pending.pop() {
            match self
                .fetch_markets_between_single_range(window_start, window_end, closed)
                .await
            {
                Ok(markets) => {
                    for market in markets {
                        dedup.insert(market.id.clone(), market);
                    }
                }
                Err(err) => {
                    let span = window_end - window_start;
                    if !closed || span <= Duration::minutes(30) {
                        return Err(err).with_context(|| {
                            format!(
                                "Gamma ainda falhou mesmo apos subdividir o range {}..{}",
                                format_datetime_for_gamma(window_start),
                                format_datetime_for_gamma(window_end)
                            )
                        });
                    }

                    let half_secs = (span.num_seconds() / 2).max(1);
                    let mid = window_start + Duration::seconds(half_secs);
                    warn!(
                        "Gamma falhou no subrange {}..{}; subdividindo",
                        format_datetime_for_gamma(window_start),
                        format_datetime_for_gamma(window_end)
                    );
                    pending.push((mid, window_end));
                    pending.push((window_start, mid));
                }
            }
        }

        Ok(dedup.into_values().collect())
    }

    pub async fn fetch_market_by_id(&self, market_id: &str) -> Result<Option<StrategyMarket>> {
        let url = format!(
            "{}/markets/{}",
            self.base_url.trim_end_matches('/'),
            market_id
        );
        let market: serde_json::Value = self
            .http
            .get(url)
            .send()
            .await
            .context("falha ao consultar market por id")?
            .error_for_status()
            .context("Gamma retornou erro HTTP ao consultar market")?
            .json()
            .await
            .context("falha ao parsear market")?;

        Ok(parse_strategy_market(&market, None, None))
    }
}

fn parse_strategy_market(
    market: &serde_json::Value,
    event_title: Option<String>,
    event_slug: Option<String>,
) -> Option<StrategyMarket> {
    Some(StrategyMarket {
        id: market.get("id").and_then(as_string)?.to_owned(),
        slug: market
            .get("slug")
            .and_then(as_string)
            .map(ToOwned::to_owned),
        event_slug,
        event_title,
        question: market.get("question").and_then(as_string)?.to_owned(),
        description: market
            .get("description")
            .and_then(as_string)
            .map(ToOwned::to_owned),
        group_item_title: market
            .get("groupItemTitle")
            .and_then(as_string)
            .map(ToOwned::to_owned),
        start_date: market
            .get("eventStartTime")
            .and_then(as_string)
            .and_then(parse_datetime)
            .or_else(|| {
                market
                    .get("startDate")
                    .and_then(as_string)
                    .and_then(parse_datetime)
            })?,
        end_date: market
            .get("endDate")
            .and_then(as_string)
            .and_then(parse_datetime)?,
        active: market.get("active").and_then(as_bool).unwrap_or(false),
        closed: market.get("closed").and_then(as_bool).unwrap_or(false),
        accepting_orders: market
            .get("acceptingOrders")
            .and_then(as_bool)
            .unwrap_or(true),
        liquidity_num: market.get("liquidityNum").and_then(as_f64),
        volume_num: market.get("volumeNum").and_then(as_f64),
        best_bid: market.get("bestBid").and_then(as_f64),
        best_ask: market.get("bestAsk").and_then(as_f64),
        clob_token_ids: market
            .get("clobTokenIds")
            .map(parse_jsonish_vec_string)
            .unwrap_or_default(),
        outcomes: market
            .get("outcomes")
            .map(parse_jsonish_vec_string)
            .unwrap_or_default(),
        outcome_prices: market
            .get("outcomePrices")
            .map(parse_jsonish_vec_f64)
            .unwrap_or_default(),
    })
}

fn as_string(value: &serde_json::Value) -> Option<&str> {
    match value {
        serde_json::Value::String(v) => Some(v.as_str()),
        _ => None,
    }
}

fn as_bool(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(v) => Some(*v),
        serde_json::Value::String(v) => v.parse::<bool>().ok(),
        _ => None,
    }
}

fn as_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(v) => v.as_f64(),
        serde_json::Value::String(v) => v.parse::<f64>().ok(),
        _ => None,
    }
}

fn normalize_datetime(value: DateTime<Utc>) -> DateTime<Utc> {
    value
        .with_nanosecond(0)
        .expect("with_nanosecond(0) deve sempre funcionar")
}

fn format_datetime_for_gamma(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_datetime(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|date| date.with_timezone(&Utc))
}

fn parse_jsonish_vec_string(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                serde_json::Value::String(v) => Some(v.clone()),
                serde_json::Value::Number(v) => Some(v.to_string()),
                _ => None,
            })
            .collect(),
        serde_json::Value::String(raw) => {
            serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
        }
        _ => vec![],
    }
}

fn parse_jsonish_vec_f64(value: &serde_json::Value) -> Vec<f64> {
    match value {
        serde_json::Value::Array(items) => items.iter().filter_map(as_f64).collect(),
        serde_json::Value::String(raw) => {
            serde_json::from_str::<Vec<f64>>(raw).unwrap_or_else(|_| {
                serde_json::from_str::<Vec<String>>(raw)
                    .map(|items| {
                        items
                            .into_iter()
                            .filter_map(|item| item.parse::<f64>().ok())
                            .collect()
                    })
                    .unwrap_or_default()
            })
        }
        _ => vec![],
    }
}
