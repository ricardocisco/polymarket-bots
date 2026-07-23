use crate::config::BayesianParams;
use crate::types::{BayesianPrediction, BayesianSignal, Candle};

#[derive(Debug, Clone)]
pub struct BayesianModel {
    params: BayesianParams,
}

impl BayesianModel {
    pub fn new(params: BayesianParams) -> Self {
        Self { params }
    }

    pub fn predict(
        &self,
        symbol: impl Into<String>,
        candles: &[Candle],
        strike_price: Option<f64>,
        current_price: Option<f64>,
        minutes_to_expiry: Option<f64>,
    ) -> BayesianPrediction {
        let symbol = symbol.into();
        if candles.len() < 15 {
            return BayesianPrediction {
                symbol,
                p_up: self.params.prior_up,
                p_down: self.params.prior_down,
                confidence: 0.5,
                edge: 0.0,
                signals: Vec::new(),
                strike_price,
                current_price,
            };
        }

        let current = current_price.or_else(|| candles.last().map(|c| c.close));
        let mut signals = Vec::new();
        let mut raw_rsi = 50.0;

        if matches!(minutes_to_expiry, Some(minutes) if minutes < 3.0) && candles.len() >= 3 {
            let base = candles[candles.len() - 3].close;
            if base > 0.0 {
                let change_pct = (candles.last().unwrap().close - base) / base * 100.0;
                if let Some(signal) = self.short_momentum_signal(change_pct) {
                    signals.push(signal);
                }
            }
        }

        if let Some(signal) = self.momentum_signal(candles) {
            raw_rsi = signal.raw_value;
            signals.push(signal);
        }
        if let Some(signal) = self.volume_signal(candles) {
            signals.push(signal);
        }
        if let Some(signal) = self.volatility_signal(candles) {
            signals.push(signal);
        }
        if let Some(signal) = self.trend_signal(candles) {
            signals.push(signal);
        }

        if raw_rsi > self.params.rsi_extreme || raw_rsi < (100.0 - self.params.rsi_extreme) {
            for signal in &mut signals {
                if signal.name == "momentum" {
                    signal.weight *= self.params.rsi_extreme_boost;
                }
            }
        }

        let (mut p_up, mut p_down) = self.combine_signals(&signals);
        if let (Some(strike), Some(current)) = (strike_price, current) {
            (p_up, p_down) = self.apply_strike_penalty(p_up, p_down, strike, current);
        }

        BayesianPrediction {
            symbol,
            p_up,
            p_down,
            confidence: p_up.max(p_down),
            edge: (p_up - p_down).abs(),
            signals,
            strike_price,
            current_price: current,
        }
    }

    fn momentum_signal(&self, candles: &[Candle]) -> Option<BayesianSignal> {
        let closes = closes(candles);
        let rsi = rsi(&closes, 14)?;

        if self.params.rsi_neutral_low <= rsi && rsi <= self.params.rsi_neutral_high {
            return None;
        }

        let (p_up, p_down) = if rsi > self.params.rsi_extreme {
            let extreme_factor = ((rsi - self.params.rsi_extreme) / 10.0).min(0.15);
            let p_up = (0.72 + extreme_factor).min(0.82);
            (p_up, 1.0 - p_up)
        } else if rsi > self.params.rsi_overbought {
            let strength = (rsi - self.params.rsi_overbought)
                / (self.params.rsi_extreme - self.params.rsi_overbought);
            let p_up = 0.65 + strength * 0.07;
            (p_up, 1.0 - p_up)
        } else if rsi > self.params.rsi_neutral_high {
            (0.57, 0.43)
        } else if rsi < self.params.rsi_oversold {
            let oversold_depth = (self.params.rsi_oversold - rsi) / self.params.rsi_oversold;
            let p_down = 0.60 + (oversold_depth * 0.15).min(0.12);
            (1.0 - p_down, p_down)
        } else {
            (0.46, 0.54)
        };

        Some(BayesianSignal {
            name: "momentum".into(),
            p_up,
            p_down,
            confidence: ((rsi - 50.0).abs() / 50.0).clamp(0.0, 1.0),
            weight: self.params.momentum_weight,
            raw_value: rsi,
        })
    }

    fn trend_signal(&self, candles: &[Candle]) -> Option<BayesianSignal> {
        let closes = closes(candles);
        let fast = ema(&closes, 5)?;
        let slow = ema(&closes, 15)?;
        if slow <= 0.0 {
            return None;
        }
        let gap_pct = (fast - slow) / slow * 100.0;
        if gap_pct.abs() < self.params.trend_min_gap_pct {
            return None;
        }

        let strength = (gap_pct.abs() / 0.4).min(1.0);
        let (p_up, p_down) = if fast > slow {
            let p_up = 0.55 + strength * (self.params.trend_max_confidence - 0.55);
            (p_up, 1.0 - p_up)
        } else {
            let p_down = 0.55 + strength * (self.params.trend_max_confidence - 0.55);
            (1.0 - p_down, p_down)
        };

        Some(BayesianSignal {
            name: "trend".into(),
            p_up,
            p_down,
            confidence: (gap_pct.abs() / 0.3).min(1.0),
            weight: self.params.trend_weight,
            raw_value: gap_pct,
        })
    }

    fn volume_signal(&self, candles: &[Candle]) -> Option<BayesianSignal> {
        if candles.len() < 5 {
            return None;
        }
        let avg_volume = candles.iter().map(|c| c.volume).sum::<f64>() / candles.len() as f64;
        if avg_volume <= 0.0 {
            return None;
        }
        let recent_volume = candles[candles.len() - 5..]
            .iter()
            .map(|c| c.volume)
            .sum::<f64>()
            / 5.0;
        let ratio = recent_volume / avg_volume;
        let price_base = candles[candles.len() - 5].close;
        let price_change = if price_base > 0.0 {
            (candles.last().unwrap().close - price_base) / price_base
        } else {
            0.0
        };

        let (p_up, p_down) = if ratio > self.params.volume_extreme_threshold {
            if price_change > 0.001 {
                (0.75, 0.25)
            } else if price_change < -0.001 {
                (0.25, 0.75)
            } else {
                (0.60, 0.40)
            }
        } else if ratio > self.params.volume_relative_threshold {
            if price_change > 0.0 {
                (0.65, 0.35)
            } else if price_change < 0.0 {
                (0.35, 0.65)
            } else {
                (0.55, 0.45)
            }
        } else {
            (0.50, 0.50)
        };

        Some(BayesianSignal {
            name: "volume".into(),
            p_up,
            p_down,
            confidence: (ratio / self.params.volume_extreme_threshold).min(1.0),
            weight: self.params.volume_weight,
            raw_value: ratio,
        })
    }

    fn volatility_signal(&self, candles: &[Candle]) -> Option<BayesianSignal> {
        if candles.len() < 2 {
            return None;
        }
        let mut trs = Vec::new();
        for idx in 1..candles.len() {
            let h = candles[idx].high;
            let l = candles[idx].low;
            let pc = candles[idx - 1].close;
            trs.push((h - l).max((h - pc).abs()).max((l - pc).abs()));
        }
        let tail = trs.iter().rev().take(14).copied().collect::<Vec<_>>();
        if tail.is_empty() {
            return None;
        }
        let atr = tail.iter().sum::<f64>() / tail.len() as f64;
        let current = candles.last()?.close;
        if current <= 0.0 {
            return None;
        }
        let atr_pct = atr / current * 100.0;
        let base = if candles.len() >= 6 {
            candles[candles.len() - 6].close
        } else {
            candles[0].close
        };
        let price_change_5m = if base > 0.0 {
            (current - base) / base
        } else {
            0.0
        };
        let trend_dir = if price_change_5m > 0.0 { 1.0 } else { -1.0 };

        let p_up = if atr_pct > 0.15 {
            0.50 - trend_dir * 0.08
        } else {
            0.50 + trend_dir * 0.04
        };

        Some(BayesianSignal {
            name: "volatility".into(),
            p_up,
            p_down: 1.0 - p_up,
            confidence: (atr_pct / 0.2).min(1.0),
            weight: self.params.volatility_weight,
            raw_value: atr,
        })
    }

    fn short_momentum_signal(&self, price_change_pct: f64) -> Option<BayesianSignal> {
        if price_change_pct.abs() <= 0.1 {
            return None;
        }
        let strength = (price_change_pct.abs() / 0.5).min(0.2);
        let (p_up, p_down) = if price_change_pct > 0.1 {
            (0.5 + strength, 0.5 - strength)
        } else {
            (0.5 - strength, 0.5 + strength)
        };

        Some(BayesianSignal {
            name: "short_momentum".into(),
            p_up,
            p_down,
            confidence: (price_change_pct.abs() / 1.0).min(0.7),
            weight: 0.20,
            raw_value: price_change_pct,
        })
    }

    fn combine_signals(&self, signals: &[BayesianSignal]) -> (f64, f64) {
        if signals.is_empty() {
            return (self.params.prior_up, self.params.prior_down);
        }
        let mut log_odds = (self.params.prior_up / self.params.prior_down).ln();
        // Os sinais usam os mesmos candles e, portanto, sao correlacionados.
        // O fator de shrinkage impede que a multiplicacao de odds produza
        // probabilidades extremas por dupla contagem da mesma informacao.
        let shrinkage = 0.60;
        for signal in signals {
            if signal.p_down > 0.0 {
                log_odds += shrinkage * signal.weight * (signal.p_up / signal.p_down).ln();
            }
        }
        let odds = log_odds.exp();
        let p_up = odds / (1.0 + odds);
        (p_up, 1.0 - p_up)
    }

    fn apply_strike_penalty(
        &self,
        mut p_up: f64,
        mut p_down: f64,
        strike: f64,
        current: f64,
    ) -> (f64, f64) {
        if strike <= 0.0 {
            return (p_up, p_down);
        }
        let diff_pct = (current - strike) / strike * 100.0;
        let predicted_up = p_up > p_down;
        if predicted_up && diff_pct < -0.5 {
            let penalty = (diff_pct.abs() / self.params.strike_penalty_factor).min(0.10);
            p_up -= penalty;
        } else if !predicted_up && diff_pct > 0.5 {
            let penalty = (diff_pct.abs() / self.params.strike_penalty_factor).min(0.10);
            p_up += penalty;
        }
        p_up = p_up.clamp(0.01, 0.99);
        p_down = 1.0 - p_up;
        (p_up, p_down)
    }
}

fn closes(candles: &[Candle]) -> Vec<f64> {
    candles.iter().map(|c| c.close).collect()
}

fn rsi(prices: &[f64], period: usize) -> Option<f64> {
    if prices.len() <= period {
        return None;
    }
    let mut gains = Vec::new();
    let mut losses = Vec::new();
    for pair in prices.windows(2) {
        let delta = pair[1] - pair[0];
        if delta > 0.0 {
            gains.push(delta);
            losses.push(0.0);
        } else {
            gains.push(0.0);
            losses.push(-delta);
        }
    }
    let gain_tail = gains.iter().rev().take(period).sum::<f64>() / period as f64;
    let loss_tail = losses.iter().rev().take(period).sum::<f64>() / period as f64;
    if loss_tail == 0.0 {
        return Some(100.0);
    }
    let rs = gain_tail / loss_tail;
    Some(100.0 - (100.0 / (1.0 + rs)))
}

fn ema(prices: &[f64], period: usize) -> Option<f64> {
    let first = *prices.first()?;
    let multiplier = 2.0 / (period as f64 + 1.0);
    let mut value = first;
    for price in &prices[1..] {
        value = price * multiplier + value * (1.0 - multiplier);
    }
    Some(value)
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
    fn oversold_momentum_points_down_in_momentum_model() {
        let model = BayesianModel::new(crate::config::base_bayesian());
        let prices = (0..30).map(|i| 100.0 - i as f64).collect::<Vec<_>>();
        let signal = model.momentum_signal(&candles(&prices)).unwrap();
        assert!(signal.p_down > signal.p_up);
    }
}
