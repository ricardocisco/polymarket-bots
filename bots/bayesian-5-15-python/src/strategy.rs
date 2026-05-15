use chrono::{DateTime, Timelike, Utc};

use crate::config::Config;
use crate::kelly::KellyCriterion;
use crate::model::BayesianModel;
use crate::types::{
    Action, Candle, OrderBookPrice, OrderTimeInForce, SignalDecision, StrategyDiagnostics,
    StrategyMarket, TradeSide,
};

#[derive(Debug, Clone)]
pub struct BayesianStrategy {
    cfg: Config,
    model: BayesianModel,
    kelly: KellyCriterion,
}

impl BayesianStrategy {
    pub fn new(cfg: Config) -> Self {
        Self {
            model: BayesianModel::new(cfg.bayesian.clone()),
            kelly: KellyCriterion::new(cfg.bankroll, cfg.kelly.clone(), cfg.risk.clone()),
            cfg,
        }
    }

    pub fn decide(
        &self,
        market: &StrategyMarket,
        candles: &[Candle],
        up_book: &OrderBookPrice,
        down_book: &OrderBookPrice,
        current_positions: usize,
        now: DateTime<Utc>,
    ) -> SignalDecision {
        if candles.len() < 30 {
            return SignalDecision::hold("candles insuficientes");
        }
        if market.strike_price <= 0.0 {
            return SignalDecision::hold("sem strike");
        }

        let current_price = candles.last().map(|c| c.close);
        let prediction = self.model.predict(
            &market.config.binance_symbol,
            candles,
            Some(market.strike_price),
            current_price,
            Some(market.minutes_left_at(now)),
        );

        if !prediction.should_trade(self.cfg.bayesian.min_trade_edge) {
            return SignalDecision::hold(format!(
                "edge bayes insuficiente: {:.3} < {:.3}",
                prediction.edge, self.cfg.bayesian.min_trade_edge
            ));
        }

        if let Some(reason) = self.apply_smart_filters(&prediction, candles, now) {
            return SignalDecision::hold(reason);
        }

        let direction = prediction.direction();
        let book = match direction {
            TradeSide::Up => up_book,
            TradeSide::Down => down_book,
        };
        let market_price = book.best_ask;
        if !(self.cfg.min_buy_price..=self.cfg.max_buy_price).contains(&market_price) {
            return SignalDecision::hold(format!(
                "ask {:.3} fora da faixa {:.2}-{:.2}",
                market_price, self.cfg.min_buy_price, self.cfg.max_buy_price
            ));
        }

        let ask_size_usd = book.ask_size * market_price;
        if ask_size_usd < self.cfg.min_ask_size_usd {
            return SignalDecision::hold(format!(
                "liquidez insuficiente: ${ask_size_usd:.2} < ${:.2}",
                self.cfg.min_ask_size_usd
            ));
        }

        let token_yes_price = if direction == TradeSide::Up {
            market_price
        } else {
            (1.0 - market_price).max(0.01)
        };
        let token_no_price = if direction == TradeSide::Down {
            market_price
        } else {
            (1.0 - market_price).max(0.01)
        };

        let kelly =
            self.kelly
                .calculate(&prediction, market_price, token_yes_price, token_no_price);
        let (ok, reason) = self.kelly.validate_bet(&kelly, current_positions);
        if !ok {
            return SignalDecision::hold(reason);
        }

        let stake = self.cfg.flat_stake_usdc.unwrap_or(kelly.position_size);
        let size = (stake / market_price).floor().max(1.0) as u32;
        let rsi = prediction
            .signals
            .iter()
            .find(|signal| signal.name == "momentum")
            .map(|signal| signal.raw_value);
        let volume_ratio = prediction
            .signals
            .iter()
            .find(|signal| signal.name == "volume")
            .map(|signal| signal.raw_value);
        let momentum = momentum_pct(candles, 5).map(|v| v * 100.0);
        let distance_pct =
            current_price.map(|price| (price - market.strike_price) / market.strike_price * 100.0);

        SignalDecision {
            action: Action::Buy,
            side: Some(direction),
            size,
            reason: format!(
                "bayes {} p_up={:.3} p_down={:.3} edge={:.3} kelly={:.3}",
                direction,
                prediction.p_up,
                prediction.p_down,
                prediction.edge,
                kelly.kelly_fraction
            ),
            order_type: Some(OrderTimeInForce::MarketableLimit),
            limit_price_cents: Some(market_price * 100.0),
            confidence: Some(prediction.confidence),
            diagnostics: Some(StrategyDiagnostics {
                execution_mode: if self.cfg.live_trading_enabled() {
                    "live".into()
                } else {
                    "dry-run".into()
                },
                size: Some(size),
                confidence: Some(prediction.confidence),
                order_type: Some("GTC".into()),
                market_price: Some(market_price),
                min_entry_price: Some(self.cfg.min_buy_price),
                max_entry_price: Some(self.cfg.max_buy_price),
                minutes_left: Some(market.minutes_left_at(now)),
                distance_pct,
                rsi,
                p_up: Some(prediction.p_up),
                p_down: Some(prediction.p_down),
                edge: Some(prediction.edge),
                kelly_fraction: Some(kelly.kelly_fraction),
                kelly_full: Some(kelly.kelly_fraction_full),
                volume_ratio,
                momentum,
                notes: prediction
                    .signals
                    .iter()
                    .map(|signal| {
                        format!(
                            "{} p_up={:.3} p_down={:.3} raw={:.4} weight={:.3}",
                            signal.name,
                            signal.p_up,
                            signal.p_down,
                            signal.raw_value,
                            signal.weight
                        )
                    })
                    .collect(),
            }),
            stake_usdc: Some(stake),
        }
    }

    fn apply_smart_filters(
        &self,
        prediction: &crate::types::BayesianPrediction,
        candles: &[Candle],
        now: DateTime<Utc>,
    ) -> Option<String> {
        let filters = &self.cfg.filters;
        if !filters.enabled {
            return None;
        }

        if filters.filter_by_hour {
            let hour = now.hour();
            if filters.blocked_hours.contains(&hour) {
                return Some(format!("horario bloqueado: {hour}h UTC"));
            }
            if !filters.preferred_hours.is_empty() && !filters.preferred_hours.contains(&hour) {
                return Some(format!("fora dos horarios preferidos: {hour}h UTC"));
            }
        }

        if !filters.allow_down_trades && prediction.direction() == TradeSide::Down {
            return Some("trades DOWN desabilitados pelo modo".into());
        }

        if filters.momentum_5m_min > 0.0 {
            let Some(momentum) = momentum_pct(candles, 5) else {
                return Some("sem candles para momentum 5m".into());
            };
            if momentum.abs() < filters.momentum_5m_min {
                return Some(format!(
                    "momentum 5m insuficiente: {:.3}% < {:.3}%",
                    momentum.abs() * 100.0,
                    filters.momentum_5m_min * 100.0
                ));
            }
        }

        if let Some(max_ratio) = filters.volume_ratio_max {
            if let Some(volume_ratio) = prediction
                .signals
                .iter()
                .find(|signal| signal.name == "volume")
                .map(|signal| signal.raw_value)
            {
                if volume_ratio > max_ratio {
                    return Some(format!(
                        "volume ratio alto: {:.2} > {:.2}",
                        volume_ratio, max_ratio
                    ));
                }
            }
        }

        None
    }
}

fn momentum_pct(candles: &[Candle], periods: usize) -> Option<f64> {
    if candles.len() <= periods {
        return None;
    }
    let start = candles[candles.len() - periods].close;
    let end = candles.last()?.close;
    if start <= 0.0 {
        return None;
    }
    Some((end - start) / start)
}
