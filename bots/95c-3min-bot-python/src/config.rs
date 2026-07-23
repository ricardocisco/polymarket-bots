use anyhow::Result;

use crate::types::MarketConfig;

#[derive(Debug, Clone)]
pub struct Config {
    pub private_key: String,
    pub dry_run: bool,
    pub allow_live_trading: bool,
    pub bankroll: f64,
    pub position_size_usdc: f64,
    pub min_edge: f64,
    pub min_entry_price: f64,
    pub max_entry_price: f64,
    pub min_minutes_left: f64,
    pub max_minutes_left: f64,
    pub min_ask_size_usd: f64,
    pub loop_interval_secs: u64,
    pub gamma_base_url: String,
    pub clob_base_url: String,
    pub binance_base_url: String,
    pub paper_trades_path: String,
    pub signature_type: String,
    pub strategy_version: String,
    pub markets: Vec<MarketConfig>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let cfg = Self {
            private_key: std::env::var("POLYMARKET_PRIVATE_KEY")
                .or_else(|_| std::env::var("PRIVATE_KEY"))
                .unwrap_or_default(),
            dry_run: env_parse("DRY_RUN", env_parse("SNIPER_DRY_RUN", true)?)?,
            allow_live_trading: env_parse("ALLOW_LIVE_TRADING", false)?,
            bankroll: env_parse("BANKROLL", 20.0)?,
            position_size_usdc: env_parse("POSITION_SIZE_USDC", 2.0)?,
            min_edge: env_parse("MIN_EDGE", 0.04)?,
            min_entry_price: env_parse("MIN_ENTRY_PRICE", 0.95)?,
            max_entry_price: env_parse("MAX_ENTRY_PRICE", 0.995)?,
            min_minutes_left: env_parse("MIN_MINUTES_LEFT", 0.5)?,
            max_minutes_left: env_parse("MAX_MINUTES_LEFT", 3.0)?,
            min_ask_size_usd: env_parse("MIN_ASK_SIZE_USD", 1.0)?,
            loop_interval_secs: env_parse("LOOP_INTERVAL", env_parse("POLL_INTERVAL", 5u64)?)?,
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
                .unwrap_or_else(|_| "rust-v2".into()),
            markets: default_markets(),
        };
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn live_trading_enabled(&self) -> bool {
        !self.dry_run && self.allow_live_trading
    }

    pub fn strategy_name(&self) -> &'static str {
        "sniper_95c_3min"
    }

    pub fn with_entry_band(mut self, min_entry_price: f64, max_entry_price: f64) -> Self {
        self.min_entry_price = min_entry_price;
        self.max_entry_price = max_entry_price;
        self
    }

    fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.min_edge)
            || !(0.0..1.0).contains(&self.min_entry_price)
            || !(0.0..1.0).contains(&self.max_entry_price)
            || self.min_entry_price > self.max_entry_price
        {
            anyhow::bail!("MIN_EDGE e faixa de entrada invalidos");
        }
        if self.bankroll <= 0.0
            || self.position_size_usdc <= 0.0
            || self.position_size_usdc > self.bankroll
        {
            anyhow::bail!("BANKROLL/POSITION_SIZE_USDC invalidos");
        }
        if self.min_minutes_left < 0.0
            || self.min_minutes_left > self.max_minutes_left
            || self.loop_interval_secs == 0
        {
            anyhow::bail!("janela temporal ou LOOP_INTERVAL invalido");
        }
        Ok(())
    }
}

fn default_markets() -> Vec<MarketConfig> {
    let assets = [("BTC", "BTCUSDT"), ("ETH", "ETHUSDT"), ("XRP", "XRPUSDT")];
    let mut out = Vec::new();
    for duration_minutes in [15, 5] {
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
