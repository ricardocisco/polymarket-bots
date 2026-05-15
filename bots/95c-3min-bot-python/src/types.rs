use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeSide {
    Up,
    Down,
}

impl Display for TradeSide {
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
pub struct MarketConfig {
    pub asset: String,
    pub duration_minutes: u32,
    pub binance_symbol: String,
}

impl MarketConfig {
    pub fn slug_prefix(&self) -> String {
        format!(
            "{}-updown-{}m",
            self.asset.to_ascii_lowercase(),
            self.duration_minutes
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyMarket {
    pub id: String,
    pub slug: String,
    pub question: String,
    pub description: Option<String>,
    pub up_token_id: String,
    pub down_token_id: String,
    pub strike_price: f64,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub up_price: f64,
    pub down_price: f64,
    pub active: bool,
    pub closed: bool,
    pub accepting_orders: bool,
    pub outcome_prices: Vec<f64>,
    pub config: MarketConfig,
}

impl StrategyMarket {
    pub fn duration(&self) -> Duration {
        self.end_time - self.start_time
    }

    pub fn minutes_left_at(&self, now: DateTime<Utc>) -> f64 {
        ((self.end_time - now).num_milliseconds() as f64 / 60_000.0).max(0.0)
    }

    pub fn token_id(&self, side: TradeSide) -> &str {
        match side {
            TradeSide::Up => &self.up_token_id,
            TradeSide::Down => &self.down_token_id,
        }
    }

    pub fn price(&self, side: TradeSide) -> f64 {
        match side {
            TradeSide::Up => self.up_price,
            TradeSide::Down => self.down_price,
        }
    }

    pub fn winner_from_outcome_prices(&self) -> Option<TradeSide> {
        let up = *self.outcome_prices.first()?;
        let down = *self.outcome_prices.get(1)?;
        if up > down {
            Some(TradeSide::Up)
        } else if down > up {
            Some(TradeSide::Down)
        } else {
            None
        }
    }

    pub fn display_label(&self) -> String {
        format!(
            "{} {}m | strike={:.4} | {}",
            self.config.asset, self.config.duration_minutes, self.strike_price, self.slug
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub open_time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSignal {
    pub symbol: String,
    pub current_price: f64,
    pub strike_price: f64,
    pub distance_pct: f64,
    pub rsi: f64,
    pub macd_signal: f64,
    pub price_momentum: f64,
    pub up_probability: f64,
    pub confidence: String,
    pub recommended_side: Option<TradeSide>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyDiagnostics {
    pub execution_mode: String,
    pub size: Option<u32>,
    pub confidence: Option<f64>,
    pub order_type: Option<String>,
    pub market_price: Option<f64>,
    pub min_entry_price: Option<f64>,
    pub max_entry_price: Option<f64>,
    pub minutes_left: Option<f64>,
    pub distance_pct: Option<f64>,
    pub rsi: Option<f64>,
    pub momentum: Option<f64>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalDecision {
    pub action: Action,
    pub side: Option<TradeSide>,
    pub size: u32,
    pub reason: String,
    pub order_type: Option<OrderTimeInForce>,
    pub limit_price_cents: Option<f64>,
    pub confidence: Option<f64>,
    pub diagnostics: Option<StrategyDiagnostics>,
    pub stake_usdc: Option<f64>,
}

impl SignalDecision {
    pub fn hold(reason: impl Into<String>) -> Self {
        Self {
            action: Action::Hold,
            side: None,
            size: 0,
            reason: reason.into(),
            order_type: None,
            limit_price_cents: None,
            confidence: None,
            diagnostics: None,
            stake_usdc: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PricePoint {
    pub t: i64,
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
    pub market_slug: String,
    pub market_ticker: String,
    pub asset: String,
    pub interval_minutes: u32,
    pub token_id: String,
    pub strike_price: f64,
    pub close_ts: i64,
    pub submitted_at: i64,
    pub side: TradeSide,
    pub size: u32,
    pub entry_price_cents: u32,
    pub stake_usdc: f64,
    pub underlying_price_usd: f64,
    pub seconds_to_close: f64,
    pub decision_reason: String,
    pub diagnostics: Option<StrategyDiagnostics>,
    pub status: PaperTradeStatus,
    pub winner_side: Option<TradeSide>,
    pub final_price_usd: Option<f64>,
    pub pnl_cents: Option<i64>,
    pub settled_at: Option<i64>,
    pub dry_run: bool,
    pub order_id: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct OpenTradeGroup {
    pub market_slug: String,
    pub close_ts: i64,
    pub asset: String,
    pub interval_minutes: u32,
    pub count: u64,
}

#[derive(Debug, Clone)]
pub struct BacktestTrade {
    pub market_slug: String,
    pub asset: String,
    pub interval_minutes: u32,
    pub entry_ts: i64,
    pub side: TradeSide,
    pub entry_price: f64,
    pub final_price: f64,
    pub pnl: f64,
    pub won: bool,
    pub reason: String,
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
