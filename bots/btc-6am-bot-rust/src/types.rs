use chrono::{DateTime, Duration, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeDirection {
    Up,
    Down,
}

impl FromStr for TradeDirection {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            other => Err(anyhow::anyhow!("direção inválida: {other}")),
        }
    }
}

impl Display for TradeDirection {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Up => write!(f, "UP"),
            Self::Down => write!(f, "DOWN"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Hold,
    Buy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderTimeInForce {
    MarketableLimit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteSnapshot {
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub last_price: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct StrategyInput {
    pub market_id: String,
    pub market_ticker: String,
    pub quote: QuoteSnapshot,
    pub now_ts: i64,
    pub close_ts: i64,
    pub seconds_to_close: f64,
    pub is_candidate: bool,
    pub market_closed: bool,
    pub liquidity_num: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyDiagnostics {
    pub execution_mode: String,
    pub size: Option<u32>,
    pub confidence: Option<f64>,
    pub order_type: Option<String>,
    pub edge: Option<f64>,
    pub max_entry_price: Option<f64>,
    pub current_price: Option<f64>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalDecision {
    pub action: Action,
    pub side: Option<TradeDirection>,
    pub size: u32,
    pub reason: String,
    pub order_type: Option<OrderTimeInForce>,
    pub limit_price_cents: Option<f64>,
    pub confidence: Option<f64>,
    pub diagnostics: Option<StrategyDiagnostics>,
    pub stake_usdc: Option<f64>,
}

impl SignalDecision {
    pub fn hold(reason: &'static str) -> Self {
        Self::hold_owned(reason.to_string())
    }

    pub fn hold_owned(reason: String) -> Self {
        Self {
            action: Action::Hold,
            side: None,
            size: 0,
            reason,
            order_type: None,
            limit_price_cents: None,
            confidence: None,
            diagnostics: None,
            stake_usdc: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionUpdate {
    pub market_id: String,
    pub order_id: Option<String>,
    pub simulated: bool,
    pub submitted_price: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct StrategyMarket {
    pub id: String,
    pub slug: Option<String>,
    pub event_slug: Option<String>,
    pub event_title: Option<String>,
    pub question: String,
    pub description: Option<String>,
    pub group_item_title: Option<String>,
    pub start_date: DateTime<Utc>,
    pub end_date: DateTime<Utc>,
    pub active: bool,
    pub closed: bool,
    pub accepting_orders: bool,
    pub liquidity_num: Option<f64>,
    pub volume_num: Option<f64>,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub clob_token_ids: Vec<String>,
    pub outcomes: Vec<String>,
    pub outcome_prices: Vec<f64>,
}

impl StrategyMarket {
    pub fn duration(&self) -> Duration {
        self.end_date - self.start_date
    }

    pub fn is_five_minute_market(&self) -> bool {
        let seconds = self.duration().num_seconds();
        (240..=360).contains(&seconds)
    }

    pub fn is_target_hour(&self, hour_utc: u32) -> bool {
        self.start_date.hour() == hour_utc
    }

    pub fn combined_text(&self) -> String {
        [
            self.event_title.as_deref().unwrap_or(""),
            self.question.as_str(),
            self.description.as_deref().unwrap_or(""),
            self.group_item_title.as_deref().unwrap_or(""),
            self.slug.as_deref().unwrap_or(""),
        ]
        .join(" ")
        .to_ascii_lowercase()
    }

    pub fn has_btc_keywords(&self) -> bool {
        let text = self.combined_text();
        (text.contains("bitcoin") || text.contains("btc"))
            && (text.contains("5-minute")
                || text.contains("5 minute")
                || text.contains("5m")
                || text.contains("5 min"))
    }

    pub fn is_up_down_market(&self) -> bool {
        let lowers = self
            .outcomes
            .iter()
            .map(|it| it.to_ascii_lowercase())
            .collect::<Vec<_>>();
        lowers.iter().any(|it| it.contains("up")) && lowers.iter().any(|it| it.contains("down"))
    }

    pub fn is_strategy_candidate(&self, hour_utc: u32) -> bool {
        (self.active || self.closed)
            && self.is_target_hour(hour_utc)
            && self.is_five_minute_market()
            && self.has_btc_keywords()
            && self.is_up_down_market()
            && self.clob_token_ids.len() >= 2
    }

    pub fn token_id_for_direction(&self, direction: TradeDirection) -> Option<String> {
        let idx = self.outcome_index_for_direction(direction)?;
        self.clob_token_ids.get(idx).cloned()
    }

    pub fn outcome_index_for_direction(&self, direction: TradeDirection) -> Option<usize> {
        self.outcomes.iter().position(|outcome| match direction {
            TradeDirection::Up => outcome.to_ascii_lowercase().contains("up"),
            TradeDirection::Down => outcome.to_ascii_lowercase().contains("down"),
        })
    }

    pub fn resolved_price_for_direction(&self, direction: TradeDirection) -> Option<f64> {
        let idx = self.outcome_index_for_direction(direction)?;
        self.outcome_prices.get(idx).copied()
    }

    pub fn display_label(&self) -> String {
        format!(
            "{} | {} | {}",
            self.start_date.format("%Y-%m-%d %H:%M:%S"),
            self.event_title.as_deref().unwrap_or("-"),
            self.question
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PriceResponse {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub price: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PriceHistoryResponse {
    #[serde(default)]
    pub history: Vec<PricePoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PricePoint {
    pub t: i64,
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub p: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PaperTradeStatus {
    Open,
    Settled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperTradeRecord {
    pub id: String,
    pub strategy_name: String,
    pub strategy_version: String,
    pub market_id: String,
    pub market_ticker: String,
    pub market_question: String,
    pub token_id: String,
    pub close_ts: i64,
    pub submitted_at: i64,
    pub side: TradeDirection,
    pub size: u32,
    pub entry_price_cents: u32,
    pub decision_reason: String,
    pub diagnostics: Option<StrategyDiagnostics>,
    pub status: PaperTradeStatus,
    pub winner_side: Option<TradeDirection>,
    pub final_price: Option<f64>,
    pub pnl_cents: Option<i64>,
    pub settled_at: Option<i64>,
    pub dry_run: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct OpenTradeGroup {
    pub market_id: String,
    pub market_ticker: String,
    pub close_ts: i64,
    pub count: u64,
}

#[derive(Debug, Clone)]
pub struct BacktestTrade {
    pub market_label: String,
    pub entry_price: f64,
    pub final_price: f64,
    pub pnl: f64,
    pub won: bool,
}

#[derive(Debug, Clone)]
pub struct BacktestResult {
    pub total_candidates: u64,
    pub total_trades: u64,
    pub winners: u64,
    pub pnl: f64,
    pub stake_total: f64,
    pub skip_reasons: BTreeMap<String, u64>,
    pub trades: Vec<BacktestTrade>,
}

pub fn deserialize_string_or_number<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(v) => v.parse::<f64>().map_err(serde::de::Error::custom),
        serde_json::Value::Number(v) => v
            .as_f64()
            .ok_or_else(|| serde::de::Error::custom("número inválido")),
        other => Err(serde::de::Error::custom(format!(
            "valor inesperado para número: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{StrategyMarket, TradeDirection};
    use chrono::{TimeZone, Utc};

    fn sample_market() -> StrategyMarket {
        StrategyMarket {
            id: "1".into(),
            slug: Some("bitcoin-up-or-down-in-5-minutes".into()),
            event_slug: Some("btc-5m-test".into()),
            event_title: Some("Bitcoin up or down in 5 minutes?".into()),
            question: "Where will Bitcoin move in 5 minutes?".into(),
            description: None,
            group_item_title: Some("BTC 5m".into()),
            start_date: Utc.with_ymd_and_hms(2026, 4, 27, 6, 0, 0).unwrap(),
            end_date: Utc.with_ymd_and_hms(2026, 4, 27, 6, 5, 0).unwrap(),
            active: true,
            closed: false,
            accepting_orders: true,
            liquidity_num: Some(1200.0),
            volume_num: Some(4000.0),
            best_bid: Some(0.49),
            best_ask: Some(0.51),
            clob_token_ids: vec!["up-token".into(), "down-token".into()],
            outcomes: vec!["Up".into(), "Down".into()],
            outcome_prices: vec![1.0, 0.0],
        }
    }

    #[test]
    fn detects_btc_5m_candidate() {
        assert!(sample_market().is_strategy_candidate(6));
    }

    #[test]
    fn maps_direction_to_token() {
        let market = sample_market();
        assert_eq!(
            market.token_id_for_direction(TradeDirection::Up),
            Some("up-token".into())
        );
        assert_eq!(
            market.token_id_for_direction(TradeDirection::Down),
            Some("down-token".into())
        );
    }
}
