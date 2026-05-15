// src/config.rs
//! Carrega e valida configuração via variáveis de ambiente (.env).

use anyhow::{Context, Result};
use rust_decimal::Decimal;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct Config {
    pub private_key: String,
    pub min_confidence: f64,
    pub max_position_size_usdc: Decimal,
    pub min_order_size_usdc: Decimal,
    pub run_interval_secs: u64,

    // ── Dry-run (padrão do first-minute-followthrough) ─────────────────────
    /// DRY_RUN=true  → nunca executa ordens reais (padrão: true)
    pub dry_run: bool,
    /// Trava extra: live trading só ativa se dry_run=false E allow_live_trading=true
    pub allow_live_trading: bool,

    // ── Estratégia Penny (300 shares a 1 centavo) ──────────────────────────
    /// Shares fixos por entrada. 0 = desabilitado, usa Quarter-Kelly padrão.
    /// Padrão env PENNY_SHARES=300  →  300 shares × $0.01 = $3 por entrada
    pub penny_shares: u32,
    /// Preço máximo (0–1) para qualificar como penny play (ex: 0.02 = 2 centavos)
    pub penny_max_price: f64,
    /// Confiança mínima para penny plays (pode ser menor que min_confidence,
    /// pois o payout 100:1 justifica edge menor)
    pub penny_min_confidence: f64,

    // ── Horizonte de busca de mercados futuros ─────────────────────────────
    /// Dias à frente para buscar mercados (padrão 3; use 14 para penny scan)
    pub extended_horizon_days: i64,

    // ── Event-driven / detecção de mudanças ────────────────────────────────
    /// Intervalo base de polling em segundos (adaptado dinamicamente pela hora do dia).
    /// Durante horas ativas (10-18h local) o intervalo é reduzido a 1/3 deste valor.
    /// Padrão: 300s (5 min durante horas ativas → "webhook simulado")
    pub change_poll_secs: u64,
    /// Variação mínima de temperatura (°C) para re-avaliar um mercado.
    /// Mudanças menores que este valor são ignoradas (sem novo trade).
    pub temp_change_threshold: f64,
    /// Variação mínima de preço (0-1) no mercado para re-avaliar.
    pub price_change_threshold: f64,
    /// Habilita análise intradiária horária para detectar pico confirmado do dia.
    pub use_intraday_trend: bool,
    /// Confiança mínima para trades baseados em tendência intradiária antecipada.
    /// Mais alto que min_confidence porque requer pico confirmado — muito mais preciso.
    pub anticipatory_min_confidence: f64,
    /// Refresh da discovery dinâmica de mercados.
    pub discovery_refresh_secs: u64,
    /// Poll de resolução para posições abertas.
    pub resolution_poll_secs: u64,
    /// Poll de forecast para mercados D+3 ou mais.
    pub weather_poll_d3_secs: u64,
    /// Poll de forecast para mercados D+2.
    pub weather_poll_d2_secs: u64,
    /// Poll de forecast para mercados D+1.
    pub weather_poll_d1_secs: u64,
    /// Poll intradiário para mercados do próprio dia.
    pub weather_intraday_poll_secs: u64,
    /// Edge mínimo por share em dólares/probabilidade.
    pub edge_min: f64,
    /// Spread máximo tolerado para executar.
    pub max_spread_cents: f64,
    /// Revisão mínima de forecast para reavaliar imediatamente.
    pub forecast_change_trigger_degrees: f64,
    /// Mudança mínima no preço implícito para reavaliar imediatamente.
    pub implied_move_trigger_cents: f64,

    // ── Consensus multi-source ─────────────────────────────────────────────
    /// Número mínimo de fontes que devem ter dados para calcular consensus.
    pub num_sources_required: usize,
    /// Spread máximo em graus entre fontes para considerar que estão em acordo.
    /// Se o spread for maior, o bot usa confiança reduzida ou pula.
    pub source_agreement_threshold: f64,
    /// Confiança mínima do consensus para entrar num trade via estratégia cross-market.
    pub consensus_min_confidence: f64,
    /// Preço máximo para comprar NO em bins adjacentes ao bin previsto (penny play cross-market).
    /// Ex: 0.15 = compra NO em bins adjacentes cotados até 15¢
    pub penny_no_max_price: f64,
    /// Raio de bins adjacentes ao bin previsto que o bot considera para NO penny play.
    /// Ex: 2 = considera até 2 bins acima e abaixo do bin correto
    pub cross_market_bins_radius: usize,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        Ok(Config {
            private_key: std::env::var("POLYMARKET_PRIVATE_KEY")
                .context("POLYMARKET_PRIVATE_KEY não definida. Configure no arquivo .env")?,

            // Padrão 0.72 corresponde à confiança de D+1 (horizonte de 1 dia)
            // calibrada contra o MAE histórico Open-Meteo vs ASOS.
            min_confidence: std::env::var("MIN_CONFIDENCE")
                .unwrap_or_else(|_| "0.72".into())
                .parse::<f64>()
                .context("MIN_CONFIDENCE deve ser float (ex: 0.72)")?,

            max_position_size_usdc: Decimal::from_str(
                &std::env::var("MAX_POSITION_SIZE_USDC").unwrap_or_else(|_| "10.0".into()),
            )
            .context("MAX_POSITION_SIZE_USDC inválido")?,

            min_order_size_usdc: Decimal::from_str(
                &std::env::var("MIN_ORDER_SIZE_USDC").unwrap_or_else(|_| "1.0".into()),
            )
            .context("MIN_ORDER_SIZE_USDC inválido")?,

            run_interval_secs: std::env::var("RUN_INTERVAL_SECS")
                .unwrap_or_else(|_| "3600".into())
                .parse::<u64>()
                .context("RUN_INTERVAL_SECS deve ser inteiro positivo")?,

            dry_run: std::env::var("DRY_RUN")
                .unwrap_or_else(|_| "true".into())
                .parse::<bool>()
                .context("DRY_RUN deve ser true ou false")?,

            allow_live_trading: std::env::var("ALLOW_LIVE_TRADING")
                .unwrap_or_else(|_| "false".into())
                .parse::<bool>()
                .context("ALLOW_LIVE_TRADING deve ser true ou false")?,

            penny_shares: std::env::var("PENNY_SHARES")
                .unwrap_or_else(|_| "0".into())
                .parse::<u32>()
                .context("PENNY_SHARES deve ser inteiro não-negativo")?,

            penny_max_price: std::env::var("PENNY_MAX_PRICE")
                .unwrap_or_else(|_| "0.02".into())
                .parse::<f64>()
                .context("PENNY_MAX_PRICE deve ser float (ex: 0.02)")?,

            penny_min_confidence: std::env::var("PENNY_MIN_CONFIDENCE")
                .unwrap_or_else(|_| "0.40".into())
                .parse::<f64>()
                .context("PENNY_MIN_CONFIDENCE deve ser float (ex: 0.40)")?,

            extended_horizon_days: std::env::var("EXTENDED_HORIZON_DAYS")
                .unwrap_or_else(|_| "3".into())
                .parse::<i64>()
                .context("EXTENDED_HORIZON_DAYS deve ser inteiro positivo")?,

            change_poll_secs: std::env::var("CHANGE_POLL_SECS")
                .unwrap_or_else(|_| "300".into())
                .parse::<u64>()
                .context("CHANGE_POLL_SECS deve ser inteiro positivo (segundos)")?,

            temp_change_threshold: std::env::var("TEMP_CHANGE_THRESHOLD")
                .unwrap_or_else(|_| "0.3".into())
                .parse::<f64>()
                .context("TEMP_CHANGE_THRESHOLD deve ser float (ex: 0.3)")?,

            price_change_threshold: std::env::var("PRICE_CHANGE_THRESHOLD")
                .unwrap_or_else(|_| "0.03".into())
                .parse::<f64>()
                .context("PRICE_CHANGE_THRESHOLD deve ser float (ex: 0.03)")?,

            use_intraday_trend: std::env::var("USE_INTRADAY_TREND")
                .unwrap_or_else(|_| "true".into())
                .parse::<bool>()
                .context("USE_INTRADAY_TREND deve ser true ou false")?,

            anticipatory_min_confidence: std::env::var("ANTICIPATORY_MIN_CONFIDENCE")
                .unwrap_or_else(|_| "0.88".into())
                .parse::<f64>()
                .context("ANTICIPATORY_MIN_CONFIDENCE deve ser float (ex: 0.88)")?,

            discovery_refresh_secs: std::env::var("DISCOVERY_REFRESH_SECS")
                .unwrap_or_else(|_| "300".into())
                .parse::<u64>()
                .context("DISCOVERY_REFRESH_SECS deve ser inteiro positivo")?,

            resolution_poll_secs: std::env::var("RESOLUTION_POLL_SECS")
                .unwrap_or_else(|_| "180".into())
                .parse::<u64>()
                .context("RESOLUTION_POLL_SECS deve ser inteiro positivo")?,

            weather_poll_d3_secs: std::env::var("WEATHER_POLL_D3_SECS")
                .unwrap_or_else(|_| "3600".into())
                .parse::<u64>()
                .context("WEATHER_POLL_D3_SECS deve ser inteiro positivo")?,

            weather_poll_d2_secs: std::env::var("WEATHER_POLL_D2_SECS")
                .unwrap_or_else(|_| "1800".into())
                .parse::<u64>()
                .context("WEATHER_POLL_D2_SECS deve ser inteiro positivo")?,

            weather_poll_d1_secs: std::env::var("WEATHER_POLL_D1_SECS")
                .unwrap_or_else(|_| "900".into())
                .parse::<u64>()
                .context("WEATHER_POLL_D1_SECS deve ser inteiro positivo")?,

            weather_intraday_poll_secs: std::env::var("WEATHER_INTRADAY_POLL_SECS")
                .unwrap_or_else(|_| "180".into())
                .parse::<u64>()
                .context("WEATHER_INTRADAY_POLL_SECS deve ser inteiro positivo")?,

            edge_min: std::env::var("EDGE_MIN")
                .unwrap_or_else(|_| "0.02".into())
                .parse::<f64>()
                .context("EDGE_MIN deve ser float (ex: 0.02)")?,

            max_spread_cents: std::env::var("MAX_SPREAD_CENTS")
                .unwrap_or_else(|_| "5.0".into())
                .parse::<f64>()
                .context("MAX_SPREAD_CENTS deve ser float (ex: 5.0)")?,

            forecast_change_trigger_degrees: std::env::var("FORECAST_CHANGE_TRIGGER_DEGREES")
                .unwrap_or_else(|_| "0.4".into())
                .parse::<f64>()
                .context("FORECAST_CHANGE_TRIGGER_DEGREES deve ser float (ex: 0.4)")?,

            implied_move_trigger_cents: std::env::var("IMPLIED_MOVE_TRIGGER_CENTS")
                .unwrap_or_else(|_| "2.0".into())
                .parse::<f64>()
                .context("IMPLIED_MOVE_TRIGGER_CENTS deve ser float (ex: 2.0)")?,

            num_sources_required: std::env::var("NUM_SOURCES_REQUIRED")
                .unwrap_or_else(|_| "3".into())
                .parse::<usize>()
                .context("NUM_SOURCES_REQUIRED deve ser inteiro positivo")?,

            source_agreement_threshold: std::env::var("SOURCE_AGREEMENT_THRESHOLD")
                .unwrap_or_else(|_| "1.5".into())
                .parse::<f64>()
                .context("SOURCE_AGREEMENT_THRESHOLD deve ser float em graus (ex: 1.5)")?,

            consensus_min_confidence: std::env::var("CONSENSUS_MIN_CONFIDENCE")
                .unwrap_or_else(|_| "0.80".into())
                .parse::<f64>()
                .context("CONSENSUS_MIN_CONFIDENCE deve ser float (ex: 0.80)")?,

            penny_no_max_price: std::env::var("PENNY_NO_MAX_PRICE")
                .unwrap_or_else(|_| "0.15".into())
                .parse::<f64>()
                .context("PENNY_NO_MAX_PRICE deve ser float (ex: 0.15)")?,

            cross_market_bins_radius: std::env::var("CROSS_MARKET_BINS_RADIUS")
                .unwrap_or_else(|_| "2".into())
                .parse::<usize>()
                .context("CROSS_MARKET_BINS_RADIUS deve ser inteiro positivo")?,
        })
    }

    /// Live trading ativo apenas quando DRY_RUN=false E ALLOW_LIVE_TRADING=true.
    pub fn live_trading_enabled(&self) -> bool {
        !self.dry_run && self.allow_live_trading
    }

    /// Variante para bins de análise/paper trading que não precisam da chave privada.
    pub fn from_env_without_private_key() -> Result<Self> {
        dotenvy::dotenv().ok();

        Ok(Config {
            private_key: std::env::var("POLYMARKET_PRIVATE_KEY").unwrap_or_default(),
            min_confidence: std::env::var("MIN_CONFIDENCE")
                .unwrap_or_else(|_| "0.72".into())
                .parse::<f64>()
                .context("MIN_CONFIDENCE deve ser float (ex: 0.72)")?,
            max_position_size_usdc: Decimal::from_str(
                &std::env::var("MAX_POSITION_SIZE_USDC").unwrap_or_else(|_| "10.0".into()),
            )
            .context("MAX_POSITION_SIZE_USDC inválido")?,
            min_order_size_usdc: Decimal::from_str(
                &std::env::var("MIN_ORDER_SIZE_USDC").unwrap_or_else(|_| "1.0".into()),
            )
            .context("MIN_ORDER_SIZE_USDC inválido")?,
            run_interval_secs: std::env::var("RUN_INTERVAL_SECS")
                .unwrap_or_else(|_| "3600".into())
                .parse::<u64>()
                .context("RUN_INTERVAL_SECS deve ser inteiro positivo")?,
            dry_run: std::env::var("DRY_RUN")
                .unwrap_or_else(|_| "true".into())
                .parse::<bool>()
                .context("DRY_RUN deve ser true ou false")?,
            allow_live_trading: std::env::var("ALLOW_LIVE_TRADING")
                .unwrap_or_else(|_| "false".into())
                .parse::<bool>()
                .context("ALLOW_LIVE_TRADING deve ser true ou false")?,
            penny_shares: std::env::var("PENNY_SHARES")
                .unwrap_or_else(|_| "0".into())
                .parse::<u32>()
                .context("PENNY_SHARES deve ser inteiro não-negativo")?,
            penny_max_price: std::env::var("PENNY_MAX_PRICE")
                .unwrap_or_else(|_| "0.02".into())
                .parse::<f64>()
                .context("PENNY_MAX_PRICE deve ser float (ex: 0.02)")?,
            penny_min_confidence: std::env::var("PENNY_MIN_CONFIDENCE")
                .unwrap_or_else(|_| "0.40".into())
                .parse::<f64>()
                .context("PENNY_MIN_CONFIDENCE deve ser float (ex: 0.40)")?,
            extended_horizon_days: std::env::var("EXTENDED_HORIZON_DAYS")
                .unwrap_or_else(|_| "3".into())
                .parse::<i64>()
                .context("EXTENDED_HORIZON_DAYS deve ser inteiro positivo")?,
            change_poll_secs: std::env::var("CHANGE_POLL_SECS")
                .unwrap_or_else(|_| "300".into())
                .parse::<u64>()
                .context("CHANGE_POLL_SECS deve ser inteiro positivo (segundos)")?,
            temp_change_threshold: std::env::var("TEMP_CHANGE_THRESHOLD")
                .unwrap_or_else(|_| "0.3".into())
                .parse::<f64>()
                .context("TEMP_CHANGE_THRESHOLD deve ser float (ex: 0.3)")?,
            price_change_threshold: std::env::var("PRICE_CHANGE_THRESHOLD")
                .unwrap_or_else(|_| "0.03".into())
                .parse::<f64>()
                .context("PRICE_CHANGE_THRESHOLD deve ser float (ex: 0.03)")?,
            use_intraday_trend: std::env::var("USE_INTRADAY_TREND")
                .unwrap_or_else(|_| "true".into())
                .parse::<bool>()
                .context("USE_INTRADAY_TREND deve ser true ou false")?,
            anticipatory_min_confidence: std::env::var("ANTICIPATORY_MIN_CONFIDENCE")
                .unwrap_or_else(|_| "0.88".into())
                .parse::<f64>()
                .context("ANTICIPATORY_MIN_CONFIDENCE deve ser float (ex: 0.88)")?,
            discovery_refresh_secs: std::env::var("DISCOVERY_REFRESH_SECS")
                .unwrap_or_else(|_| "300".into())
                .parse::<u64>()
                .context("DISCOVERY_REFRESH_SECS deve ser inteiro positivo")?,
            resolution_poll_secs: std::env::var("RESOLUTION_POLL_SECS")
                .unwrap_or_else(|_| "180".into())
                .parse::<u64>()
                .context("RESOLUTION_POLL_SECS deve ser inteiro positivo")?,
            weather_poll_d3_secs: std::env::var("WEATHER_POLL_D3_SECS")
                .unwrap_or_else(|_| "3600".into())
                .parse::<u64>()
                .context("WEATHER_POLL_D3_SECS deve ser inteiro positivo")?,
            weather_poll_d2_secs: std::env::var("WEATHER_POLL_D2_SECS")
                .unwrap_or_else(|_| "1800".into())
                .parse::<u64>()
                .context("WEATHER_POLL_D2_SECS deve ser inteiro positivo")?,
            weather_poll_d1_secs: std::env::var("WEATHER_POLL_D1_SECS")
                .unwrap_or_else(|_| "900".into())
                .parse::<u64>()
                .context("WEATHER_POLL_D1_SECS deve ser inteiro positivo")?,
            weather_intraday_poll_secs: std::env::var("WEATHER_INTRADAY_POLL_SECS")
                .unwrap_or_else(|_| "180".into())
                .parse::<u64>()
                .context("WEATHER_INTRADAY_POLL_SECS deve ser inteiro positivo")?,
            edge_min: std::env::var("EDGE_MIN")
                .unwrap_or_else(|_| "0.02".into())
                .parse::<f64>()
                .context("EDGE_MIN deve ser float (ex: 0.02)")?,
            max_spread_cents: std::env::var("MAX_SPREAD_CENTS")
                .unwrap_or_else(|_| "5.0".into())
                .parse::<f64>()
                .context("MAX_SPREAD_CENTS deve ser float (ex: 5.0)")?,
            forecast_change_trigger_degrees: std::env::var("FORECAST_CHANGE_TRIGGER_DEGREES")
                .unwrap_or_else(|_| "0.4".into())
                .parse::<f64>()
                .context("FORECAST_CHANGE_TRIGGER_DEGREES deve ser float (ex: 0.4)")?,
            implied_move_trigger_cents: std::env::var("IMPLIED_MOVE_TRIGGER_CENTS")
                .unwrap_or_else(|_| "2.0".into())
                .parse::<f64>()
                .context("IMPLIED_MOVE_TRIGGER_CENTS deve ser float (ex: 2.0)")?,
            num_sources_required: std::env::var("NUM_SOURCES_REQUIRED")
                .unwrap_or_else(|_| "3".into())
                .parse::<usize>()
                .context("NUM_SOURCES_REQUIRED deve ser inteiro positivo")?,
            source_agreement_threshold: std::env::var("SOURCE_AGREEMENT_THRESHOLD")
                .unwrap_or_else(|_| "1.5".into())
                .parse::<f64>()
                .context("SOURCE_AGREEMENT_THRESHOLD deve ser float em graus (ex: 1.5)")?,
            consensus_min_confidence: std::env::var("CONSENSUS_MIN_CONFIDENCE")
                .unwrap_or_else(|_| "0.80".into())
                .parse::<f64>()
                .context("CONSENSUS_MIN_CONFIDENCE deve ser float (ex: 0.80)")?,
            penny_no_max_price: std::env::var("PENNY_NO_MAX_PRICE")
                .unwrap_or_else(|_| "0.15".into())
                .parse::<f64>()
                .context("PENNY_NO_MAX_PRICE deve ser float (ex: 0.15)")?,
            cross_market_bins_radius: std::env::var("CROSS_MARKET_BINS_RADIUS")
                .unwrap_or_else(|_| "2".into())
                .parse::<usize>()
                .context("CROSS_MARKET_BINS_RADIUS deve ser inteiro positivo")?,
        })
    }

    pub fn with_runtime_overrides(
        mut self,
        min_confidence: Option<f64>,
        max_position_size_usdc: Option<Decimal>,
        run_interval_secs: Option<u64>,
    ) -> Self {
        if let Some(v) = min_confidence {
            self.min_confidence = v;
        }
        if let Some(v) = max_position_size_usdc {
            self.max_position_size_usdc = v;
        }
        if let Some(v) = run_interval_secs {
            self.run_interval_secs = v;
        }
        self
    }
}
