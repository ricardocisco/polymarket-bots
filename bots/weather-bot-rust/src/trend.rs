// src/trend.rs
//! Análise de tendência intradiária de temperatura.
//!
//! Recebe snapshots horários reais (horas já decorridas do dia) e produz:
//!   • Máxima observada até agora (observações diretas)
//!   • Hora local em que ocorreu o pico
//!   • Tendência das últimas 3h (slope °C/h)
//!   • Projeção da máxima final do dia
//!   • Confiança na projeção (cresce ao longo do dia)
//!
//! CASO DE USO CHAVE — "trade antecipado":
//!   Mercado pergunta: "Máxima do dia ≥ 40°C?"
//!   São 16h local, máx observada = 38.2°C, slope = -0.8°C/h
//!   → Pico ocorreu há 2h, temperatura em queda → 40°C é inatingível
//!   → evaluate_with_trend compra NO com confiança > 92% ANTES do mercado fechar

use crate::types::{HourlySnapshot, TrendAnalysis};

/// Produz análise completa de tendência a partir de snapshots horários.
///
/// `station_lon` é usado para estimar o fuso-horário local da estação
/// (aproximação: 15° de longitude = 1h de diferença).
pub fn analyze_trend(hourly: &[HourlySnapshot], station_lon: f64) -> TrendAnalysis {
    if hourly.is_empty() {
        return TrendAnalysis {
            observed_max: f64::NEG_INFINITY,
            projected_max: f64::NEG_INFINITY,
            slope_3h: 0.0,
            data_hour_utc: 0,
            local_hour: 0,
            projection_confidence: 0.0,
            hours_since_peak: None,
        };
    }

    let last = hourly.last().unwrap();
    let data_hour_utc = last.hour_utc;

    // Estimativa de hora local baseada na longitude
    let tz_offset = estimate_tz_offset(station_lon);
    let local_hour = ((data_hour_utc as i16 + tz_offset as i16).rem_euclid(24)) as u8;

    // Máxima observada
    let observed_max = hourly
        .iter()
        .map(|s| s.temp)
        .fold(f64::NEG_INFINITY, f64::max);

    // Índice e hora do pico
    let peak_idx = hourly
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            a.temp
                .partial_cmp(&b.temp)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
        .unwrap_or(0);

    // Quantas leituras horárias se passaram desde o pico
    let hours_since_peak = if peak_idx < hourly.len().saturating_sub(1) {
        Some((hourly.len() - 1 - peak_idx) as u8)
    } else {
        None
    };

    // Slope das últimas ~3h
    let slope_3h = compute_slope_3h(hourly);

    // Projeção da máxima do fim do dia
    let projected_max =
        project_daily_max(hourly, observed_max, local_hour, slope_3h, hours_since_peak);

    // Confiança na projeção
    let projection_confidence = compute_confidence(local_hour, slope_3h, hours_since_peak);

    TrendAnalysis {
        observed_max,
        projected_max,
        slope_3h,
        data_hour_utc,
        local_hour,
        projection_confidence,
        hours_since_peak,
    }
}

/// Calcula o slope médio das últimas ~3 horas (°C/hora).
/// Positivo = temperatura subindo; negativo = caindo.
fn compute_slope_3h(hourly: &[HourlySnapshot]) -> f64 {
    let n = hourly.len();
    if n < 2 {
        return 0.0;
    }
    // Janela de até 4 pontos (da mais recente para mais antiga)
    let window: Vec<&HourlySnapshot> = hourly.iter().rev().take(4).collect();
    if window.len() < 2 {
        return 0.0;
    }
    // ΔT dividido pela janela em horas
    let delta_temp = window[0].temp - window[window.len() - 1].temp;
    let delta_hours = (window.len() - 1) as f64;
    delta_temp / delta_hours
}

/// Projeta a máxima final do dia.
///
/// Se o pico já foi confirmado (temperatura em queda há ≥ 2h), a máxima está
/// consolidada e a projeção simplesmente retorna o máximo observado.
///
/// Caso contrário, extrapola usando slope e horário típico de pico (~14h local).
fn project_daily_max(
    hourly: &[HourlySnapshot],
    observed_max: f64,
    local_hour: u8,
    slope_3h: f64,
    hours_since_peak: Option<u8>,
) -> f64 {
    // Pico confirmado: temperatura em queda por 2+ horas consecutivas
    if hours_since_peak.map(|h| h >= 2).unwrap_or(false) && slope_3h < 0.0 {
        return observed_max;
    }

    // Antes das 10h local: muito cedo para projetar, retorna observado
    if local_hour < 10 {
        return observed_max;
    }

    // Entre 10h e 16h: pico típico está a caminho (~14h local)
    let typical_peak_local = 14u8;
    let hours_to_peak = if local_hour < typical_peak_local {
        (typical_peak_local - local_hour) as f64
    } else {
        0.0
    };

    let current_temp = hourly.last().map(|s| s.temp).unwrap_or(observed_max);
    // Extrapola apenas com slope positivo (não projeta queda além do observado)
    let extrapolated = current_temp + slope_3h.max(0.0) * hours_to_peak;

    // Projeção nunca pode ser menor que o já observado
    extrapolated.max(observed_max)
}

/// Confiança na projeção intradiária.
///
/// Quanto mais tarde no dia, mais consolidada está a máxima — confiança sobe.
/// Temperatura ainda em subida rápida → penalidade (incerteza maior).
/// Pico claramente passado (queda estável) → bônus.
fn compute_confidence(local_hour: u8, slope_3h: f64, hours_since_peak: Option<u8>) -> f64 {
    let base: f64 = match local_hour {
        0..=8 => 0.25,   // madrugada/manhã: temperatura ainda pode subir muito
        9..=11 => 0.45,  // manhã: incerteza moderada sobre o pico
        12..=13 => 0.60, // ao redor do pico típico
        14..=16 => 0.80, // tarde: pico provavelmente passou
        17..=20 => 0.92, // noite: máxima do dia praticamente confirmada
        _ => 0.96,       // madrugada do dia seguinte: definitivamente encerrado
    };

    // Bônus quando pico claramente passou e temperatura em queda
    let peak_bonus: f64 = if hours_since_peak.map(|h| h >= 2).unwrap_or(false) && slope_3h < -0.3 {
        0.08
    } else {
        0.0
    };

    // Penalidade quando temperatura ainda sobe rapidamente
    let rise_penalty: f64 = if slope_3h > 1.5 {
        -0.15
    } else if slope_3h > 0.8 {
        -0.07
    } else {
        0.0
    };

    (base + peak_bonus + rise_penalty).clamp(0.10_f64, 0.98_f64)
}

/// Estima o offset de fuso-horário UTC a partir da longitude.
/// Aproximação: 15° de longitude = 1h. Imprecisa mas suficiente para detectar
/// hora local do dia e avaliar se o pico diário já ocorreu.
pub fn estimate_tz_offset(lon: f64) -> i8 {
    (lon / 15.0).round().clamp(-12.0, 14.0) as i8
}
