use anyhow::{Context, Result};
use rust_decimal::Decimal;
use std::str::FromStr;

use crate::types::TradeDirection;

#[derive(Debug, Clone)]
pub struct Config {
    pub private_key: String,
    pub dry_run: bool,
    pub allow_live_trading: bool,
    pub database_url: Option<String>,
    pub paper_trades_path: String,
    pub target_hour_utc: u32,
    pub trade_direction: TradeDirection,
    pub expected_win_rate: f64,
    pub expected_win_rate_sample_size: u64,
    pub confidence_z: f64,
    pub min_edge: f64,
    pub max_entry_price: f64,
    pub position_size_usdc: Decimal,
    pub min_liquidity: f64,
    pub min_ask_size_usd: f64,
    pub entry_delay_secs: i64,
    pub entry_window_secs: i64,
    pub poll_interval_active_secs: u64,
    pub poll_interval_idle_secs: u64,
    pub max_daily_trades: u32,
    pub gamma_base_url: String,
    pub clob_base_url: String,
    pub signature_type: String,
    pub strategy_version: String,
    pub backtest_slippage_bps: f64,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();
        Ok(Self {
            private_key: std::env::var("POLYMARKET_PRIVATE_KEY")
                .context("POLYMARKET_PRIVATE_KEY não definida")?,
            ..Self::shared_from_env()?
        })
    }

    pub fn from_env_without_private_key() -> Result<Self> {
        dotenvy::dotenv().ok();
        Ok(Self {
            private_key: std::env::var("POLYMARKET_PRIVATE_KEY").unwrap_or_default(),
            ..Self::shared_from_env()?
        })
    }

    fn shared_from_env() -> Result<Self> {
        let cfg = Self {
            private_key: String::new(),
            dry_run: env_parse("DRY_RUN", true)?,
            allow_live_trading: env_parse("ALLOW_LIVE_TRADING", false)?,
            database_url: std::env::var("DATABASE_URL")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            paper_trades_path: std::env::var("PAPER_TRADES_PATH")
                .unwrap_or_else(|_| "data/paper_trades.json".into()),
            target_hour_utc: env_parse("TARGET_HOUR_UTC", 6u32)?,
            trade_direction: std::env::var("TRADE_DIRECTION")
                .unwrap_or_else(|_| "up".into())
                .parse()?,
            expected_win_rate: env_parse("EXPECTED_WIN_RATE", 0.578f64)?,
            expected_win_rate_sample_size: env_parse("EXPECTED_WIN_RATE_SAMPLE_SIZE", 0u64)?,
            confidence_z: env_parse("EXPECTED_WIN_RATE_CONFIDENCE_Z", 1.96f64)?,
            min_edge: env_parse("MIN_EDGE", 0.02f64)?,
            max_entry_price: env_parse("MAX_ENTRY_PRICE", 0.55f64)?,
            position_size_usdc: Decimal::from_str(
                &std::env::var("POSITION_SIZE_USDC").unwrap_or_else(|_| "5.0".into()),
            )
            .context("POSITION_SIZE_USDC inválido")?,
            min_liquidity: env_parse("MIN_LIQUIDITY", 500f64)?,
            min_ask_size_usd: env_parse("MIN_ASK_SIZE_USD", 5f64)?,
            entry_delay_secs: env_parse("ENTRY_DELAY_SECS", 5i64)?,
            entry_window_secs: env_parse("ENTRY_WINDOW_SECS", 300i64)?,
            poll_interval_active_secs: env_parse("POLL_INTERVAL_ACTIVE_SECS", 10u64)?,
            poll_interval_idle_secs: env_parse("POLL_INTERVAL_IDLE_SECS", 60u64)?,
            max_daily_trades: env_parse("MAX_DAILY_TRADES", 12u32)?,
            gamma_base_url: std::env::var("GAMMA_BASE_URL")
                .unwrap_or_else(|_| "https://gamma-api.polymarket.com".into()),
            clob_base_url: std::env::var("CLOB_BASE_URL")
                .unwrap_or_else(|_| "https://clob.polymarket.com".into()),
            signature_type: std::env::var("POLYMARKET_SIGNATURE_TYPE")
                .unwrap_or_else(|_| "eoa".into())
                .to_ascii_lowercase(),
            strategy_version: std::env::var("STRATEGY_VERSION").unwrap_or_else(|_| "v1".into()),
            backtest_slippage_bps: env_parse("BACKTEST_SLIPPAGE_BPS", 50.0f64)?,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn live_trading_enabled(&self) -> bool {
        !self.dry_run && self.allow_live_trading
    }

    pub fn with_max_entry_price(mut self, value: f64) -> Self {
        self.max_entry_price = value;
        self
    }

    pub fn with_trade_direction(mut self, value: TradeDirection) -> Self {
        self.trade_direction = value;
        self
    }

    pub fn with_target_hour(mut self, value: u32) -> Self {
        self.target_hour_utc = value;
        self
    }

    /// Limite inferior de Wilson para evitar tratar uma taxa historica pontual
    /// como uma probabilidade conhecida sem incerteza amostral.
    pub fn conservative_win_rate(&self) -> Option<f64> {
        let n = self.expected_win_rate_sample_size as f64;
        if n <= 0.0 {
            return None;
        }
        let p = self.expected_win_rate;
        let z2 = self.confidence_z * self.confidence_z;
        let denominator = 1.0 + z2 / n;
        let center = p + z2 / (2.0 * n);
        let margin = self.confidence_z * ((p * (1.0 - p) + z2 / (4.0 * n)) / n).sqrt();
        Some(((center - margin) / denominator).clamp(0.0, 1.0))
    }

    fn validate(&self) -> Result<()> {
        if self.target_hour_utc > 23 {
            anyhow::bail!("TARGET_HOUR_UTC deve estar entre 0 e 23");
        }
        for (name, value) in [
            ("EXPECTED_WIN_RATE", self.expected_win_rate),
            ("MIN_EDGE", self.min_edge),
            ("MAX_ENTRY_PRICE", self.max_entry_price),
        ] {
            if !(0.0..=1.0).contains(&value) {
                anyhow::bail!("{name} deve estar entre 0 e 1");
            }
        }
        if self.entry_window_secs < self.entry_delay_secs {
            anyhow::bail!("ENTRY_WINDOW_SECS deve ser >= ENTRY_DELAY_SECS");
        }
        if self.position_size_usdc <= Decimal::ZERO {
            anyhow::bail!("POSITION_SIZE_USDC deve ser positivo");
        }
        if self.min_liquidity < 0.0 || self.min_ask_size_usd <= 0.0 {
            anyhow::bail!("MIN_LIQUIDITY/MIN_ASK_SIZE_USD invalidos");
        }
        Ok(())
    }
}

fn env_parse<T>(key: &str, default: T) -> Result<T>
where
    T: std::str::FromStr + Copy + ToString,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    std::env::var(key)
        .unwrap_or_else(|_| default.to_string())
        .parse::<T>()
        .map_err(|e| anyhow::anyhow!("{key} inválido: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wilson_bound_is_below_point_estimate() {
        let mut cfg = Config::shared_from_env().unwrap();
        cfg.expected_win_rate = 0.578;
        cfg.expected_win_rate_sample_size = 100;
        let lower = cfg.conservative_win_rate().unwrap();
        assert!(lower < cfg.expected_win_rate);
        assert!(lower > 0.0);
    }

    #[test]
    fn missing_sample_size_disables_probability() {
        let mut cfg = Config::shared_from_env().unwrap();
        cfg.expected_win_rate_sample_size = 0;
        assert_eq!(cfg.conservative_win_rate(), None);
    }
}
