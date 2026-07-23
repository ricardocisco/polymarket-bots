use anyhow::{Context, Result};
use reqwest::Url;
use tokio::time::sleep;

use crate::types::{deserialize_string_or_number, PriceHistoryResponse, PricePoint, QuoteSnapshot};
use serde::Deserialize;

pub struct ClobOrderbookFeed {
    http: reqwest::Client,
    base_url: String,
}

impl ClobOrderbookFeed {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("btc-6am-bot-rust/0.1")
            .timeout(std::time::Duration::from_secs(45))
            .build()
            .context("falha ao criar cliente HTTP do CLOB")?;
        Ok(Self {
            http,
            base_url: base_url.into(),
        })
    }

    pub async fn quote_for_token(&self, token_id: &str) -> Result<QuoteSnapshot> {
        let mut url = Url::parse(&format!("{}/book", self.base_url.trim_end_matches('/')))?;
        url.query_pairs_mut().append_pair("token_id", token_id);
        let book: OrderBookResponse = self
            .http
            .get(url)
            .send()
            .await
            .context("falha ao consultar /book")?
            .error_for_status()
            .context("erro HTTP em /book")?
            .json()
            .await
            .context("falha ao parsear /book")?;
        let ask = book.asks.first().context("order book sem asks")?;
        Ok(QuoteSnapshot {
            best_bid: book.bids.first().map(|level| level.price),
            best_ask: Some(ask.price),
            last_price: Some(ask.price),
            ask_size: Some(ask.size),
        })
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

        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 1..=3 {
            let response = match self.http.get(url.clone()).send().await {
                Ok(response) => response,
                Err(err) => {
                    last_err = Some(
                        anyhow::Error::new(err).context("falha ao consultar historico de precos"),
                    );
                    if attempt < 3 {
                        sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
                        continue;
                    }
                    break;
                }
            };

            let response = match response.error_for_status() {
                Ok(response) => response,
                Err(err) => {
                    last_err =
                        Some(anyhow::Error::new(err).context("erro HTTP em /prices-history"));
                    if attempt < 3 {
                        sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
                        continue;
                    }
                    break;
                }
            };

            let resp: PriceHistoryResponse = match response.json().await {
                Ok(payload) => payload,
                Err(err) => {
                    last_err =
                        Some(anyhow::Error::new(err).context("falha ao parsear /prices-history"));
                    if attempt < 3 {
                        sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
                        continue;
                    }
                    break;
                }
            };

            return Ok(resp.history);
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("falha desconhecida em /prices-history")))
    }
}

#[derive(Debug, Deserialize)]
struct OrderBookResponse {
    #[serde(default)]
    bids: Vec<BookLevel>,
    #[serde(default)]
    asks: Vec<BookLevel>,
}

#[derive(Debug, Deserialize)]
struct BookLevel {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    price: f64,
    #[serde(deserialize_with = "deserialize_string_or_number")]
    size: f64,
}
