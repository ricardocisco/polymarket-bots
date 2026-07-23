use anyhow::Result;
use chrono::Utc;
use tracing::warn;

use crate::feed::{BinanceFeed, GammaMarketsFeed};
use crate::storage::PaperTradeStore;
use crate::types::MarketConfig;

pub async fn reconcile_due_trades(
    store: &PaperTradeStore,
    gamma: &GammaMarketsFeed,
    binance: &BinanceFeed,
) -> Result<u64> {
    let now_ts = Utc::now().timestamp();
    let mut settled = 0u64;

    for group in store.fetch_open_groups().await? {
        if group.close_ts > now_ts {
            continue;
        }
        let market_cfg = MarketConfig {
            asset: group.asset.clone(),
            duration_minutes: group.interval_minutes,
            binance_symbol: format!("{}USDT", group.asset.to_ascii_uppercase()),
        };
        let market = gamma
            .fetch_market_by_slug(&group.market_slug, market_cfg)
            .await?
            .filter(|market| market.closed);

        let final_price = binance
            .price_at(
                &format!("{}USDT", group.asset.to_ascii_uppercase()),
                group.close_ts,
            )
            .await
            .ok()
            .flatten();

        let (winner_side, final_underlying) = if let Some(market) = market {
            let final_underlying = final_price.unwrap_or(market.strike_price);
            // A resolucao oficial do mercado e a unica fonte autorizada para o
            // vencedor. Binance fica apenas como metadado de preco final.
            let side = market.winner_from_outcome_prices();
            (side, final_underlying)
        } else {
            (None, final_price.unwrap_or(0.0))
        };

        let Some(winner_side) = winner_side else {
            warn!(
                "{} | nao foi possivel determinar vencedor",
                group.market_slug
            );
            continue;
        };

        settled += store
            .settle_open_trades(&group.market_slug, winner_side, final_underlying, now_ts)
            .await?;
    }

    Ok(settled)
}
