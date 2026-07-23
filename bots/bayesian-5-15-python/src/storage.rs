use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::types::{
    OpenTradeGroup, PaperTradeRecord, PaperTradeStatus, SignalDecision, StrategyMarket, TradeSide,
};

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
        submitted_at: i64,
        side: TradeSide,
        entry_price_cents: u32,
        underlying_price_usd: f64,
        seconds_to_close: f64,
        decision: &SignalDecision,
        dry_run: bool,
        order_id: Option<String>,
        strategy_name: &str,
        strategy_version: &str,
    ) -> Result<()> {
        let mut trades = self.load_trades()?;
        if trades.iter().any(|trade| {
            trade.market_slug == market.slug
                && trade.status == PaperTradeStatus::Open
                && trade.side == side
        }) {
            return Ok(());
        }

        trades.push(PaperTradeRecord {
            id: format!("{}-{}-{}", market.slug, side, submitted_at),
            strategy_name: strategy_name.into(),
            strategy_version: strategy_version.into(),
            market_slug: market.slug.clone(),
            market_ticker: market.display_label(),
            asset: market.config.asset.clone(),
            interval_minutes: market.config.duration_minutes,
            token_id: market.token_id(side).to_owned(),
            strike_price: market.strike_price,
            close_ts: market.end_time.timestamp(),
            submitted_at,
            side,
            size: decision.size,
            entry_price_cents,
            stake_usdc: decision.stake_usdc.unwrap_or_default(),
            underlying_price_usd,
            seconds_to_close,
            decision_reason: decision.reason.clone(),
            diagnostics: decision.diagnostics.clone(),
            status: PaperTradeStatus::Open,
            winner_side: None,
            final_price_usd: None,
            pnl_cents: None,
            settled_at: None,
            dry_run,
            order_id,
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
                .entry(trade.market_slug.clone())
                .or_insert(OpenTradeGroup {
                    market_slug: trade.market_slug.clone(),
                    close_ts: trade.close_ts,
                    asset: trade.asset.clone(),
                    interval_minutes: trade.interval_minutes,
                    count: 0,
                });
            entry.count += 1;
        }
        Ok(groups.into_values().collect())
    }

    pub async fn settle_open_trades(
        &self,
        market_slug: &str,
        winner_side: TradeSide,
        final_price_usd: f64,
        settled_at: i64,
    ) -> Result<u64> {
        let mut trades = self.load_trades()?;
        let mut updated = 0;

        for trade in &mut trades {
            if trade.market_slug != market_slug || trade.status != PaperTradeStatus::Open {
                continue;
            }

            let pnl_cents = if trade.side == winner_side {
                (100i64 - trade.entry_price_cents as i64) * trade.size as i64
            } else {
                -(trade.entry_price_cents as i64) * trade.size as i64
            };
            trade.status = PaperTradeStatus::Settled;
            trade.winner_side = Some(winner_side);
            trade.final_price_usd = Some(final_price_usd);
            trade.pnl_cents = Some(pnl_cents);
            trade.settled_at = Some(settled_at);
            updated += 1;
        }

        self.save_trades(&trades)?;
        Ok(updated)
    }

    pub async fn has_open_trade(&self, market_slug: &str) -> Result<bool> {
        Ok(self.load_trades()?.into_iter().any(|trade| {
            trade.market_slug == market_slug && trade.status == PaperTradeStatus::Open
        }))
    }

    pub async fn list_open_trades(&self) -> Result<Vec<PaperTradeRecord>> {
        Ok(self
            .load_trades()?
            .into_iter()
            .filter(|trade| trade.status == PaperTradeStatus::Open)
            .collect())
    }

    pub async fn settled_results(&self) -> Result<Vec<(i64, i64)>> {
        Ok(self
            .load_trades()?
            .into_iter()
            .filter_map(|trade| Some((trade.pnl_cents?, trade.settled_at?)))
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
