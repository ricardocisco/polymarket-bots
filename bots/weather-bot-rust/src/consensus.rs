// src/consensus.rs
//! Motor de consensus multi-fonte para previsão de temperatura.
//!
//! Agrega previsões de múltiplas fontes meteorológicas, calcula médias ponderadas
//! pela confiabilidade de cada fonte e produz um `ConsensusResult` com:
//!   - Temperatura prevista pelo consensus
//!   - Confiança calibrada (base ± penalidade de spread ± bônus de acordo)
//!   - Spread entre fontes (indicador de incerteza inter-modelo)
//!
//! ## Fórmula de confiança
//! ```text
//! conf = horizon_base × (1 - spread_penalty) × agreement_bonus
//!
//! horizon_base: D+0=0.82, D+1=0.74, D+2=0.66, D+3+=0.58
//! spread_penalty = clamp(spread_celsius / 3.0, 0.0, 0.30)
//! agreement_bonus = +0.10 se ≥ 4 fontes concordam dentro de 1°C
//! ```

use tracing::{debug, info};

use crate::types::{ConsensusResult, SourceForecast};

/// Calcula o consensus a partir de uma lista de previsões de fontes distintas.
///
/// # Parâmetros
/// - `sources`: previsões coletadas (de `WeatherClient::fetch_all_sources`)
/// - `days_ahead`: horizonte de previsão em dias (0 = hoje, 1 = amanhã, ...)
pub fn compute_consensus(sources: &[SourceForecast], days_ahead: usize) -> ConsensusResult {
    if sources.is_empty() {
        return ConsensusResult {
            predicted_temp: f64::NAN,
            confidence: 0.0,
            spread: 0.0,
            uncertainty: f64::INFINITY,
            sources_count: 0,
            sources: Vec::new(),
        };
    }

    // ── 1. Média ponderada por confiabilidade da fonte ────────────────────
    let total_weight: f64 = sources.iter().map(|s| s.source.reliability_weight()).sum();
    let weighted_mean: f64 = sources
        .iter()
        .map(|s| s.predicted_max * s.source.reliability_weight())
        .sum::<f64>()
        / total_weight;

    // ── 2. Spread (max - min entre as fontes) ─────────────────────────────
    let max_temp = sources
        .iter()
        .map(|s| s.predicted_max)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_temp = sources
        .iter()
        .map(|s| s.predicted_max)
        .fold(f64::INFINITY, f64::min);
    let spread = (max_temp - min_temp).max(0.0);

    // ── 3. Confiança base pelo horizonte ──────────────────────────────────
    let predictive_variance = sources
        .iter()
        .map(|source| {
            let weight = source.source.reliability_weight();
            weight
                * ((source.predicted_max - weighted_mean).powi(2)
                    + source.uncertainty.max(0.1).powi(2))
        })
        .sum::<f64>()
        / total_weight;
    let uncertainty = predictive_variance.sqrt().max(0.25);

    // ── 4. Penalidade por spread alto entre fontes ────────────────────────
    // Spread de 3°C → penalidade máxima de -30%
    let spread_penalty = (spread / 3.0).clamp(0.0, 0.50);

    // ── 5. Bônus por acordo entre fontes ──────────────────────────────────
    // Considera "em acordo" se a previsão está dentro de 1°C da média
    let agreement_radius = uncertainty;
    let agreeing_sources = sources
        .iter()
        .filter(|s| (s.predicted_max - weighted_mean).abs() <= agreement_radius)
        .count();

    // Bônus escalonado: +5% por 3 fontes concordando, +10% por 4+
    let agreement_bonus: f64 = if agreeing_sources >= 3 { 1.0 } else { 0.90 };

    // ── 6. Confiança final ────────────────────────────────────────────────
    let horizon_penalty = 1.0 + days_ahead.saturating_sub(1) as f64 * 0.10;
    let confidence = ((1.0 / (1.0 + uncertainty * horizon_penalty / 2.0))
        * (1.0 - spread_penalty)
        * agreement_bonus)
        .clamp(0.10, 0.90);

    info!(
        "[consensus] D+{} | Prev={:.1} | Spread={:.1} | Fontes={} ({} concordam) | Conf={:.1}%",
        days_ahead,
        weighted_mean,
        spread,
        sources.len(),
        agreeing_sources,
        confidence * 100.0,
    );

    debug!(
        "[consensus] fontes: {}",
        sources
            .iter()
            .map(|s| format!("{}: {:.1}", s.source.name(), s.predicted_max))
            .collect::<Vec<_>>()
            .join(", ")
    );

    ConsensusResult {
        predicted_temp: weighted_mean,
        confidence,
        spread,
        uncertainty,
        sources_count: sources.len(),
        sources: sources.to_vec(),
    }
}

/// Converte um `ConsensusResult` em um `Forecast` legado para compatibilidade
/// com o pipeline existente de estratégia/trading.
pub fn consensus_to_forecast(
    consensus: &ConsensusResult,
    icao: &str,
    unit: crate::markets::TempUnit,
) -> crate::types::Forecast {
    crate::types::Forecast {
        icao: icao.to_string(),
        max_temp: consensus.predicted_temp,
        unit,
        confidence: consensus.confidence,
        uncertainty: consensus.uncertainty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WeatherSource;

    fn make_source(source: WeatherSource, temp: f64) -> SourceForecast {
        SourceForecast {
            source,
            predicted_max: temp,
            uncertainty: 1.0,
        }
    }

    #[test]
    fn empty_sources_returns_zero_confidence() {
        let result = compute_consensus(&[], 1);
        assert_eq!(result.confidence, 0.0);
        assert_eq!(result.sources_count, 0);
    }

    #[test]
    fn consensus_with_tight_spread_has_bounded_uncertainty() {
        // 4 fontes concordando em ~30°C → alta confiança
        let sources = vec![
            make_source(WeatherSource::OpenMeteoGfs, 30.0),
            make_source(WeatherSource::OpenMeteoIcon, 30.3),
            make_source(WeatherSource::OpenMeteoEcmwf, 29.8),
            make_source(WeatherSource::AviationWeatherTaf, 30.1),
        ];
        let result = compute_consensus(&sources, 1);
        assert!(result.confidence > 0.50, "conf={}", result.confidence);
        assert!(
            (0.5..=1.5).contains(&result.uncertainty),
            "sigma={}",
            result.uncertainty
        );
        assert!(result.spread < 1.0, "spread={}", result.spread);
        assert!((result.predicted_temp - 30.0).abs() < 0.5);
    }

    #[test]
    fn high_spread_reduces_confidence() {
        // 3 fontes discordando muito → confiança reduzida
        let sources = vec![
            make_source(WeatherSource::OpenMeteoGfs, 28.0),
            make_source(WeatherSource::OpenMeteoIcon, 32.0),
            make_source(WeatherSource::OpenMeteoEcmwf, 35.0),
        ];
        let result = compute_consensus(&sources, 1);
        assert!(result.confidence < 0.65, "conf={}", result.confidence);
        assert!(result.spread > 3.0, "spread={}", result.spread);
    }

    #[test]
    fn ecmwf_has_higher_weight_than_gfs() {
        // ECMWF pesa mais → consensus puxa para a temperatura do ECMWF
        let sources = vec![
            make_source(WeatherSource::OpenMeteoGfs, 28.0),
            make_source(WeatherSource::OpenMeteoEcmwf, 31.0),
        ];
        let result = compute_consensus(&sources, 1);
        // Com peso maior do ECMWF (1.20 vs 1.00), média ponderada > 29.5
        assert!(
            result.predicted_temp > 29.5,
            "temp={}",
            result.predicted_temp
        );
    }

    #[test]
    fn is_reliable_checks_sources_and_spread() {
        let sources = vec![
            make_source(WeatherSource::OpenMeteoGfs, 30.0),
            make_source(WeatherSource::OpenMeteoIcon, 30.3),
            make_source(WeatherSource::OpenMeteoEcmwf, 29.8),
        ];
        let result = compute_consensus(&sources, 1);
        assert!(result.is_reliable(3, 1.5));
        assert!(!result.is_reliable(4, 1.5)); // menos de 4 fontes
    }
}
