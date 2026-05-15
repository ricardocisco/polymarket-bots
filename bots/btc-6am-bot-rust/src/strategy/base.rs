use crate::types::{ExecutionUpdate, QuoteSnapshot, SignalDecision, StrategyInput};

pub trait Strategy: Send {
    fn name(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn decide(&mut self, input: &StrategyInput) -> SignalDecision;

    fn on_order_submitted(&mut self, _update: &ExecutionUpdate, _quote: &QuoteSnapshot) {}
}
