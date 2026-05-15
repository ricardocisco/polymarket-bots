use chrono::{DateTime, Duration, Utc};

use crate::config::Config;
use crate::strategy::base::Strategy;
use crate::types::{
    Action, OrderTimeInForce, QuoteSnapshot, SignalDecision, StrategyDiagnostics, StrategyInput,
    StrategyMarket,
};

pub struct SixAmStrategy {
    pub config: Config,
}

impl SixAmStrategy {
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl Strategy for SixAmStrategy {
    fn name(&self) -> &'static str {
        "btc_6am"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn decide(&mut self, input: &StrategyInput) -> SignalDecision {
        let current_price = input
            .quote
            .best_ask
            .or(input.quote.last_price)
            .unwrap_or(-1.0);

        if !input.is_candidate {
            return SignalDecision::hold("mercado fora do filtro estrutural");
        }
        if !input.market_closed
            && input.liquidity_num.unwrap_or_default() < self.config.min_liquidity
        {
            return SignalDecision::hold_owned(format!(
                "liquidez {:.2} < mínimo {:.2}",
                input.liquidity_num.unwrap_or_default(),
                self.config.min_liquidity
            ));
        }
        if !(0.0..1.0).contains(&current_price) {
            return SignalDecision::hold_owned(format!("preço inválido: {current_price:.4}"));
        }
        if current_price > self.config.max_entry_price {
            return SignalDecision::hold_owned(format!(
                "preço {:.3} acima do teto {:.3}",
                current_price, self.config.max_entry_price
            ));
        }

        let edge = self.config.expected_win_rate - current_price;
        if edge < self.config.min_edge {
            return SignalDecision::hold_owned(format!(
                "edge {:.2}pp abaixo do mínimo {:.2}pp",
                edge * 100.0,
                self.config.min_edge * 100.0
            ));
        }

        SignalDecision {
            action: Action::Buy,
            side: Some(self.config.trade_direction),
            size: 1,
            reason: format!(
                "{} | {} | edge {:.2}pp",
                input.market_ticker,
                self.config.trade_direction,
                edge * 100.0
            ),
            order_type: Some(OrderTimeInForce::MarketableLimit),
            limit_price_cents: Some(current_price * 100.0),
            confidence: Some(self.config.expected_win_rate),
            diagnostics: Some(StrategyDiagnostics {
                execution_mode: if self.config.dry_run {
                    "paper".into()
                } else {
                    "live".into()
                },
                size: Some(1),
                confidence: Some(self.config.expected_win_rate),
                order_type: Some("marketable_limit".into()),
                edge: Some(edge),
                max_entry_price: Some(self.config.max_entry_price),
                current_price: Some(current_price),
                notes: vec![format!("seconds_to_close={:.1}", input.seconds_to_close)],
            }),
            stake_usdc: Some(
                self.config
                    .position_size_usdc
                    .to_string()
                    .parse()
                    .unwrap_or(0.0),
            ),
        }
    }
}

pub fn strategy_summary(cfg: &Config) -> String {
    format!(
        "BTC 5m @ {:02}:00 UTC | lado={} | hit-rate esperado={:.1}% | entrada<= {:.3} | edge mín={:.1}pp | stake={} USDC",
        cfg.target_hour_utc,
        cfg.trade_direction,
        cfg.expected_win_rate * 100.0,
        cfg.max_entry_price,
        cfg.min_edge * 100.0,
        cfg.position_size_usdc
    )
}

pub fn is_entry_window_open(market: &StrategyMarket, now: DateTime<Utc>, cfg: &Config) -> bool {
    let start = market.start_date + Duration::seconds(cfg.entry_delay_secs);
    let end = market.start_date + Duration::seconds(cfg.entry_window_secs);
    now >= start && now <= end
}

pub fn build_strategy_input(
    market: &StrategyMarket,
    quote: QuoteSnapshot,
    now: DateTime<Utc>,
    cfg: &Config,
) -> StrategyInput {
    StrategyInput {
        market_id: market.id.clone(),
        market_ticker: market.display_label(),
        quote,
        now_ts: now.timestamp(),
        close_ts: market.end_date.timestamp(),
        seconds_to_close: (market.end_date - now).num_milliseconds() as f64 / 1000.0,
        is_candidate: market.is_strategy_candidate(cfg.target_hour_utc),
        market_closed: market.closed,
        liquidity_num: market.liquidity_num,
    }
}
