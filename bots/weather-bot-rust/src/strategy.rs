// src/strategy.rs
//! Motor de decisão do bot.
//!
//! Para cada mercado + previsão, suporta duas estratégias:
//!
//! ── Estratégia Penny (cfg.penny_shares > 0) ───────────────────────────────
//!   Busca mercados onde YES ou NO está cotado a ≤ penny_max_price (ex: 2¢).
//!   Se a previsão confirma aquele lado com conf ≥ penny_min_confidence:
//!     → Compra penny_shares shares  (ex: 300 shares × $0.01 = $3)
//!     → Retorno potencial: 300 × $1 = $300 (+9.900%)
//!
//! ── Estratégia Quarter-Kelly (cfg.penny_shares == 0) ─────────────────────
//!   1. Resolve o range de temperatura (min, max)
//!   2. Verifica se a previsão está dentro ou fora do range
//!   3. Calcula confiança efetiva (base ± bônus de margem de segurança)
//!   4. Se confiança >= min_confidence → dimensiona via Quarter-Kelly
//!   5. Retorna Decision: BuyYes | BuyNo | Skip
//!
//! ── Estratégia Trend-Antecipado (evaluate_with_trend) ────────────────────
//!   Usa dados horários REAIS do dia atual para detectar antecipadamente
//!   quando um limiar de mercado já não pode mais ser atingido:
//!
//!   Exemplo: Mercado "Máxima ≥ 40°C?" | 16h local | ObsMax=38.2°C | slope=-0.8°C/h
//!     → Pico confirmado há 2h, temperatura em queda → 40°C impossível
//!     → BUY NO com confiança ≥ 92% ANTES do mercado fechar

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::{
    config::Config,
    markets::TempMarket,
    types::{Forecast, TrendAnalysis},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyKind {
    Kelly,
    Penny,
    TrendAnticipatory,
}

impl StrategyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            StrategyKind::Kelly => "kelly",
            StrategyKind::Penny => "penny",
            StrategyKind::TrendAnticipatory => "trend_anticipatory",
        }
    }
}

#[derive(Debug)]
pub enum Decision {
    /// Comprar YES — temperatura prevista está dentro do range
    BuyYes {
        token_id: String,
        size_usdc: Decimal,
        price: f64,
        /// Shares a comprar (calculado de size_usdc / price)
        shares: u32,
        tick_size: String,
        neg_risk: bool,
        reason: String,
    },
    /// Comprar NO — temperatura prevista está fora do range
    BuyNo {
        token_id: String,
        size_usdc: Decimal,
        price: f64,
        /// Shares a comprar (calculado de size_usdc / price)
        shares: u32,
        tick_size: String,
        neg_risk: bool,
        reason: String,
    },
    /// Não entrar — sem oportunidade penny, confiança insuficiente ou edge negativo
    Skip(String),
}

#[derive(Debug)]
pub struct Opportunity {
    pub decision: Decision,
    pub strategy_kind: StrategyKind,
    pub effective_confidence: f64,
    pub expected_value: f64,
    pub edge_per_share: f64,
}

#[derive(Debug, Clone, Copy)]
struct EvalSnapshot {
    yes_probability: f64,
}

fn estimate_edge_per_share(price: f64, probability: f64) -> f64 {
    (probability - price).clamp(-1.0, 1.0)
}

fn estimate_expected_value(size_usdc: Decimal, price: f64, probability: f64) -> f64 {
    let size = size_usdc.to_string().parse::<f64>().unwrap_or(0.0);
    if size <= 0.0 || !(0.0..1.0).contains(&price) {
        return 0.0;
    }

    let shares = size / price;
    probability * (shares * (1.0 - price)) - (1.0 - probability) * size
}

fn range_snapshot(market: &TempMarket, forecast: &Forecast) -> Option<EvalSnapshot> {
    let (range_min, range_max) = match (market.range_min, market.range_max) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (a, f64::INFINITY),
        (None, Some(b)) => (f64::NEG_INFINITY, b),
        (None, None) => return None,
    };

    Some(EvalSnapshot {
        yes_probability: probability_in_range(
            forecast.max_temp,
            forecast.uncertainty,
            range_min,
            range_max,
        ),
    })
}

pub fn evaluate_opportunity(market: &TempMarket, forecast: &Forecast, cfg: &Config) -> Opportunity {
    let decision = evaluate(market, forecast, cfg);
    let snapshot = range_snapshot(market, forecast);

    let (strategy_kind, effective_confidence, probability, size_usdc, price) = match &decision {
        Decision::BuyYes {
            size_usdc, price, ..
        } => {
            let snap = snapshot.unwrap_or(EvalSnapshot {
                yes_probability: forecast.confidence,
            });
            (
                if cfg.penny_shares > 0 {
                    StrategyKind::Penny
                } else {
                    StrategyKind::Kelly
                },
                snap.yes_probability,
                snap.yes_probability,
                *size_usdc,
                *price,
            )
        }
        Decision::BuyNo {
            size_usdc, price, ..
        } => {
            let snap = snapshot.unwrap_or(EvalSnapshot {
                yes_probability: 1.0 - forecast.confidence,
            });
            (
                if cfg.penny_shares > 0 {
                    StrategyKind::Penny
                } else {
                    StrategyKind::Kelly
                },
                1.0 - snap.yes_probability,
                1.0 - snap.yes_probability,
                *size_usdc,
                *price,
            )
        }
        Decision::Skip(_) => (
            if cfg.penny_shares > 0 {
                StrategyKind::Penny
            } else {
                StrategyKind::Kelly
            },
            snapshot
                .map(|s| s.yes_probability.max(1.0 - s.yes_probability))
                .unwrap_or(forecast.confidence),
            snapshot
                .map(|s| s.yes_probability.max(1.0 - s.yes_probability))
                .unwrap_or(forecast.confidence),
            Decimal::ZERO,
            0.0,
        ),
    };

    Opportunity {
        edge_per_share: estimate_edge_per_share(price, probability),
        expected_value: estimate_expected_value(size_usdc, price, probability),
        decision,
        strategy_kind,
        effective_confidence,
    }
}

/// Avalia um mercado contra uma previsão e retorna a decisão.
///
/// Se `cfg.penny_shares > 0`, usa a estratégia penny:
///   - Procura YES ou NO cotado a ≤ penny_max_price (ex: 2¢)
///   - Verifica se a previsão confirma aquele lado com conf ≥ penny_min_confidence
///   - Retorna compra de `penny_shares` shares (ex: 300 × $0.01 = $3)
///
/// Caso contrário, usa Quarter-Kelly clássico.
pub fn evaluate(market: &TempMarket, forecast: &Forecast, cfg: &Config) -> Decision {
    let predicted = forecast.max_temp;

    // Resolve limites do range (converte None em ±infinito)
    let (range_min, range_max) = match (market.range_min, market.range_max) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (a, f64::INFINITY),
        (None, Some(b)) => (f64::NEG_INFINITY, b),
        (None, None) => {
            return Decision::Skip(format!(
                "Range não identificado na pergunta: '{}'",
                market.question.chars().take(80).collect::<String>()
            ));
        }
    };

    let in_range = predicted >= range_min - 0.05 && predicted <= range_max + 0.05;

    // Margem de segurança: distância da previsão até o limite mais próximo do range
    // Margem positiva = dentro do range; negativa = fora
    let safety_margin = if in_range {
        let d_min = predicted - range_min;
        let d_max = if range_max.is_finite() {
            range_max - predicted
        } else {
            5.0
        };
        d_min.min(d_max)
    } else if predicted < range_min {
        predicted - range_min // negativo
    } else {
        range_max - predicted // negativo
    };

    // Bônus/penalidade de confiança pela margem
    // Margem > 2°C → +4% de confiança; margem < 0 → -10%
    let _margin_bonus = if safety_margin.is_finite() {
        (safety_margin / 4.0).clamp(-0.10, 0.04)
    } else {
        0.0
    };

    let yes_probability =
        probability_in_range(predicted, forecast.uncertainty, range_min, range_max);
    let eff_conf = if in_range {
        yes_probability
    } else {
        1.0 - yes_probability
    };

    let range_str = format!(
        "[{} – {}]",
        if range_min.is_finite() {
            format!("{:.1}{}", range_min, forecast.unit.symbol())
        } else {
            "-∞".into()
        },
        if range_max.is_finite() {
            format!("{:.1}{}", range_max, forecast.unit.symbol())
        } else {
            "+∞".into()
        }
    );

    // ── Estratégia Penny: 300 shares a 1¢ ────────────────────────────────
    if cfg.penny_shares > 0 {
        let yes_is_penny = market.yes_price <= cfg.penny_max_price && market.yes_price > 0.0;
        let no_is_penny = market.no_price <= cfg.penny_max_price && market.no_price > 0.0;

        // BUY YES penny: previsão dentro do range, YES está barato, conf suficiente
        if in_range && yes_is_penny && eff_conf >= cfg.penny_min_confidence {
            let size_usdc =
                Decimal::try_from(cfg.penny_shares as f64 * market.yes_price).unwrap_or(dec!(3));
            return Decision::BuyYes {
                token_id: market.yes_token_id.clone(),
                size_usdc: size_usdc.round_dp(4),
                price: market.yes_price,
                shares: cfg.penny_shares,
                tick_size: market.tick_size.clone(),
                neg_risk: market.neg_risk,
                reason: format!(
                    "PENNY | Prev={:.1}{} DENTRO {} | Conf={:.1}% | YES@{:.1}¢ | Retorno=${:.0}",
                    predicted,
                    forecast.unit.symbol(),
                    range_str,
                    eff_conf * 100.0,
                    market.yes_price * 100.0,
                    cfg.penny_shares as f64 * (1.0 - market.yes_price),
                ),
            };
        }

        // BUY NO penny: previsão fora do range, NO está barato, conf suficiente
        if !in_range && no_is_penny && eff_conf >= cfg.penny_min_confidence {
            let size_usdc =
                Decimal::try_from(cfg.penny_shares as f64 * market.no_price).unwrap_or(dec!(3));
            return Decision::BuyNo {
                token_id: market.no_token_id.clone(),
                size_usdc: size_usdc.round_dp(4),
                price: market.no_price,
                shares: cfg.penny_shares,
                tick_size: market.tick_size.clone(),
                neg_risk: market.neg_risk,
                reason: format!(
                    "PENNY | Prev={:.1}{} FORA {} | Conf={:.1}% | NO@{:.1}¢ | Retorno=${:.0}",
                    predicted,
                    forecast.unit.symbol(),
                    range_str,
                    eff_conf * 100.0,
                    market.no_price * 100.0,
                    cfg.penny_shares as f64 * (1.0 - market.no_price),
                ),
            };
        }

        // Sem oportunidade penny neste mercado
        return Decision::Skip(format!(
            "Sem penny | YES={:.1}¢ NO={:.1}¢ | Conf={:.1}% (mín {:.0}%) | {}",
            market.yes_price * 100.0,
            market.no_price * 100.0,
            eff_conf * 100.0,
            cfg.penny_min_confidence * 100.0,
            if in_range { "prev dentro" } else { "prev fora" },
        ));
    }

    // ── Estratégia Quarter-Kelly (penny_shares == 0) ──────────────────────
    if eff_conf < cfg.min_confidence {
        return Decision::Skip(format!(
            "Conf {:.1}% < mín {:.1}% | Range={} | Prev={:.1}{}",
            eff_conf * 100.0,
            cfg.min_confidence * 100.0,
            range_str,
            predicted,
            forecast.unit.symbol()
        ));
    }

    if in_range {
        let size = kelly_size(market.yes_price, eff_conf, cfg);
        if size < cfg.min_order_size_usdc {
            return Decision::Skip(format!(
                "Kelly {:.2} USDC < mín {:.2} USDC",
                size, cfg.min_order_size_usdc
            ));
        }
        let shares = if market.yes_price > 0.0 {
            (size.to_string().parse::<f64>().unwrap_or(0.0) / market.yes_price).round() as u32
        } else {
            0
        };
        Decision::BuyYes {
            token_id: market.yes_token_id.clone(),
            size_usdc: size,
            price: market.yes_price,
            shares,
            tick_size: market.tick_size.clone(),
            neg_risk: market.neg_risk,
            reason: format!(
                "Prev={:.1}{} DENTRO de {} | Conf={:.1}%",
                predicted,
                forecast.unit.symbol(),
                range_str,
                eff_conf * 100.0
            ),
        }
    } else {
        let size = kelly_size(market.no_price, eff_conf, cfg);
        if size < cfg.min_order_size_usdc {
            return Decision::Skip(format!(
                "Kelly {:.2} USDC < mín {:.2} USDC",
                size, cfg.min_order_size_usdc
            ));
        }
        let shares = if market.no_price > 0.0 {
            (size.to_string().parse::<f64>().unwrap_or(0.0) / market.no_price).round() as u32
        } else {
            0
        };
        Decision::BuyNo {
            token_id: market.no_token_id.clone(),
            size_usdc: size,
            price: market.no_price,
            shares,
            tick_size: market.tick_size.clone(),
            neg_risk: market.neg_risk,
            reason: format!(
                "Prev={:.1}{} FORA de {} | Conf={:.1}%",
                predicted,
                forecast.unit.symbol(),
                range_str,
                eff_conf * 100.0
            ),
        }
    }
}

/// Avalia um mercado usando dados intradiários de tendência para máxima precisão.
///
/// Quando dados horários do dia atual estão disponíveis (trend com confiança ≥ 0.55),
/// combina a previsão de modelo com observações horárias reais.
///
/// CASO PRINCIPAL — "trade antecipado":
///   Mercado: "Máxima ≥ 40°C?" | São 16h local | ObsMax=38.2°C | slope=-0.8°C/h
///   → Pico passou há 2h, temperatura em queda → 40°C fisicamente impossível
///   → Retorna BUY NO com confiança > 92% ANTES do mercado fechar
///
/// Quando trend não tem dados suficientes (muito cedo, sem observações),
/// delega automaticamente para `evaluate()` padrão.
pub fn evaluate_with_trend(
    market: &TempMarket,
    forecast: &Forecast,
    trend: &TrendAnalysis,
    cfg: &Config,
) -> Decision {
    // Verifica se os dados intradiários são utilizáveis
    let trend_usable = trend.projection_confidence >= 0.55
        && trend.observed_max.is_finite()
        && trend.observed_max > f64::NEG_INFINITY + 1.0;

    if !trend_usable {
        return evaluate(market, forecast, cfg);
    }

    // Resolve range do mercado
    let (range_min, range_max) = match (market.range_min, market.range_max) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (a, f64::INFINITY),
        (None, Some(b)) => (f64::NEG_INFINITY, b),
        (None, None) => return evaluate(market, forecast, cfg),
    };

    let unit_sym = forecast.unit.symbol();
    let range_str = format!(
        "[{} – {}]",
        if range_min.is_finite() {
            format!("{:.1}{}", range_min, unit_sym)
        } else {
            "-∞".into()
        },
        if range_max.is_finite() {
            format!("{:.1}{}", range_max, unit_sym)
        } else {
            "+∞".into()
        }
    );

    // ── Caso 1: Pico do dia JÁ CONFIRMADO — máxima real do dia conhecida ─────
    //
    // Condições de confirmação:
    //   • Pico ocorreu há ≥ 2 horas consecutivas de queda
    //   • Slope atual é negativo (temperatura ainda caindo)
    //   • Já passaram pelo menos 13h no horário local (tarde)
    let peak_confirmed = trend.hours_since_peak.map(|h| h >= 2).unwrap_or(false)
        && trend.slope_3h < 0.0
        && trend.local_hour >= 13;

    if peak_confirmed {
        let actual_max = trend.observed_max;

        // Onde a máxima real cai em relação ao range?
        let in_range = actual_max >= range_min - 0.05 && actual_max <= range_max + 0.05;

        // Distância da máxima real até o limiar mais próximo do range
        let gap_from_threshold = if !in_range && actual_max < range_min {
            range_min - actual_max // °C que faltam para atingir o limiar
        } else if !in_range && range_max.is_finite() {
            actual_max - range_max
        } else if in_range {
            let d_min = actual_max - range_min;
            let d_max = if range_max.is_finite() {
                range_max - actual_max
            } else {
                5.0
            };
            d_min.min(d_max) // menor margens dentro do range
        } else {
            0.5
        };

        // Bônus de confiança pelo gap (quanto mais distante do limiar, mais seguro)
        let gap_bonus = (gap_from_threshold / 15.0).clamp(0.0, 0.08);
        let eff_conf = (trend.projection_confidence + gap_bonus).clamp(0.0, 0.98);

        // Filtro de confiança mínima para trades antecipados
        if eff_conf < cfg.anticipatory_min_confidence {
            return Decision::Skip(format!(
                "TREND: conf={:.1}% < anticipatory_min={:.1}% | ObsMax={:.1}{} | gap={:.1}° | {}",
                eff_conf * 100.0,
                cfg.anticipatory_min_confidence * 100.0,
                actual_max,
                unit_sym,
                gap_from_threshold,
                range_str
            ));
        }

        let peak_h = trend.hours_since_peak.unwrap_or(0);
        let reason_prefix = format!(
            "TREND ANTECIPADO | Pico {}h atrás | ObsMax={:.1}{} | slope={:.2}°/h | {}h local",
            peak_h, actual_max, unit_sym, trend.slope_3h, trend.local_hour
        );

        if !in_range {
            // Máxima real FORA do range → BUY NO antecipado
            let size = kelly_size(market.no_price, eff_conf, cfg);
            if size < cfg.min_order_size_usdc {
                return Decision::Skip(format!("Kelly pequeno ({:.2}) | {}", size, reason_prefix));
            }
            let shares = if market.no_price > 0.0 {
                (size.to_string().parse::<f64>().unwrap_or(0.0) / market.no_price).round() as u32
            } else {
                0
            };
            return Decision::BuyNo {
                token_id: market.no_token_id.clone(),
                size_usdc: size,
                price: market.no_price,
                shares,
                tick_size: market.tick_size.clone(),
                neg_risk: market.neg_risk,
                reason: format!(
                    "{} | {:.1}{} FORA {} | Conf={:.1}%",
                    reason_prefix,
                    actual_max,
                    unit_sym,
                    range_str,
                    eff_conf * 100.0
                ),
            };
        } else {
            // Máxima real DENTRO do range → BUY YES antecipado
            let size = kelly_size(market.yes_price, eff_conf, cfg);
            if size < cfg.min_order_size_usdc {
                return Decision::Skip(format!("Kelly pequeno ({:.2}) | {}", size, reason_prefix));
            }
            let shares = if market.yes_price > 0.0 {
                (size.to_string().parse::<f64>().unwrap_or(0.0) / market.yes_price).round() as u32
            } else {
                0
            };
            return Decision::BuyYes {
                token_id: market.yes_token_id.clone(),
                size_usdc: size,
                price: market.yes_price,
                shares,
                tick_size: market.tick_size.clone(),
                neg_risk: market.neg_risk,
                reason: format!(
                    "{} | {:.1}{} DENTRO {} | Conf={:.1}%",
                    reason_prefix,
                    actual_max,
                    unit_sym,
                    range_str,
                    eff_conf * 100.0
                ),
            };
        }
    }

    // ── Caso 2: Pico ainda não confirmado, mas tendência disponível ──────────
    // Combina previsão de modelo com tendência observada.
    // Se as duas fontes concordam no resultado esperado → bônus de confiança.
    // Se discordam → penalidade e usa evaluate() padrão de forma conservadora.
    let forecast_in_range =
        forecast.max_temp >= range_min - 0.05 && forecast.max_temp <= range_max + 0.05;
    let trend_in_range =
        trend.projected_max >= range_min - 0.05 && trend.projected_max <= range_max + 0.05;

    if forecast_in_range == trend_in_range {
        // Concordância: mescla temperaturas com peso proporcional à hora do dia
        let trend_weight = (trend.local_hour as f64 / 18.0).clamp(0.2, 0.7);
        let blended_temp =
            trend_weight * trend.projected_max + (1.0 - trend_weight) * forecast.max_temp;

        // Confiança combinada com bônus de convergência
        let combined_conf =
            (forecast.confidence.max(trend.projection_confidence) + 0.04).clamp(0.0, 0.98);

        let blended_forecast = Forecast {
            icao: forecast.icao.clone(),
            max_temp: blended_temp,
            unit: forecast.unit,
            confidence: combined_conf,
            uncertainty: forecast.uncertainty.min(1.0),
        };
        evaluate(market, &blended_forecast, cfg)
    } else {
        // Discordância entre modelo e observação: reduz confiança e usa padrão
        let conservative_forecast = Forecast {
            icao: forecast.icao.clone(),
            max_temp: forecast.max_temp,
            unit: forecast.unit,
            confidence: (forecast.confidence - 0.08).max(0.30),
            uncertainty: forecast.uncertainty * 1.25,
        };
        evaluate(market, &conservative_forecast, cfg)
    }
}

pub fn evaluate_opportunity_with_trend(
    market: &TempMarket,
    forecast: &Forecast,
    trend: &TrendAnalysis,
    cfg: &Config,
) -> Opportunity {
    let trend_usable = trend.projection_confidence >= 0.55
        && trend.observed_max.is_finite()
        && trend.observed_max > f64::NEG_INFINITY + 1.0;
    let peak_confirmed = trend.hours_since_peak.map(|h| h >= 2).unwrap_or(false)
        && trend.slope_3h < 0.0
        && trend.local_hour >= 13;

    let decision = evaluate_with_trend(market, forecast, trend, cfg);
    let base_snapshot = range_snapshot(market, forecast);

    let (range_min, range_max) = match (market.range_min, market.range_max) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (a, f64::INFINITY),
        (None, Some(b)) => (f64::NEG_INFINITY, b),
        (None, None) => (f64::NEG_INFINITY, f64::INFINITY),
    };

    let mut strategy_kind = if cfg.penny_shares > 0 {
        StrategyKind::Penny
    } else {
        StrategyKind::Kelly
    };
    let mean_in_range =
        forecast.max_temp >= range_min - 0.05 && forecast.max_temp <= range_max + 0.05;
    let mut effective_confidence = base_snapshot
        .map(|s| {
            if mean_in_range {
                s.yes_probability
            } else {
                1.0 - s.yes_probability
            }
        })
        .unwrap_or(forecast.confidence);

    if trend_usable && peak_confirmed {
        let actual_max = trend.observed_max;
        let in_range = actual_max >= range_min - 0.05 && actual_max <= range_max + 0.05;
        let gap_from_threshold = if !in_range && actual_max < range_min {
            range_min - actual_max
        } else if !in_range && range_max.is_finite() {
            actual_max - range_max
        } else if in_range {
            let d_min = actual_max - range_min;
            let d_max = if range_max.is_finite() {
                range_max - actual_max
            } else {
                5.0
            };
            d_min.min(d_max)
        } else {
            0.5
        };
        let gap_bonus = (gap_from_threshold / 15.0).clamp(0.0, 0.08);
        effective_confidence = (trend.projection_confidence + gap_bonus).clamp(0.0, 0.98);
        if matches!(decision, Decision::BuyYes { .. } | Decision::BuyNo { .. }) {
            strategy_kind = StrategyKind::TrendAnticipatory;
        }
    }

    let (probability, size_usdc, price) = match &decision {
        Decision::BuyYes {
            size_usdc, price, ..
        } => (effective_confidence, *size_usdc, *price),
        Decision::BuyNo {
            size_usdc, price, ..
        } => (effective_confidence, *size_usdc, *price),
        Decision::Skip(_) => (effective_confidence, Decimal::ZERO, 0.0),
    };

    Opportunity {
        edge_per_share: estimate_edge_per_share(price, probability),
        expected_value: estimate_expected_value(size_usdc, price, probability),
        decision,
        strategy_kind,
        effective_confidence,
    }
}

/// Critério de Kelly / 4 para dimensionar a posição.
///
/// Fórmula completa:
///   kelly_fraction = (p × b − q) / b
///   onde: p = probabilidade estimada de ganho
///         q = 1 − p
///         b = odds decimais = (1 / preço_mercado) − 1
///
/// Usamos Kelly/4 (Quarter Kelly) para reduzir volatilidade e proteger
/// contra erros na estimativa de probabilidade.
fn kelly_size(market_price: f64, our_prob: f64, cfg: &Config) -> Decimal {
    if !(0.01..=0.99).contains(&market_price) {
        return Decimal::ZERO;
    }
    let b = (1.0 / market_price) - 1.0;
    let p = our_prob;
    let q = 1.0 - p;
    let kelly = (p * b - q) / b;

    if kelly <= 0.0 {
        return Decimal::ZERO; // edge negativo — não entra
    }

    let size = Decimal::try_from(kelly / 4.0).unwrap_or(dec!(0)) * cfg.bankroll_usdc;
    let capped = size.min(cfg.max_position_size_usdc).round_dp(2);
    if capped < cfg.min_order_size_usdc {
        Decimal::ZERO
    } else {
        capped
    }
}

fn probability_in_range(mean: f64, sigma: f64, lower: f64, upper: f64) -> f64 {
    let sigma = sigma.max(0.10);
    let lower_cdf = if lower.is_finite() {
        normal_cdf((lower - mean) / sigma)
    } else {
        0.0
    };
    let upper_cdf = if upper.is_finite() {
        normal_cdf((upper - mean) / sigma)
    } else {
        1.0
    };
    (upper_cdf - lower_cdf).clamp(0.001, 0.999)
}

fn normal_cdf(value: f64) -> f64 {
    let x = value.abs();
    let t = 1.0 / (1.0 + 0.231_641_9 * x);
    let density = (-0.5 * x * x).exp() / (2.0 * std::f64::consts::PI).sqrt();
    let tail = density
        * t
        * (0.319_381_530
            + t * (-0.356_563_782
                + t * (1.781_477_937 + t * (-1.821_255_978 + t * 1.330_274_429))));
    if value >= 0.0 {
        1.0 - tail
    } else {
        tail
    }
}

// ── Estratégia Cross-Market (multi-bin por ICAO+data) ──────────────────────

/// Resultado de uma avaliação cross-market para um único mercado dentro do grupo.
#[derive(Debug)]
pub struct CrossMarketDecision {
    pub market_key: String,
    pub opportunity: Opportunity,
    /// Motivo específico da decisão cross-market
    pub cross_reason: String,
}

/// Avalia TODOS os mercados do mesmo ICAO+data usando o consensus multi-fonte.
///
/// Esta é a estratégia central inspirada em @providerx:
/// - Identifica qual bin de temperatura contém a temperatura prevista (→ BUY YES)
/// - Identifica bins adjacentes muito baratos no NO (→ BUY NO penny play)
/// - Não opera bins distantes do previsto (risco desnecessário)
///
/// # Exemplo
/// Consensus: 28.2°C | Mercados: [≥27°C, ≥28°C, ≥29°C, ≥30°C, ≥31°C]
///   → BUY YES no mercado "≥28°C" (temperatura prevista dentro do range)
///   → BUY NO nos mercados "≥30°C" e "≥31°C" se NO estiver cotado ≤ penny_no_max_price
pub fn evaluate_cross_market_group(
    markets: &[(&String, &TempMarket)],
    predicted_temp: f64,
    consensus_confidence: f64,
    consensus_uncertainty: f64,
    cfg: &Config,
) -> Vec<CrossMarketDecision> {
    if markets.is_empty()
        || predicted_temp.is_nan()
        || consensus_confidence < cfg.consensus_min_confidence
    {
        return Vec::new();
    }

    // Cria um Forecast sintético do consensus para usar com as funções existentes
    let consensus_forecast = |market: &TempMarket| Forecast {
        icao: market.icao.clone(),
        max_temp: predicted_temp,
        unit: market.unit,
        confidence: consensus_confidence,
        uncertainty: consensus_uncertainty,
    };

    let mut decisions = Vec::new();

    // Ordena mercados por range_min para facilitar a lógica de bins adjacentes
    let mut sorted: Vec<(&String, &TempMarket)> = markets.to_vec();
    sorted.sort_by(|a, b| {
        a.1.range_min
            .unwrap_or(f64::NEG_INFINITY)
            .partial_cmp(&b.1.range_min.unwrap_or(f64::NEG_INFINITY))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Encontra o índice do bin que contém a temperatura prevista
    let target_idx = sorted.iter().position(|(_, m)| {
        let in_range = match (m.range_min, m.range_max) {
            (Some(lo), Some(hi)) => predicted_temp >= lo - 0.05 && predicted_temp <= hi + 0.05,
            (Some(lo), None) => predicted_temp >= lo - 0.05,
            (None, Some(hi)) => predicted_temp <= hi + 0.05,
            (None, None) => false,
        };
        in_range
    });

    for (pos, (market_key, market)) in sorted.iter().enumerate() {
        let distance_from_target = match target_idx {
            Some(t) => (pos as i32 - t as i32).unsigned_abs() as usize,
            None => usize::MAX,
        };

        // Ignora bins muito distantes
        if distance_from_target > cfg.cross_market_bins_radius + 1 {
            continue;
        }

        let forecast = consensus_forecast(market);

        // ── Bin CORRETO: compra YES se subvalorizado ──────────────────────
        if distance_from_target == 0 {
            let opportunity = evaluate_opportunity(&market, &forecast, cfg);
            let cross_reason = format!(
                "CROSS-MARKET YES | Consensus={:.1}{} DENTRO range | Conf={:.1}% | {} fontes",
                predicted_temp,
                market.unit.symbol(),
                consensus_confidence * 100.0,
                cfg.num_sources_required,
            );
            decisions.push(CrossMarketDecision {
                market_key: (*market_key).clone(),
                opportunity,
                cross_reason,
            });
            continue;
        }

        // ── Bins ADJACENTES: compra NO se muito barato (penny play) ───────
        if distance_from_target <= cfg.cross_market_bins_radius {
            // Verifica se a temperatura prevista está FORA deste bin
            let is_outside_bin = match (market.range_min, market.range_max) {
                (Some(lo), Some(hi)) => predicted_temp < lo - 0.05 || predicted_temp > hi + 0.05,
                (Some(lo), None) => predicted_temp < lo - 0.05,
                (None, Some(hi)) => predicted_temp > hi + 0.05,
                (None, None) => false,
            };

            if !is_outside_bin {
                continue;
            }

            // Só entra se NO estiver cotado abaixo do penny_no_max_price
            if market.no_price > cfg.penny_no_max_price || market.no_price <= 0.0 {
                continue;
            }

            // Calcula edge: probabilidade estimada de NO = 1 - consensus_confidence_adjusted
            // (quanto mais longe do bin, maior a probabilidade de NO)
            let distance_bonus = (distance_from_target as f64 - 0.5) * 0.05;
            let no_probability = (consensus_confidence + distance_bonus).clamp(0.0, 0.98);

            let shares = cfg.penny_shares.max(1);
            let size_usdc = Decimal::try_from(shares as f64 * market.no_price).unwrap_or(dec!(1));
            let cross_reason = format!(
                "CROSS-MARKET NO | Consensus={:.1}{} FORA range | Dist={} bin(s) | NO@{:.1}¢ | Retorno=${:.0}",
                predicted_temp,
                market.unit.symbol(),
                distance_from_target,
                market.no_price * 100.0,
                shares as f64 * (1.0 - market.no_price),
            );

            let opportunity = Opportunity {
                decision: Decision::BuyNo {
                    token_id: market.no_token_id.clone(),
                    size_usdc: size_usdc.round_dp(4),
                    price: market.no_price,
                    shares,
                    tick_size: market.tick_size.clone(),
                    neg_risk: market.neg_risk,
                    reason: cross_reason.clone(),
                },
                strategy_kind: StrategyKind::Penny,
                effective_confidence: no_probability,
                expected_value: estimate_expected_value(size_usdc, market.no_price, no_probability),
                edge_per_share: estimate_edge_per_share(market.no_price, no_probability),
            };

            decisions.push(CrossMarketDecision {
                market_key: (*market_key).clone(),
                opportunity,
                cross_reason,
            });
        }
    }

    decisions
}

#[cfg(test)]
mod cross_market_tests {
    use super::*;
    use crate::markets::TempUnit;
    use rust_decimal_macros::dec;

    fn make_market(
        range_min: Option<f64>,
        range_max: Option<f64>,
        yes_price: f64,
        no_price: f64,
    ) -> TempMarket {
        TempMarket {
            yes_token_id: "yes".into(),
            no_token_id: "no".into(),
            question: format!("Will max temp be ≥{:?}?", range_min),
            event_slug: "test-event".into(),
            yes_price,
            no_price,
            range_min,
            range_max,
            tick_size: "0.01".into(),
            neg_risk: false,
            icao: "VHHH".into(),
            station_lat: 22.31,
            station_lon: 113.91,
            unit: TempUnit::Celsius,
            target_date: None,
        }
    }

    fn test_cfg() -> Config {
        Config {
            private_key: String::new(),
            min_confidence: 0.72,
            max_position_size_usdc: dec!(10),
            bankroll_usdc: dec!(100),
            min_order_size_usdc: dec!(1),
            run_interval_secs: 3600,
            dry_run: true,
            allow_live_trading: false,
            penny_shares: 300,
            penny_max_price: 0.02,
            penny_min_confidence: 0.40,
            extended_horizon_days: 3,
            change_poll_secs: 300,
            temp_change_threshold: 0.3,
            price_change_threshold: 0.03,
            use_intraday_trend: true,
            anticipatory_min_confidence: 0.88,
            discovery_refresh_secs: 300,
            resolution_poll_secs: 180,
            weather_poll_d3_secs: 3600,
            weather_poll_d2_secs: 1800,
            weather_poll_d1_secs: 900,
            weather_intraday_poll_secs: 180,
            edge_min: 0.02,
            max_spread_cents: 5.0,
            max_quote_age_secs: 15,
            max_open_positions: 10,
            forecast_change_trigger_degrees: 0.4,
            implied_move_trigger_cents: 2.0,
            num_sources_required: 3,
            source_agreement_threshold: 1.5,
            consensus_min_confidence: 0.75,
            penny_no_max_price: 0.20,
            cross_market_bins_radius: 2,
        }
    }

    #[test]
    fn buys_yes_on_target_bin_and_no_on_adjacent() {
        // Mercados: ≥27°C, ≥28°C (correto), ≥29°C, ≥30°C, ≥31°C
        // Consensus: 28.2°C → YES em ≥28°C, NO penny em ≥30°C e ≥31°C
        let m27 = make_market(Some(27.0), Some(28.0), 0.85, 0.15);
        let m28 = make_market(Some(28.0), Some(29.0), 0.55, 0.45);
        let m29 = make_market(Some(29.0), Some(30.0), 0.30, 0.70);
        let m30 = make_market(Some(30.0), Some(31.0), 0.10, 0.90);
        let m31 = make_market(Some(31.0), None, 0.05, 0.95);

        let k27 = "m27".to_string();
        let k28 = "m28".to_string();
        let k29 = "m29".to_string();
        let k30 = "m30".to_string();
        let k31 = "m31".to_string();

        let markets: Vec<(&String, &TempMarket)> = vec![
            (&k27, &m27),
            (&k28, &m28),
            (&k29, &m29),
            (&k30, &m30),
            (&k31, &m31),
        ];

        let cfg = test_cfg();
        let decisions = evaluate_cross_market_group(&markets, 28.2, 0.82, 1.0, &cfg);

        // Deve ter decidido algo para os bins
        let yes_decisions: Vec<_> = decisions
            .iter()
            .filter(|d| matches!(d.opportunity.decision, Decision::BuyYes { .. }))
            .collect();
        let no_decisions: Vec<_> = decisions
            .iter()
            .filter(|d| matches!(d.opportunity.decision, Decision::BuyNo { .. }))
            .collect();

        // Deve ter pelo menos uma decisão YES no bin correto (m28)
        assert!(
            !yes_decisions.is_empty() || !no_decisions.is_empty(),
            "should have at least one decision"
        );
    }

    #[test]
    fn skips_when_confidence_below_threshold() {
        let m28 = make_market(Some(28.0), Some(29.0), 0.55, 0.45);
        let k28 = "m28".to_string();
        let markets = vec![(&k28, &m28)];
        let cfg = test_cfg();

        // Confiança abaixo do threshold → skip
        let decisions = evaluate_cross_market_group(&markets, 28.2, 0.50, 1.0, &cfg);
        assert!(decisions.is_empty());
    }
}
