use chrono::{DateTime, Utc};

use crate::config::Config;
use crate::types::{
    Action, Candle, MarketSignal, OrderTimeInForce, SignalDecision, StrategyDiagnostics,
    StrategyMarket, TradeSide,
};

pub struct SniperStrategy {
    cfg: Config,
}

impl SniperStrategy {
    pub fn new(cfg: Config) -> Self {
        Self { cfg }
    }

    pub fn decide(
        &self,
        market: &StrategyMarket,
        signal: &MarketSignal,
        now: DateTime<Utc>,
    ) -> SignalDecision {
        let minutes_left = market.minutes_left_at(now);
        if minutes_left < self.cfg.min_minutes_left {
            return SignalDecision::hold(format!(
                "muito perto do fim ({minutes_left:.2}m < {:.2}m)",
                self.cfg.min_minutes_left
            ));
        }
        if minutes_left > self.cfg.max_minutes_left {
            return SignalDecision::hold(format!(
                "ainda cedo ({minutes_left:.2}m > {:.2}m)",
                self.cfg.max_minutes_left
            ));
        }

        let side = match signal.recommended_side {
            Some(side) => side,
            None => {
                if market.up_price > 0.98 {
                    TradeSide::Up
                } else if market.down_price > 0.98 {
                    TradeSide::Down
                } else {
                    return SignalDecision::hold("sinal neutro sem dominancia de preco");
                }
            }
        };

        let price = market.price(side);
        if price < self.cfg.min_entry_price {
            return SignalDecision::hold(format!(
                "preco baixo {:.3} < {:.3}",
                price, self.cfg.min_entry_price
            ));
        }
        if price > self.cfg.max_entry_price {
            return SignalDecision::hold(format!(
                "preco teto {:.3} > {:.3}",
                price, self.cfg.max_entry_price
            ));
        }
        if side == TradeSide::Up && signal.distance_pct < 0.0 {
            return SignalDecision::hold(format!(
                "UP caro mas underlying abaixo do strike ({:.3}%)",
                signal.distance_pct
            ));
        }
        if side == TradeSide::Down && signal.distance_pct > 0.0 {
            return SignalDecision::hold(format!(
                "DOWN caro mas underlying acima do strike ({:.3}%)",
                signal.distance_pct
            ));
        }

        let probability = match side {
            TradeSide::Up => signal.up_probability,
            TradeSide::Down => 1.0 - signal.up_probability,
        };
        let edge = probability - price;
        if edge < self.cfg.min_edge {
            return SignalDecision::hold(format!(
                "edge {:.2}pp abaixo do minimo {:.2}pp",
                edge * 100.0,
                self.cfg.min_edge * 100.0
            ));
        }

        SignalDecision {
            action: Action::Buy,
            side: Some(side),
            size: 1,
            reason: format!(
                "{} | {} @ {:.3} | edge {:.2}pp | {:.2}m left",
                market.display_label(),
                side,
                price,
                edge * 100.0,
                minutes_left
            ),
            order_type: Some(OrderTimeInForce::MarketableLimit),
            limit_price_cents: Some(price * 100.0),
            confidence: Some(probability),
            diagnostics: Some(StrategyDiagnostics {
                execution_mode: if self.cfg.live_trading_enabled() {
                    "live".into()
                } else {
                    "paper".into()
                },
                size: Some(1),
                confidence: Some(probability),
                order_type: Some("marketable_limit".into()),
                market_price: Some(price),
                min_entry_price: Some(self.cfg.min_entry_price),
                max_entry_price: Some(self.cfg.max_entry_price),
                minutes_left: Some(minutes_left),
                distance_pct: Some(signal.distance_pct),
                rsi: Some(signal.rsi),
                momentum: Some(signal.price_momentum),
                notes: vec![format!("up_probability={:.4}", signal.up_probability)],
            }),
            stake_usdc: Some(self.cfg.position_size_usdc.min(self.cfg.bankroll)),
        }
    }
}

pub fn analyze_candles(
    symbol: &str,
    strike_price: f64,
    minutes_left: f64,
    candles: &[Candle],
) -> Option<MarketSignal> {
    if candles.is_empty() {
        return None;
    }

    let closes = candles.iter().map(|c| c.close).collect::<Vec<_>>();
    let current_price = *closes.last()?;
    let rsi = calculate_rsi(&closes, 14);
    let macd = calculate_macd(&closes);
    let momentum = price_momentum(&closes, 5);
    let distance_pct = if strike_price > 0.0 {
        ((current_price - strike_price) / strike_price) * 100.0
    } else {
        0.0
    };

    // Probabilidade estrutural de terminar acima do strike. A volatilidade e a
    // distancia sao calculadas em log-retornos, portanto funcionam na mesma
    // escala para BTC, ETH e XRP. Indicadores tecnicos apenas fazem um ajuste
    // pequeno; eles nao sao tratados como evidencias independentes.
    let sigma_per_minute = realized_log_volatility(&closes).max(0.000_05);
    let horizon = minutes_left.max(1.0 / 60.0);
    let z = if strike_price > 0.0 && current_price > 0.0 {
        (current_price / strike_price).ln() / (sigma_per_minute * horizon.sqrt())
    } else {
        0.0
    };
    let structural_prob = normal_cdf(z);
    let rsi_adjustment = ((rsi - 50.0) / 50.0).clamp(-1.0, 1.0) * 0.025;
    let momentum_adjustment = (momentum / 0.25).clamp(-1.0, 1.0) * 0.025;
    let macd_pct = if current_price > 0.0 {
        macd / current_price * 100.0
    } else {
        0.0
    };
    let macd_adjustment = (macd_pct / 0.05).clamp(-1.0, 1.0) * 0.015;
    let up_prob = (structural_prob + rsi_adjustment + momentum_adjustment + macd_adjustment)
        .clamp(0.001, 0.999);
    let down_prob = 1.0 - up_prob;
    let edge = (up_prob - 0.5_f64).abs();
    let confidence = if edge >= 0.15 {
        "high"
    } else if edge >= 0.08 {
        "medium"
    } else {
        "low"
    };
    let recommended_side = if up_prob >= 0.54 {
        Some(TradeSide::Up)
    } else if down_prob >= 0.54 {
        Some(TradeSide::Down)
    } else {
        None
    };

    Some(MarketSignal {
        symbol: symbol.into(),
        current_price,
        strike_price,
        distance_pct,
        rsi,
        macd_signal: macd,
        price_momentum: momentum,
        up_probability: up_prob,
        confidence: confidence.into(),
        recommended_side,
    })
}

fn calculate_rsi(closes: &[f64], period: usize) -> f64 {
    if closes.len() < period + 1 {
        return 50.0;
    }
    let start = closes.len() - period - 1;
    let mut gains = 0.0;
    let mut losses = 0.0;
    for i in start + 1..=start + period {
        let delta = closes[i] - closes[i - 1];
        if delta > 0.0 {
            gains += delta;
        } else {
            losses += delta.abs();
        }
    }
    let avg_gain = gains / period as f64;
    let avg_loss = losses / period as f64;
    if avg_loss == 0.0 {
        return 100.0;
    }
    let rs = avg_gain / avg_loss;
    100.0 - (100.0 / (1.0 + rs))
}

fn calculate_macd(closes: &[f64]) -> f64 {
    if closes.len() < 26 {
        return 0.0;
    }
    ema(closes, 12) - ema(closes, 26)
}

fn ema(data: &[f64], period: usize) -> f64 {
    if data.len() < period {
        return *data.last().unwrap_or(&0.0);
    }
    let k = 2.0 / (period as f64 + 1.0);
    let mut value = data[..period].iter().sum::<f64>() / period as f64;
    for price in &data[period..] {
        value = price * k + value * (1.0 - k);
    }
    value
}

fn price_momentum(closes: &[f64], periods: usize) -> f64 {
    if closes.len() < periods + 1 {
        return 0.0;
    }
    let old = closes[closes.len() - periods - 1];
    let new = closes[closes.len() - 1];
    if old == 0.0 {
        0.0
    } else {
        ((new - old) / old) * 100.0
    }
}

fn realized_log_volatility(closes: &[f64]) -> f64 {
    let returns = closes
        .windows(2)
        .filter_map(|window| {
            (window[0] > 0.0 && window[1] > 0.0).then(|| (window[1] / window[0]).ln())
        })
        .collect::<Vec<_>>();
    if returns.len() < 2 {
        return 0.0;
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance = returns
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (returns.len() - 1) as f64;
    variance.sqrt()
}

// Aproximacao de Abramowitz-Stegun para a CDF normal padrao.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn candles(prices: &[f64]) -> Vec<Candle> {
        prices
            .iter()
            .enumerate()
            .map(|(i, price)| Candle {
                open_time: i as i64 * 60,
                open: *price,
                high: *price,
                low: *price,
                close: *price,
                volume: 1.0,
            })
            .collect()
    }

    #[test]
    fn probability_can_exceed_entry_band_when_strike_is_safe() {
        let prices = (0..50).map(|i| 100.0 + i as f64 * 0.02).collect::<Vec<_>>();
        let signal = analyze_candles("TEST", 99.0, 1.0, &candles(&prices)).unwrap();
        assert!(signal.up_probability > 0.99, "p={}", signal.up_probability);
    }

    #[test]
    fn scale_invariant_probability_is_similar() {
        let prices = (0..50).map(|i| 100.0 + i as f64 * 0.03).collect::<Vec<_>>();
        let scaled = prices
            .iter()
            .map(|price| price * 1000.0)
            .collect::<Vec<_>>();
        let a = analyze_candles("A", 99.0, 2.0, &candles(&prices)).unwrap();
        let b = analyze_candles("B", 99_000.0, 2.0, &candles(&scaled)).unwrap();
        assert!((a.up_probability - b.up_probability).abs() < 1e-9);
    }
}
