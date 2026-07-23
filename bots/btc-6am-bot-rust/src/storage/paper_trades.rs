use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::feed::GammaMarketsFeed;
use crate::types::{
    OpenTradeGroup, PaperTradeRecord, PaperTradeStatus, SignalDecision, StrategyMarket,
    TradeDirection,
};
use anyhow::{Context, Result};

pub struct PaperTradeStore {
    path: PathBuf,
}

impl PaperTradeStore {
    pub async fn connect(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("falha ao criar {}", parent.display()))?;
        }
        if !path.exists() {
            fs::write(&path, "[]")
                .with_context(|| format!("falha ao inicializar {}", path.display()))?;
        }
        Ok(Self { path })
    }

    pub async fn insert_open_trade(
        &self,
        market: &StrategyMarket,
        token_id: &str,
        submitted_at: i64,
        side: TradeDirection,
        entry_price_cents: u32,
        _seconds_to_close: f64,
        decision: &SignalDecision,
        dry_run: bool,
        strategy_name: &str,
        strategy_version: &str,
    ) -> Result<()> {
        let mut trades = self.load_trades()?;
        if trades.iter().any(|trade| {
            trade.market_id == market.id
                && trade.status == PaperTradeStatus::Open
                && trade.side == side
        }) {
            return Ok(());
        }

        trades.push(PaperTradeRecord {
            id: format!("{}-{}", market.id, submitted_at),
            strategy_name: strategy_name.into(),
            strategy_version: strategy_version.into(),
            market_id: market.id.clone(),
            market_ticker: market.display_label(),
            market_question: market.question.clone(),
            token_id: token_id.into(),
            close_ts: market.end_date.timestamp(),
            submitted_at,
            side,
            size: decision.size,
            entry_price_cents,
            decision_reason: decision.reason.clone(),
            diagnostics: decision.diagnostics.clone(),
            status: PaperTradeStatus::Open,
            winner_side: None,
            final_price: None,
            pnl_cents: None,
            settled_at: None,
            dry_run,
            created_at: now_ts(),
        });
        self.save_trades(&trades)
    }

    pub async fn fetch_open_groups(&self) -> Result<Vec<OpenTradeGroup>> {
        let trades = self.load_trades()?;
        let mut groups = std::collections::BTreeMap::<String, OpenTradeGroup>::new();

        for trade in trades
            .into_iter()
            .filter(|trade| trade.status == PaperTradeStatus::Open)
        {
            let entry = groups
                .entry(trade.market_id.clone())
                .or_insert(OpenTradeGroup {
                    market_id: trade.market_id.clone(),
                    market_ticker: trade.market_ticker.clone(),
                    close_ts: trade.close_ts,
                    count: 0,
                });
            entry.count += 1;
        }

        Ok(groups.into_values().collect())
    }

    pub async fn settle_open_trades(
        &self,
        market_id: &str,
        winner_side: TradeDirection,
        final_price: f64,
        settled_at: i64,
    ) -> Result<u64> {
        let mut trades = self.load_trades()?;
        let mut updated = 0u64;

        for trade in &mut trades {
            if trade.market_id != market_id || trade.status != PaperTradeStatus::Open {
                continue;
            }

            let pnl_cents = if trade.side == winner_side {
                (100i64 - trade.entry_price_cents as i64) * trade.size as i64
            } else {
                -(trade.entry_price_cents as i64) * trade.size as i64
            };

            trade.status = PaperTradeStatus::Settled;
            trade.winner_side = Some(winner_side);
            trade.final_price = Some(final_price);
            trade.pnl_cents = Some(pnl_cents);
            trade.settled_at = Some(settled_at);
            updated += 1;
        }

        self.save_trades(&trades)?;
        Ok(updated)
    }

    pub async fn has_open_trade(&self, market_id: &str) -> Result<bool> {
        Ok(self
            .load_trades()?
            .into_iter()
            .any(|trade| trade.market_id == market_id && trade.status == PaperTradeStatus::Open))
    }

    pub async fn has_trade(&self, market_id: &str) -> Result<bool> {
        Ok(self
            .load_trades()?
            .into_iter()
            .any(|trade| trade.market_id == market_id))
    }

    pub async fn count_trades_since(&self, submitted_at: i64) -> Result<usize> {
        Ok(self
            .load_trades()?
            .into_iter()
            .filter(|trade| trade.submitted_at >= submitted_at)
            .count())
    }

    pub async fn list_open_trades(&self) -> Result<Vec<PaperTradeRecord>> {
        Ok(self
            .load_trades()?
            .into_iter()
            .filter(|trade| trade.status == PaperTradeStatus::Open)
            .collect())
    }

    fn load_trades(&self) -> Result<Vec<PaperTradeRecord>> {
        let raw = fs::read_to_string(&self.path)
            .with_context(|| format!("falha ao ler {}", self.path.display()))?;
        serde_json::from_str(&raw)
            .with_context(|| format!("JSON de trades corrompido em {}", self.path.display()))
    }

    fn save_trades(&self, trades: &[PaperTradeRecord]) -> Result<()> {
        let raw = serde_json::to_string_pretty(trades)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, raw).with_context(|| format!("falha ao escrever {}", tmp.display()))?;
        replace_file(&tmp, &self.path)
    }
}

pub async fn reconcile_due_trades(
    store: &PaperTradeStore,
    gamma: &GammaMarketsFeed,
    now_ts: i64,
) -> Result<u64> {
    let groups = store.fetch_open_groups().await?;
    let mut settled = 0u64;

    for group in groups {
        if group.close_ts > now_ts {
            continue;
        }
        let Some(market) = gamma.fetch_market_by_id(&group.market_id).await? else {
            continue;
        };

        let final_up = market
            .resolved_price_for_direction(TradeDirection::Up)
            .unwrap_or(0.0);
        let final_down = market
            .resolved_price_for_direction(TradeDirection::Down)
            .unwrap_or(0.0);

        let winner_side = if final_up >= final_down {
            TradeDirection::Up
        } else {
            TradeDirection::Down
        };

        settled += store
            .settle_open_trades(
                &group.market_id,
                winner_side,
                final_up.max(final_down),
                now_ts,
            )
            .await?;
    }

    Ok(settled)
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn replace_file(tmp: &Path, destination: &Path) -> Result<()> {
    match fs::rename(tmp, destination) {
        Ok(()) => Ok(()),
        Err(_) if destination.exists() => {
            let backup = destination.with_extension("json.bak");
            if backup.exists() {
                fs::remove_file(&backup)?;
            }
            fs::rename(destination, &backup)?;
            if let Err(err) = fs::rename(tmp, destination) {
                let _ = fs::rename(&backup, destination);
                return Err(err).context("falha ao instalar novo ledger de trades");
            }
            fs::remove_file(backup)?;
            Ok(())
        }
        Err(err) => Err(err).context("falha ao substituir ledger de trades"),
    }
}

#[allow(dead_code)]
fn _ensure_parent_exists(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}
