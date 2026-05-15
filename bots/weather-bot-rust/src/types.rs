// src/types.rs
//! Tipos compartilhados entre bot real, setup e backtest.

use crate::markets::TempUnit;

// ── Multi-source weather types ─────────────────────────────────────────────

/// Identifica a fonte de uma previsão de temperatura.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeatherSource {
    /// Open-Meteo com modelo GFS (Global Forecast System — NOAA)
    OpenMeteoGfs,
    /// Open-Meteo com modelo ICON (Deutschland Wetterdienst — DWD)
    OpenMeteoIcon,
    /// Open-Meteo com modelo ECMWF IFS (European Centre for Medium-Range)
    OpenMeteoEcmwf,
    /// Open-Meteo Ensemble (média de modelos, fornece spread de incerteza)
    OpenMeteoEnsemble,
    /// Aviation Weather TAF — previsão oficial de aeroporto (aviationweather.gov)
    AviationWeatherTaf,
    /// Wunderground forecast — scraping da página de previsão (mesma fonte da resolução)
    WundergroundForecast,
}

impl WeatherSource {
    pub fn name(self) -> &'static str {
        match self {
            WeatherSource::OpenMeteoGfs => "Open-Meteo/GFS",
            WeatherSource::OpenMeteoIcon => "Open-Meteo/ICON",
            WeatherSource::OpenMeteoEcmwf => "Open-Meteo/ECMWF",
            WeatherSource::OpenMeteoEnsemble => "Open-Meteo/Ensemble",
            WeatherSource::AviationWeatherTaf => "AviationWeather/TAF",
            WeatherSource::WundergroundForecast => "Wunderground/Forecast",
        }
    }

    /// Peso de confiabilidade histórica da fonte (0.0–1.0).
    /// Usado para calcular a média ponderada no consensus.
    pub fn reliability_weight(self) -> f64 {
        match self {
            // ECMWF é historicamente o modelo mais preciso para temperatura
            WeatherSource::OpenMeteoEcmwf => 1.20,
            // TAF oficial do aeroporto = mesma estação da resolução
            WeatherSource::AviationWeatherTaf => 1.15,
            // Wunderground = mesma fonte usada para resolucao
            WeatherSource::WundergroundForecast => 1.10,
            // GFS é o modelo global padrão da NOAA
            WeatherSource::OpenMeteoGfs => 1.00,
            // ICON é bom para Europa, médio para resto do mundo
            WeatherSource::OpenMeteoIcon => 0.95,
            // Ensemble é uma média, útil para estimar spread
            WeatherSource::OpenMeteoEnsemble => 0.90,
        }
    }
}

/// Previsão de temperatura de uma fonte específica.
#[derive(Debug, Clone)]
pub struct SourceForecast {
    pub source: WeatherSource,
    /// Temperatura máxima prevista (na unidade do mercado)
    pub predicted_max: f64,
    /// Incerteza intrínseca estimada da fonte (±°C ou ±°F)
    pub uncertainty: f64,
}

/// Resultado do consensus multi-fonte para um ICAO+data.
#[derive(Debug, Clone)]
pub struct ConsensusResult {
    /// Temperatura máxima prevista pelo consensus (média ponderada)
    pub predicted_temp: f64,
    /// Confiança geral do consensus (0.0–1.0)
    pub confidence: f64,
    /// Spread entre a fonte mais quente e a mais fria (em graus)
    pub spread: f64,
    /// Número de fontes que contribuíram para este consensus
    pub sources_count: usize,
    /// Fontes individuais
    pub sources: Vec<SourceForecast>,
}

impl ConsensusResult {
    /// Retorna `true` se o consensus tem dados suficientes para ser confiável.
    pub fn is_reliable(&self, min_sources: usize, max_spread: f64) -> bool {
        self.sources_count >= min_sources && self.spread <= max_spread
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Yes,
    No,
}

/// Previsão de temperatura máxima para um ICAO num dado dia.
/// Usada tanto pelo bot (previsão futura) quanto pelo backtest (dado real histórico).
#[derive(Debug, Clone)]
pub struct Forecast {
    /// Código ICAO da estação (ex: "SBGR", "KMIA", "EGLC")
    pub icao: String,
    /// Temperatura máxima prevista/registrada
    pub max_temp: f64,
    /// Unidade de temperatura (Celsius ou Fahrenheit)
    pub unit: TempUnit,
    /// Confiança estimada (0.0–1.0)
    /// - Bot real: calculada pela estabilidade dos 3 dias de previsão
    /// - Backtest: 1.0 (dado real histórico confirmado)
    pub confidence: f64,
}

/// Snapshot de temperatura de uma hora específica (dados intradiários do Open-Meteo).
#[derive(Debug, Clone)]
pub struct HourlySnapshot {
    /// Hora UTC (0–23)
    pub hour_utc: u8,
    /// Temperatura registrada nessa hora (já com correção de viés)
    pub temp: f64,
}

/// Análise de tendência intradiária de temperatura para um mercado.
///
/// Produzida a partir de dados horários reais (horas já decorridas do dia),
/// complementando a previsão de modelo com observações diretas.
///
/// Lógica central:
///   - Se o pico diário já ocorreu (temp em queda há ≥ 2h), a máxima está consolidada.
///   - Quanto mais tarde no dia, mais alta é `projection_confidence`.
///   - Se `observed_max` + `trend` indicam que o limiar do mercado não será atingido,
///     o bot pode comprar antecipadamente com alta confiança.
#[derive(Debug, Clone)]
pub struct TrendAnalysis {
    /// Máxima temperatura real observada nas horas já decorridas do dia
    pub observed_max: f64,
    /// Projeção da máxima total do dia (baseada em observações + hora atual)
    pub projected_max: f64,
    /// Tendência de temperatura nas últimas ~3 horas (°C/h; positivo = subindo)
    pub slope_3h: f64,
    /// Hora UTC do último dado disponível
    pub data_hour_utc: u8,
    /// Hora local estimada (baseada na longitude da estação)
    pub local_hour: u8,
    /// Confiança na projeção (0.0–1.0); cresce conforme o dia avança
    pub projection_confidence: f64,
    /// Quantas horas atrás ocorreu o pico do dia (None = pico ainda não confirmado)
    pub hours_since_peak: Option<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct QuoteSnapshot {
    pub yes_bid: Option<f64>,
    pub yes_ask: Option<f64>,
    pub no_bid: Option<f64>,
    pub no_ask: Option<f64>,
    pub ts: i64,
}

impl QuoteSnapshot {
    #[must_use]
    pub fn best_buy_price(&self, side: Side) -> Option<f64> {
        match side {
            Side::Yes => best_min(self.yes_ask, complement(self.no_bid)),
            Side::No => best_min(self.no_ask, complement(self.yes_bid)),
        }
    }

    #[must_use]
    pub fn best_sell_price(&self, side: Side) -> Option<f64> {
        match side {
            Side::Yes => best_max(self.yes_bid, complement(self.no_ask)),
            Side::No => best_max(self.no_bid, complement(self.yes_ask)),
        }
    }

    #[must_use]
    pub fn spread(&self, side: Side) -> Option<f64> {
        Some(self.best_sell_price(side)? - self.best_buy_price(side)?)
            .filter(|spread| spread.is_finite() && *spread >= 0.0)
    }
}

fn complement(price: Option<f64>) -> Option<f64> {
    price
        .map(|price| 1.0 - price)
        .filter(|price| price.is_finite() && (0.0..=1.0).contains(price))
}

fn best_min(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn best_max(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}
