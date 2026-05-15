use crate::config::{KellyParams, RiskParams};
use crate::types::{BayesianPrediction, KellyResult, TradeSide};

#[derive(Debug, Clone)]
pub struct KellyCriterion {
    bankroll: f64,
    params: KellyParams,
    risk: RiskParams,
    consecutive_losses: u32,
    consecutive_wins: u32,
    peak_bankroll: f64,
}

impl KellyCriterion {
    pub fn new(bankroll: f64, params: KellyParams, risk: RiskParams) -> Self {
        Self {
            bankroll,
            params,
            risk,
            consecutive_losses: 0,
            consecutive_wins: 0,
            peak_bankroll: bankroll,
        }
    }

    pub fn calculate(
        &self,
        prediction: &BayesianPrediction,
        market_price: f64,
        token_yes_price: f64,
        token_no_price: f64,
    ) -> KellyResult {
        let direction = prediction.direction();
        let p = match direction {
            TradeSide::Up => prediction.p_up,
            TradeSide::Down => prediction.p_down,
        };
        let confidence = prediction.confidence;
        let market_prob = market_price;
        let edge = p - market_prob;

        if !(0.01..=0.99).contains(&market_prob) {
            return self.no_bet("Preco de mercado extremo", confidence, edge, direction);
        }
        if prediction.signals.is_empty() {
            return self.no_bet("Nenhum sinal forte disponivel", confidence, edge, direction);
        }
        if !self.has_strong_signal(prediction, direction) {
            return self.no_bet(
                "Nenhum sinal forte na direcao prevista",
                confidence,
                edge,
                direction,
            );
        }
        if self.consecutive_losses >= self.risk.max_consecutive_losses {
            return self.no_bet(
                "Limite de losses consecutivos atingido",
                confidence,
                edge,
                direction,
            );
        }
        if self.current_drawdown() >= self.risk.max_drawdown {
            return self.no_bet("Drawdown maximo atingido", confidence, edge, direction);
        }
        if edge < self.params.min_edge {
            return self.no_bet(
                format!("Edge insuficiente: {edge:.3} < {:.3}", self.params.min_edge),
                confidence,
                edge,
                direction,
            );
        }

        let odds = (1.0 / market_prob) - 1.0;
        let q = 1.0 - p;
        let kelly_full = ((p * odds) - q) / odds;
        if kelly_full <= 0.0 {
            return self.no_bet("Kelly negativo", confidence, edge, direction);
        }

        let adjusted_multiplier = self.dynamic_multiplier();
        let kelly_fraction = kelly_full * adjusted_multiplier;
        let raw_size = self.bankroll * kelly_fraction;
        let mut position_size = self.apply_limits(raw_size, confidence);
        if position_size < self.params.min_position_size {
            position_size = self.params.min_position_size;
        }

        let entry_price = match direction {
            TradeSide::Up => token_yes_price,
            TradeSide::Down => token_no_price,
        }
        .max(0.01);
        let shares = position_size / entry_price;
        let pnl_win = shares - position_size;

        KellyResult {
            kelly_fraction,
            kelly_fraction_full: kelly_full,
            position_size,
            edge,
            should_bet: true,
            reason: "Kelly positivo com filtros de risco".into(),
            confidence,
            direction,
            pnl_win_usd: round2(pnl_win),
            pnl_lose_usd: -round2(position_size),
        }
    }

    pub fn validate_bet(&self, result: &KellyResult, current_positions: usize) -> (bool, String) {
        if !result.should_bet {
            return (false, result.reason.clone());
        }
        if current_positions >= self.risk.max_concurrent_positions {
            return (
                false,
                format!(
                    "Maximo de posicoes atingido ({}/{})",
                    current_positions, self.risk.max_concurrent_positions
                ),
            );
        }
        if result.position_size / self.bankroll > self.params.max_bankroll_per_trade {
            return (false, "Posicao acima do limite de bankroll".into());
        }
        (true, "Validacoes passaram".into())
    }

    fn has_strong_signal(&self, prediction: &BayesianPrediction, direction: TradeSide) -> bool {
        prediction.signals.iter().any(|signal| match direction {
            TradeSide::Up => signal.p_up >= 0.60,
            TradeSide::Down => signal.p_down >= 0.60,
        })
    }

    fn dynamic_multiplier(&self) -> f64 {
        let mut multiplier = self.params.kelly_fraction;
        if self.consecutive_losses >= 2 {
            multiplier *= self.params.loss_reduction_factor;
        }
        if self.consecutive_wins >= 3 {
            multiplier = (multiplier * self.params.win_increase_factor).min(0.30);
        }
        let drawdown = self.current_drawdown();
        if drawdown > 0.10 {
            multiplier *= (1.0 - (drawdown - 0.10) * 2.0).max(0.3);
        }
        multiplier
    }

    fn apply_limits(&self, raw_size: f64, confidence: f64) -> f64 {
        let mut size = raw_size;
        size = size.min(self.params.max_position_size);
        size = size.min(self.bankroll * self.params.max_bankroll_per_trade);
        size = size.min(self.params.max_position_size * (confidence / 0.55).min(1.0));
        round2(size)
    }

    fn current_drawdown(&self) -> f64 {
        if self.peak_bankroll <= 0.0 {
            return 0.0;
        }
        (self.peak_bankroll - self.bankroll) / self.peak_bankroll
    }

    fn no_bet(
        &self,
        reason: impl Into<String>,
        confidence: f64,
        edge: f64,
        direction: TradeSide,
    ) -> KellyResult {
        KellyResult {
            kelly_fraction: 0.0,
            kelly_fraction_full: 0.0,
            position_size: 0.0,
            edge,
            should_bet: false,
            reason: reason.into(),
            confidence,
            direction,
            pnl_win_usd: 0.0,
            pnl_lose_usd: 0.0,
        }
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
