use std::str::FromStr;

use alloy_primitives::U256;
use anyhow::{Context, Result};
use polymarket_client_sdk::auth::state::Unauthenticated;
use polymarket_client_sdk::auth::{LocalSigner, Signer};
use polymarket_client_sdk::clob::types::{Side, SignatureType};
use polymarket_client_sdk::clob::{Client, Config as ClobConfig};
use polymarket_client_sdk::POLYGON;
use rust_decimal::Decimal;

use crate::config::Config;

#[derive(Debug, Clone)]
pub struct OrderIntent {
    pub market_slug: String,
    pub token_id: String,
    pub price: f64,
    pub stake_usdc: f64,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ExecutionUpdate {
    pub order_id: Option<String>,
    pub simulated: bool,
    pub submitted_price: Option<f64>,
}

pub enum ExchangeExecutor {
    DryRun,
    Live(LiveExchangeExecutor),
}

pub struct LiveExchangeExecutor {
    private_key: String,
    clob_base_url: String,
    signature_type: String,
}

impl ExchangeExecutor {
    pub async fn from_config(cfg: &Config) -> Result<Self> {
        if !cfg.live_trading_enabled() {
            return Ok(Self::DryRun);
        }
        if cfg.private_key.trim().is_empty() {
            anyhow::bail!("POLYMARKET_PRIVATE_KEY/PRIVATE_KEY nao definida para live trading");
        }
        Ok(Self::Live(LiveExchangeExecutor {
            private_key: cfg.private_key.clone(),
            clob_base_url: cfg.clob_base_url.clone(),
            signature_type: cfg.signature_type.clone(),
        }))
    }

    pub async fn submit(&self, intent: &OrderIntent) -> Result<ExecutionUpdate> {
        match self {
            Self::DryRun => Ok(ExecutionUpdate {
                order_id: Some(format!("dryrun-{}", intent.market_slug)),
                simulated: true,
                submitted_price: Some(intent.price),
            }),
            Self::Live(exec) => exec.submit(intent).await,
        }
    }
}

impl LiveExchangeExecutor {
    async fn submit(&self, intent: &OrderIntent) -> Result<ExecutionUpdate> {
        let signer = LocalSigner::from_str(&self.private_key)
            .context("private key invalida")?
            .with_chain_id(Some(POLYGON));

        let builder = Client::<Unauthenticated>::new(&self.clob_base_url, ClobConfig::default())?
            .authentication_builder(&signer);
        let builder = match self.signature_type.as_str() {
            "1" | "proxy" | "poly_proxy" => builder.signature_type(SignatureType::Proxy),
            "2" | "gnosis" | "gnosis_safe" | "safe" => {
                builder.signature_type(SignatureType::GnosisSafe)
            }
            _ => builder,
        };
        let client = builder
            .authenticate()
            .await
            .context("falha ao autenticar no CLOB")?;

        let shares = decimal_shares(intent.stake_usdc, intent.price)?;
        let token_id: U256 = intent.token_id.parse().context("token_id invalido")?;
        let price = Decimal::from_str(&format!("{:.6}", intent.price))
            .context("falha ao converter preco")?;

        let order = client
            .limit_order()
            .token_id(token_id)
            .price(price)
            .size(shares)
            .side(Side::Buy)
            .build()
            .await?;
        let signed = client.sign(&signer, order).await?;
        let resp = client.post_order(signed).await?;

        Ok(ExecutionUpdate {
            order_id: Some(format!("{:?}", resp)),
            simulated: false,
            submitted_price: Some(intent.price),
        })
    }
}

fn decimal_shares(stake_usdc: f64, price: f64) -> Result<Decimal> {
    if price <= 0.0 {
        anyhow::bail!("preco invalido para calcular shares");
    }
    let stake = Decimal::from_str(&format!("{stake_usdc:.6}"))?;
    let price = Decimal::from_str(&format!("{price:.6}"))?;
    Ok((stake / price).round_dp(6))
}
