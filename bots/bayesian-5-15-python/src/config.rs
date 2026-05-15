use anyhow::Result;

use crate::types::MarketConfig;

#[derive(Debug, Clone)]
pub struct Config {
    pub private_key: String,
    pub dry_run: bool,
    pub allow_live_trading: bool,
    pub bankroll: f64,
    pub mode: TradingMode,
    pub bayesian: BayesianParams,
    pub kelly: KellyParams,
    pub risk: RiskParams,
    pub filters: SmartFilters,
    pub min_buy_price: f64,
    pub max_buy_price: f64,
    pub min_ask_size_usd: f64,
    pub flat_stake_usdc: Option<f64>,
    pub loop_interval_secs: u64,
    pub gamma_base_url: String,
    pub clob_base_url: String,
    pub binance_base_url: String,
    pub paper_trades_path: String,
    pub signature_type: String,
    pub strategy_version: String,
    pub markets: Vec<MarketConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingMode {
    Conservative,
    Aggressive,
    AggressiveOptimized,
    Degen,
}

#[derive(Debug, Clone)]
pub struct BayesianParams {
    pub prior_up: f64,
    pub prior_down: f64,
    pub momentum_weight: f64,
    pub volume_weight: f64,
    pub volatility_weight: f64,
    pub trend_weight: f64,
    pub min_trade_edge: f64,
    pub rsi_extreme_boost: f64,
    pub strike_penalty_factor: f64,
    pub min_signal_strength: f64,
    pub rsi_overbought: f64,
    pub rsi_oversold: f64,
    pub rsi_extreme: f64,
    pub rsi_neutral_low: f64,
    pub rsi_neutral_high: f64,
    pub trend_max_confidence: f64,
    pub trend_min_gap_pct: f64,
    pub volume_relative_threshold: f64,
    pub volume_extreme_threshold: f64,
}

#[derive(Debug, Clone)]
pub struct KellyParams {
    pub kelly_fraction: f64,
    pub max_position_size: f64,
    pub min_position_size: f64,
    pub min_edge: f64,
    pub max_bankroll_per_trade: f64,
    pub loss_reduction_factor: f64,
    pub win_increase_factor: f64,
}

#[derive(Debug, Clone)]
pub struct RiskParams {
    pub max_concurrent_positions: usize,
    pub max_drawdown: f64,
    pub loss_cooldown_minutes: u64,
    pub max_consecutive_losses: u32,
}

#[derive(Debug, Clone)]
pub struct SmartFilters {
    pub enabled: bool,
    pub allow_down_trades: bool,
    pub momentum_5m_min: f64,
    pub volume_ratio_max: Option<f64>,
    pub filter_by_hour: bool,
    pub blocked_hours: Vec<u32>,
    pub preferred_hours: Vec<u32>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let mode = TradingMode::from_env_value(
            &std::env::var("BAYESIAN_MODE")
                .or_else(|_| std::env::var("TRADING_MODE"))
                .unwrap_or_else(|_| "AGGRESSIVE_OPTIMIZED".into()),
        )?;
        let (bayesian, kelly, risk, filters) = mode.params();
        let flat = env_parse("BAYESIAN_FLAT_STAKE_USDC", 1.0)?;

        Ok(Self {
            private_key: std::env::var("POLYMARKET_PRIVATE_KEY")
                .or_else(|_| std::env::var("PRIVATE_KEY"))
                .unwrap_or_default(),
            dry_run: env_bool("BAYESIAN_DRY_RUN", env_bool("DRY_RUN", true)?)?,
            allow_live_trading: env_bool("ALLOW_LIVE_TRADING", false)?,
            bankroll: env_parse("BAYESIAN_BANKROLL", env_parse("BANKROLL", 20.0)?)?,
            mode,
            bayesian,
            kelly,
            risk,
            filters,
            min_buy_price: env_parse("BAYESIAN_MIN_BUY_PRICE", 0.50)?,
            max_buy_price: env_parse("BAYESIAN_MAX_BUY_PRICE", 0.58)?,
            min_ask_size_usd: env_parse("BAYESIAN_MIN_ASK_SIZE_USD", 5.0)?,
            flat_stake_usdc: if flat > 0.0 { Some(flat) } else { None },
            loop_interval_secs: env_parse("LOOP_INTERVAL", 5u64)?,
            gamma_base_url: std::env::var("GAMMA_BASE_URL")
                .unwrap_or_else(|_| "https://gamma-api.polymarket.com".into()),
            clob_base_url: std::env::var("CLOB_BASE_URL")
                .or_else(|_| std::env::var("CLOB_URL"))
                .unwrap_or_else(|_| "https://clob.polymarket.com".into()),
            binance_base_url: std::env::var("BINANCE_BASE_URL")
                .unwrap_or_else(|_| "https://api.binance.com".into()),
            paper_trades_path: std::env::var("PAPER_TRADES_PATH")
                .unwrap_or_else(|_| "data/paper_trades.json".into()),
            signature_type: std::env::var("POLYMARKET_SIGNATURE_TYPE")
                .or_else(|_| std::env::var("SIGNATURE_TYPE"))
                .unwrap_or_else(|_| "proxy".into())
                .to_ascii_lowercase(),
            strategy_version: std::env::var("STRATEGY_VERSION")
                .unwrap_or_else(|_| "rust-bayes-v1".into()),
            markets: default_markets(),
        })
    }

    pub fn live_trading_enabled(&self) -> bool {
        !self.dry_run && self.allow_live_trading
    }

    pub fn strategy_name(&self) -> &'static str {
        "bayesian_5_15"
    }

    pub fn with_entry_band(mut self, min_buy_price: f64, max_buy_price: f64) -> Self {
        self.min_buy_price = min_buy_price;
        self.max_buy_price = max_buy_price;
        self
    }
}

impl TradingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Conservative => "CONSERVATIVE",
            Self::Aggressive => "AGGRESSIVE",
            Self::AggressiveOptimized => "AGGRESSIVE_OPTIMIZED",
            Self::Degen => "DEGEN",
        }
    }

    fn from_env_value(value: &str) -> Result<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "CONSERVATIVE" => Ok(Self::Conservative),
            "AGGRESSIVE" => Ok(Self::Aggressive),
            "AGGRESSIVE_OPTIMIZED" | "OPTIMIZED" => Ok(Self::AggressiveOptimized),
            "DEGEN" => Ok(Self::Degen),
            other => anyhow::bail!(
                "BAYESIAN_MODE invalido: {other}. Use CONSERVATIVE, AGGRESSIVE, AGGRESSIVE_OPTIMIZED ou DEGEN"
            ),
        }
    }

    fn params(&self) -> (BayesianParams, KellyParams, RiskParams, SmartFilters) {
        let mut bayes = base_bayesian();
        let mut kelly = base_kelly();
        let mut risk = base_risk();
        let mut filters = SmartFilters {
            enabled: false,
            allow_down_trades: true,
            momentum_5m_min: 0.0,
            volume_ratio_max: None,
            filter_by_hour: false,
            blocked_hours: vec![],
            preferred_hours: vec![],
        };

        match self {
            Self::Conservative => {
                bayes.prior_up = 0.45;
                bayes.prior_down = 0.55;
                bayes.momentum_weight = 0.35;
                bayes.volume_weight = 0.20;
                bayes.volatility_weight = 0.10;
                bayes.trend_weight = 0.35;
                bayes.strike_penalty_factor = 50.0;
                bayes.trend_max_confidence = 0.65;

                kelly.kelly_fraction = 0.20;
                kelly.max_position_size = 5.0;
                kelly.min_position_size = 2.50;
                kelly.min_edge = 0.05;
                kelly.max_bankroll_per_trade = 0.15;
                kelly.win_increase_factor = 1.05;

                risk.max_concurrent_positions = 2;
                risk.max_drawdown = 0.15;
                risk.loss_cooldown_minutes = 60;
                risk.max_consecutive_losses = 2;
            }
            Self::Aggressive => {
                bayes.prior_up = 0.45;
                bayes.prior_down = 0.55;
                bayes.momentum_weight = 0.35;
                bayes.volume_weight = 0.20;
                bayes.volatility_weight = 0.10;
                bayes.trend_weight = 0.35;
                bayes.strike_penalty_factor = 50.0;
                bayes.trend_max_confidence = 0.65;

                kelly.kelly_fraction = 0.35;
                kelly.max_position_size = 5.0;
                kelly.min_position_size = 2.50;
                kelly.min_edge = 0.05;
                kelly.max_bankroll_per_trade = 0.13;
                kelly.win_increase_factor = 1.10;

                risk.max_concurrent_positions = 2;
                risk.max_drawdown = 0.25;
                risk.loss_cooldown_minutes = 30;
                risk.max_consecutive_losses = 3;
            }
            Self::AggressiveOptimized => {
                bayes.prior_up = 0.55;
                bayes.prior_down = 0.45;
                bayes.momentum_weight = 0.35;
                bayes.volume_weight = 0.15;
                bayes.volatility_weight = 0.15;
                bayes.trend_weight = 0.35;
                bayes.strike_penalty_factor = 50.0;
                bayes.trend_max_confidence = 0.65;

                kelly.kelly_fraction = 0.35;
                kelly.max_position_size = 5.0;
                kelly.min_position_size = 2.50;
                kelly.min_edge = 0.05;
                kelly.max_bankroll_per_trade = 0.13;
                kelly.loss_reduction_factor = 0.40;
                kelly.win_increase_factor = 1.05;

                risk.max_concurrent_positions = 2;
                risk.max_drawdown = 0.20;
                risk.loss_cooldown_minutes = 45;
                risk.max_consecutive_losses = 3;

                filters.enabled = true;
                filters.allow_down_trades = false;
                filters.momentum_5m_min = 0.0005;
                filters.volume_ratio_max = Some(1.5);
                filters.filter_by_hour = true;
                filters.blocked_hours = vec![2, 3, 5, 12, 18, 19, 22];
            }
            Self::Degen => {
                bayes.prior_up = 0.45;
                bayes.prior_down = 0.55;
                bayes.momentum_weight = 0.40;
                bayes.volume_weight = 0.25;
                bayes.volatility_weight = 0.05;
                bayes.trend_weight = 0.30;
                bayes.rsi_extreme_boost = 2.0;
                bayes.strike_penalty_factor = 30.0;
                bayes.trend_max_confidence = 0.70;

                kelly.kelly_fraction = 0.50;
                kelly.max_position_size = 15.0;
                kelly.min_position_size = 2.50;
                kelly.min_edge = 0.01;
                kelly.max_bankroll_per_trade = 0.40;
                kelly.loss_reduction_factor = 0.70;
                kelly.win_increase_factor = 1.20;

                risk.max_concurrent_positions = 6;
                risk.max_drawdown = 0.40;
                risk.loss_cooldown_minutes = 15;
                risk.max_consecutive_losses = 5;
            }
        }

        (bayes, kelly, risk, filters)
    }
}

fn base_bayesian() -> BayesianParams {
    BayesianParams {
        prior_up: 0.62,
        prior_down: 0.38,
        momentum_weight: 0.40,
        volume_weight: 0.10,
        volatility_weight: 0.10,
        trend_weight: 0.40,
        min_trade_edge: 0.08,
        rsi_extreme_boost: 1.5,
        strike_penalty_factor: 30.0,
        min_signal_strength: 0.60,
        rsi_overbought: 68.0,
        rsi_oversold: 32.0,
        rsi_extreme: 78.0,
        rsi_neutral_low: 42.0,
        rsi_neutral_high: 58.0,
        trend_max_confidence: 0.72,
        trend_min_gap_pct: 0.10,
        volume_relative_threshold: 1.5,
        volume_extreme_threshold: 2.5,
    }
}

fn base_kelly() -> KellyParams {
    KellyParams {
        kelly_fraction: 0.35,
        max_position_size: 5.0,
        min_position_size: 2.50,
        min_edge: 0.07,
        max_bankroll_per_trade: 0.13,
        loss_reduction_factor: 0.5,
        win_increase_factor: 1.1,
    }
}

fn base_risk() -> RiskParams {
    RiskParams {
        max_concurrent_positions: 2,
        max_drawdown: 0.25,
        loss_cooldown_minutes: 30,
        max_consecutive_losses: 3,
    }
}

fn default_markets() -> Vec<MarketConfig> {
    let assets = [
        ("BTC", "BTCUSDT"),
        ("ETH", "ETHUSDT"),
        ("SOL", "SOLUSDT"),
        ("XRP", "XRPUSDT"),
    ];
    let mut out = Vec::new();
    for duration_minutes in [5, 15] {
        for (asset, symbol) in assets {
            out.push(MarketConfig {
                asset: asset.into(),
                duration_minutes,
                binance_symbol: symbol.into(),
            });
        }
    }
    out
}

fn env_bool(key: &str, default: bool) -> Result<bool> {
    Ok(std::env::var(key)
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default))
}

fn env_parse<T>(key: &str, default: T) -> Result<T>
where
    T: std::str::FromStr + Copy + ToString,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    std::env::var(key)
        .unwrap_or_else(|_| default.to_string())
        .parse::<T>()
        .map_err(|err| anyhow::anyhow!("{key} invalido: {err}"))
}
