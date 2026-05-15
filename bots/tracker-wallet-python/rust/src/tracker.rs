use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use mongodb::Database;
use poise::serenity_prelude as serenity;
use serenity::builder::{CreateEmbed, CreateEmbedFooter, CreateMessage};
use serenity::http::Http;
use serenity::model::id::ChannelId;
use serenity::model::Timestamp;
use tokio::time::{self, Duration};
use tracing::{debug, error, info, warn};

use crate::models::{Subscription, Wallet};
use crate::polymarket::{fetch_closed_position, fetch_user_activity, PolyActivity};

const CHECK_INTERVAL_SECS: u64 = 15;
const SENT_CACHE_TTL_SECS: i64 = 120;

// ─── Entry Tier ──────────────────────────────────────────────────────────────

const ENTRY_HIGH_USD: f64 = 1000.0;
const ENTRY_MEDIUM_USD: f64 = 500.0;
const ENTRY_LOW_USD: f64 = 10.0;

struct EntryTier {
    label: &'static str,
    emoji: &'static str,
}

fn get_entry_tier(value_usd: f64) -> EntryTier {
    if value_usd >= ENTRY_HIGH_USD {
        EntryTier {
            label: "ALTA",
            emoji: "🚀",
        }
    } else if value_usd >= ENTRY_MEDIUM_USD {
        EntryTier {
            label: "MEDIANA",
            emoji: "📈",
        }
    } else if value_usd < ENTRY_LOW_USD {
        EntryTier {
            label: "BAIXA",
            emoji: "🧊",
        }
    } else {
        EntryTier {
            label: "MÉDIA",
            emoji: "➖",
        }
    }
}

// ─── Loop principal ───────────────────────────────────────────────────────────

pub async fn start_tracker_loop(http: Arc<Http>, db: Database, http_client: reqwest::Client) {
    info!("🔥 TRACKER INICIADO");
    info!("⏱️  Intervalo: {}s", CHECK_INTERVAL_SECS);

    let mut interval = time::interval(Duration::from_secs(CHECK_INTERVAL_SECS));

    // Cache de dedup: id do trade → timestamp (evita mensagens duplicadas)
    let mut sent_messages: HashMap<String, i64> = HashMap::new();

    loop {
        interval.tick().await;

        let now = Utc::now().timestamp();

        // Limpa entradas antigas do cache
        sent_messages.retain(|_, ts| now - *ts < SENT_CACHE_TTL_SECS);

        // Carrega todas as carteiras monitoradas
        let wallets_col = db.collection::<Wallet>("wallets");
        let wallets: Vec<Wallet> = match wallets_col.find(doc! {}).await {
            Ok(cursor) => match cursor.try_collect().await {
                Ok(v) => v,
                Err(e) => {
                    error!("❌ Erro ao coletar wallets: {e}");
                    continue;
                }
            },
            Err(e) => {
                error!("❌ Erro ao buscar wallets: {e}");
                continue;
            }
        };

        debug!("📊 Carteiras monitoradas: {}", wallets.len());

        if wallets.is_empty() {
            debug!("⚠️ Nenhuma carteira cadastrada");
            continue;
        }

        for wallet in wallets {
            if !wallet.address.starts_with("0x") {
                warn!("⚠️ Endereço inválido: {}", wallet.address);
                continue;
            }

            // Verifica se há inscrições ativas para esta carteira
            let subs_col = db.collection::<Subscription>("subscriptions");
            let subs: Vec<Subscription> = match subs_col
                .find(doc! { "walletAddress": &wallet.address })
                .await
            {
                Ok(cursor) => match cursor.try_collect().await {
                    Ok(v) => v,
                    Err(e) => {
                        error!("❌ Erro ao coletar subscriptions: {e}");
                        continue;
                    }
                },
                Err(e) => {
                    error!("❌ Erro ao buscar subscriptions: {e}");
                    continue;
                }
            };

            if subs.is_empty() {
                debug!("⚠️ Carteira {} sem inscrições ativas", &wallet.address[..8]);
                continue;
            }

            debug!(
                "🔍 Checando {}... ({} canal(is))",
                &wallet.address[..8],
                subs.len()
            );

            // Busca atividades desde o último timestamp via API oficial
            let activities =
                fetch_user_activity(&http_client, &wallet.address, wallet.last_timestamp).await;

            if activities.is_empty() {
                continue;
            }

            info!(
                "🚨 MUDANÇA DETECTADA: {} operação(ões) para {}",
                activities.len(),
                &wallet.address[..8]
            );

            // Atualiza lastTimestamp para o maior timestamp recebido
            let max_ts = activities.iter().map(|a| a.timestamp).max().unwrap_or(now);
            if let Err(e) = wallets_col
                .update_one(
                    doc! { "address": &wallet.address },
                    doc! { "$set": { "lastTimestamp": max_ts } },
                )
                .await
            {
                error!("❌ Erro ao atualizar lastTimestamp: {e}");
            }

            // Busca username uma vez por carteira via campo da API
            // (display_name já vem em cada trade — usado no build_embed)

            for trade in &activities {
                // Dedup por transaction hash
                if sent_messages.contains_key(&trade.id) {
                    debug!(
                        "⏭️ Pulando duplicata: {}",
                        &trade.id[..20.min(trade.id.len())]
                    );
                    continue;
                }

                // Para SELL: busca PNL via /closed-positions
                let realized_pnl = if trade.side == "SELL" && !trade.condition_id.is_empty() {
                    fetch_closed_position(&http_client, &wallet.address, &trade.condition_id).await
                } else {
                    None
                };

                let embed = build_embed(trade, &wallet.address, realized_pnl);

                let mut sent_count: u32 = 0;

                for sub in &subs {
                    // ─── Filtros por subscription ─────────────────────────
                    if let Some(ref filters) = sub.filters {
                        // Filtro de valor mínimo (USD)
                        if let Some(min_usd) = filters.min_usd {
                            if min_usd > 0.0 && trade.usdc_size < min_usd {
                                continue;
                            }
                        }
                        // Filtro de palavras-chave no título do mercado
                        if let Some(ref keywords) = filters.keywords {
                            if !keywords.is_empty() {
                                let title_lower = trade.market_title.to_lowercase();
                                if !keywords
                                    .iter()
                                    .any(|k| title_lower.contains(&k.to_lowercase()))
                                {
                                    continue;
                                }
                            }
                        }
                    }
                    // ──────────────────────────────────────────────────────

                    let channel_id_u64: u64 = match sub.channel_id.parse() {
                        Ok(v) => v,
                        Err(_) => {
                            error!("❌ channelId inválido: {}", sub.channel_id);
                            continue;
                        }
                    };

                    let channel = ChannelId::new(channel_id_u64);

                    match channel
                        .send_message(&http, CreateMessage::new().add_embed(embed.clone()))
                        .await
                    {
                        Ok(_) => {
                            sent_count += 1;
                            info!("✓ Enviado para canal {}", sub.channel_id);
                        }
                        Err(e) => {
                            error!("❌ Erro ao enviar para {}: {e}", sub.channel_id);
                        }
                    }
                }

                if sent_count > 0 {
                    sent_messages.insert(trade.id.clone(), now);
                    info!("✅ Mensagem enviada para {sent_count} canal(is)");
                } else {
                    warn!("⚠️ Nenhum canal recebeu a mensagem (filtros ou erros)");
                }

                // Delay entre mensagens para evitar rate limit do Discord
                tokio::time::sleep(Duration::from_millis(500)).await;
            }

            // Pausa entre carteiras para evitar rate limit da Polymarket API
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        debug!("✓ Ciclo de verificação concluído");
    }
}

// ─── Construção do embed Discord ──────────────────────────────────────────────

fn build_embed(
    trade: &PolyActivity,
    wallet_address: &str,
    realized_pnl: Option<f64>,
) -> CreateEmbed {
    let (type_label, color, emoji) = match trade.side.as_str() {
        "BUY" => ("COMPROU", 0x00ff00u32, "🟢"),
        "SELL" => ("VENDEU", 0xff0000u32, "🔴"),
        _ => ("OPERAÇÃO", 0x808080u32, "📊"),
    };

    let market_url = if trade.market_url.is_empty() {
        format!("https://polymarket.com/profile/{wallet_address}")
    } else {
        trade.market_url.clone()
    };

    let market_title_linked = if trade.market_url.is_empty() {
        trade.market_title.clone()
    } else {
        format!("[{}]({})", trade.market_title, trade.market_url)
    };

    let addr_short = format!(
        "{}...{}",
        &wallet_address[..6],
        &wallet_address[wallet_address.len() - 4..]
    );

    let mut description = String::new();
    // Usa display_name vindo direto da API (é o pseudonym/name do trader)
    if let Some(ref name) = trade.display_name {
        description.push_str(&format!("**Trader:** @{name}\n"));
    }
    description.push_str(&format!("**Mercado:** {market_title_linked}\n"));
    description.push_str(&format!("**Posição:** {}\n", trade.outcome));
    description.push_str(&format!(
        "**Carteira:** [`{addr_short}`](https://polymarket.com/profile/{wallet_address})"
    ));

    let tier = get_entry_tier(trade.usdc_size);

    let mut embed = CreateEmbed::new()
        .title(format!("{emoji} {type_label}"))
        .url(&market_url)
        .color(color)
        .description(&description)
        .field(
            format!("{} Entrada", tier.emoji),
            format!("{} (${:.2})", tier.label, trade.usdc_size),
            true,
        )
        .field("💵 Preço", format!("${:.3}", trade.price), true)
        .field("📊 Shares", format!("{:.1}", trade.shares), true)
        .field("💰 Valor", format!("${:.2}", trade.usdc_size), true)
        .footer(CreateEmbedFooter::new("Polymarket Tracker"));

    // PNL: exibe somente em SELLs com lucro positivo
    if trade.side == "SELL" {
        if let Some(pnl) = realized_pnl {
            if pnl > 0.0 {
                embed = embed.field("🏆 Profit", format!("${pnl:.2}"), true);
            } else if pnl < 0.0 {
                embed = embed.field("📉 Loss", format!("-${:.2}", pnl.abs()), true);
            }
        }
    }

    if let Ok(ts) = Timestamp::from_unix_timestamp(trade.timestamp) {
        embed = embed.timestamp(ts);
    }

    if !trade.market_image_url.is_empty() {
        embed = embed.image(&trade.market_image_url);
    }

    embed
}
