use std::time::Duration;

use alloy_primitives::U256;
use anyhow::Result;
use futures::StreamExt;
use polymarket_client_sdk::clob::ws::types::response::{BookUpdate, PriceChange};
use polymarket_client_sdk::clob::ws::Client as ClobWsClient;
use rust_decimal::Decimal;
use tokio::sync::watch;
use tokio::time::sleep;

use crate::types::QuoteSnapshot;

const RECONNECT_DELAY: Duration = Duration::from_secs(3);

pub struct OrderbookFeed {
    up_u256: U256,
    down_u256: U256,
    _up_rx: watch::Receiver<Option<TopOfBook>>,
    _down_rx: watch::Receiver<Option<TopOfBook>>,
    quote_rx: watch::Receiver<Option<QuoteSnapshot>>,
}

impl OrderbookFeed {
    pub fn start(yes_token_id: &str, no_token_id: &str) -> Result<Self> {
        let up_u256: U256 = yes_token_id
            .parse()
            .map_err(|_| anyhow::anyhow!("yes_token_id invalido: {yes_token_id}"))?;
        let down_u256: U256 = no_token_id
            .parse()
            .map_err(|_| anyhow::anyhow!("no_token_id invalido: {no_token_id}"))?;

        let (up_tx, up_rx) = watch::channel::<Option<TopOfBook>>(None);
        let (down_tx, down_rx) = watch::channel::<Option<TopOfBook>>(None);
        let (quote_tx, quote_rx) = watch::channel::<Option<QuoteSnapshot>>(None);

        tokio::spawn(async move {
            loop {
                clear_all(&up_tx, &down_tx, &quote_tx);

                match ws_loop(up_u256, down_u256, &up_tx, &down_tx, &quote_tx).await {
                    Ok(()) => eprintln!("[orderbook_ws] stream encerrado, reconectando..."),
                    Err(e) => eprintln!("[orderbook_ws] erro: {e:#}"),
                }

                clear_all(&up_tx, &down_tx, &quote_tx);
                sleep(RECONNECT_DELAY).await;
            }
        });

        Ok(Self {
            up_u256,
            down_u256,
            _up_rx: up_rx,
            _down_rx: down_rx,
            quote_rx,
        })
    }

    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.quote_rx.borrow().is_some()
    }

    pub fn subscribe(&self) -> watch::Receiver<Option<QuoteSnapshot>> {
        self.quote_rx.clone()
    }

    #[must_use]
    pub fn get_quote(&self) -> Option<QuoteSnapshot> {
        *self.quote_rx.borrow()
    }

    pub async fn wait_ready(&self, timeout: Duration) -> Result<()> {
        let mut rx = self.subscribe();

        if rx.borrow().is_some() {
            return Ok(());
        }

        tokio::time::timeout(timeout, async move {
            loop {
                rx.changed()
                    .await
                    .map_err(|_| anyhow::anyhow!("stream do orderbook encerrado"))?;
                if rx.borrow().is_some() {
                    return Ok(());
                }
            }
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "timeout aguardando snapshot inicial do orderbook (yes={}, no={})",
                self.up_u256,
                self.down_u256,
            )
        })?
    }
}

async fn ws_loop(
    up_u256: U256,
    down_u256: U256,
    up_tx: &watch::Sender<Option<TopOfBook>>,
    down_tx: &watch::Sender<Option<TopOfBook>>,
    quote_tx: &watch::Sender<Option<QuoteSnapshot>>,
) -> Result<()> {
    let ws_client = ClobWsClient::default();
    let book_stream = ws_client.subscribe_orderbook(vec![up_u256, down_u256])?;
    let price_stream = ws_client.subscribe_prices(vec![up_u256, down_u256])?;
    let mut book_stream = Box::pin(book_stream);
    let mut price_stream = Box::pin(price_stream);
    let mut book_closed = false;
    let mut price_closed = false;

    loop {
        tokio::select! {
            result = book_stream.next(), if !book_closed => {
                match result {
                    Some(result) => route_book_update(result?, up_u256, down_u256, up_tx, down_tx, quote_tx),
                    None => book_closed = true,
                }
            }
            result = price_stream.next(), if !price_closed => {
                match result {
                    Some(result) => route_price_change(result?, up_u256, down_u256, up_tx, down_tx, quote_tx),
                    None => price_closed = true,
                }
            }
            else => break,
        }

        if book_closed && price_closed {
            break;
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct TopOfBook {
    best_bid: Option<f64>,
    best_ask: Option<f64>,
    ts: i64,
}

fn route_book_update(
    update: BookUpdate,
    up_u256: U256,
    down_u256: U256,
    up_tx: &watch::Sender<Option<TopOfBook>>,
    down_tx: &watch::Sender<Option<TopOfBook>>,
    quote_tx: &watch::Sender<Option<QuoteSnapshot>>,
) {
    let tx = if update.asset_id == up_u256 {
        up_tx
    } else if update.asset_id == down_u256 {
        down_tx
    } else {
        return;
    };

    if tx.borrow().is_some() {
        return;
    }

    let snapshot = TopOfBook {
        best_bid: update.bids.first().and_then(|l| to_price(&l.price)),
        best_ask: update.asks.first().and_then(|l| to_price(&l.price)),
        ts: update.timestamp,
    };

    if snapshot.best_bid.is_none() && snapshot.best_ask.is_none() {
        return;
    }

    let _ = tx.send(Some(snapshot));
    publish_quote(up_tx, down_tx, quote_tx);
}

fn route_price_change(
    update: PriceChange,
    up_u256: U256,
    down_u256: U256,
    up_tx: &watch::Sender<Option<TopOfBook>>,
    down_tx: &watch::Sender<Option<TopOfBook>>,
    quote_tx: &watch::Sender<Option<QuoteSnapshot>>,
) {
    for change in update.price_changes {
        if change.asset_id == up_u256 {
            merge_top_of_book(
                up_tx,
                change.best_bid.as_ref(),
                change.best_ask.as_ref(),
                update.timestamp,
            );
        } else if change.asset_id == down_u256 {
            merge_top_of_book(
                down_tx,
                change.best_bid.as_ref(),
                change.best_ask.as_ref(),
                update.timestamp,
            );
        }
    }

    publish_quote(up_tx, down_tx, quote_tx);
}

fn merge_top_of_book(
    tx: &watch::Sender<Option<TopOfBook>>,
    best_bid: Option<&Decimal>,
    best_ask: Option<&Decimal>,
    ts: i64,
) {
    let bid = best_bid.and_then(to_price);
    let ask = best_ask.and_then(to_price);

    if bid.is_none() && ask.is_none() {
        return;
    }

    let mut next = (*tx.borrow()).unwrap_or(TopOfBook {
        best_bid: None,
        best_ask: None,
        ts,
    });

    if let Some(bid) = bid {
        next.best_bid = Some(bid);
    }
    if let Some(ask) = ask {
        next.best_ask = Some(ask);
    }
    next.ts = ts;

    let _ = tx.send(Some(next));
}

fn to_price(price: &Decimal) -> Option<f64> {
    let value = price.to_string().parse::<f64>().ok()?;
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Some(value)
    } else {
        None
    }
}

fn publish_quote(
    up_tx: &watch::Sender<Option<TopOfBook>>,
    down_tx: &watch::Sender<Option<TopOfBook>>,
    quote_tx: &watch::Sender<Option<QuoteSnapshot>>,
) {
    let up = *up_tx.borrow();
    let down = *down_tx.borrow();

    let quote = match (up, down) {
        (Some(up), Some(down)) => Some(QuoteSnapshot {
            yes_bid: up.best_bid,
            yes_ask: up.best_ask,
            no_bid: down.best_bid,
            no_ask: down.best_ask,
            ts: up.ts.max(down.ts),
        }),
        _ => None,
    };

    let _ = quote_tx.send(quote);
}

fn clear_all(
    up_tx: &watch::Sender<Option<TopOfBook>>,
    down_tx: &watch::Sender<Option<TopOfBook>>,
    quote_tx: &watch::Sender<Option<QuoteSnapshot>>,
) {
    let _ = up_tx.send(None);
    let _ = down_tx.send(None);
    let _ = quote_tx.send(None);
}
