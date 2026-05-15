use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use poise::serenity_prelude as serenity;
use serenity::builder::{CreateEmbed, CreateEmbedFooter};
use tracing::info;

use crate::models::{Subscription, Wallet};
use crate::polymarket::{get_username_from_address, resolve_user};

// ─── Estado compartilhado do bot ─────────────────────────────────────────────

pub struct BotData {
    pub db: mongodb::Database,
    pub http_client: reqwest::Client,
}

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Context<'a> = poise::Context<'a, BotData, Error>;

// ─── Comandos slash ───────────────────────────────────────────────────────────

/// Começa a rastrear uma carteira Polymarket neste canal
#[poise::command(slash_command, guild_only)]
pub async fn track(
    ctx: Context<'_>,
    #[description = "Endereço 0x ou @username da carteira"] input: String,
) -> Result<(), Error> {
    ctx.defer().await?;

    let db = &ctx.data().db;
    let http_client = &ctx.data().http_client;

    info!("🔍 Tentando rastrear: {input}");

    let address = match resolve_user(http_client, &input).await {
        Some(a) => a,
        None => {
            ctx.say(
                "❌ Não consegui encontrar o endereço.\n\
                 Certifique-se de que:\n\
                 • O username está correto (ex: @nickname)\n\
                 • Ou use o endereço 0x completo da carteira",
            )
            .await?;
            return Ok(());
        }
    };

    let wallets_col = db.collection::<Wallet>("wallets");
    let subs_col = db.collection::<Subscription>("subscriptions");
    let channel_id = ctx.channel_id().get().to_string();

    // Cria carteira se ainda não existe
    if wallets_col
        .find_one(doc! { "address": &address })
        .await?
        .is_none()
    {
        let now_secs = Utc::now().timestamp();
        wallets_col
            .insert_one(Wallet {
                id: None,
                address: address.clone(),
                last_timestamp: now_secs,
            })
            .await?;
        info!("🆕 Nova carteira criada: {address}");
    } else {
        info!("♻️ Carteira já existe: {address}");
    }

    // Verifica duplicata de subscription para este canal
    if subs_col
        .find_one(doc! {
            "channelId": &channel_id,
            "walletAddress": &address
        })
        .await?
        .is_some()
    {
        ctx.say(format!(
            "⚠️ Este canal já está rastreando a carteira:\n\
             [`{}...{}`](https://polymarket.com/profile/{address})",
            &address[..6],
            &address[address.len() - 4..]
        ))
        .await?;
        return Ok(());
    }

    subs_col
        .insert_one(Subscription {
            id: None,
            channel_id: channel_id.clone(),
            wallet_address: address.clone(),
            user_id: Some(ctx.author().id.get().to_string()),
            filters: None,
        })
        .await?;

    info!(
        "✅ Inscrição criada: Canal {channel_id} → {}",
        &address[..8]
    );

    ctx.say(format!(
        "✅ **Rastreamento Ativado!**\n\n\
         📡 **Carteira:** [`{}...{}`](https://polymarket.com/profile/{address})\n\
         ⏰ Você receberá alertas de **novas operações** (compras e vendas).\n\n\
         💡 **Como funciona:** O bot verifica a cada 15s via API oficial da Polymarket.",
        &address[..6],
        &address[address.len() - 4..]
    ))
    .await?;

    Ok(())
}

/// Para de rastrear uma carteira neste canal
#[poise::command(slash_command, guild_only)]
pub async fn untrack(
    ctx: Context<'_>,
    #[description = "Endereço 0x ou @username da carteira"] input: String,
) -> Result<(), Error> {
    ctx.defer().await?;

    let db = &ctx.data().db;
    let http_client = &ctx.data().http_client;
    let channel_id = ctx.channel_id().get().to_string();

    let address = match resolve_user(http_client, &input).await {
        Some(a) => a,
        None => {
            ctx.say("❌ Não encontrei essa carteira. Use o mesmo formato usado no `/track`.")
                .await?;
            return Ok(());
        }
    };

    let subs_col = db.collection::<Subscription>("subscriptions");

    match subs_col
        .find_one_and_delete(doc! {
            "channelId": &channel_id,
            "walletAddress": &address
        })
        .await?
    {
        None => {
            ctx.say(format!(
                "⚠️ Este canal não estava rastreando:\n`{address}`"
            ))
            .await?;
        }
        Some(_) => {
            // Garbage collection: remove carteira se não tem mais inscrições
            let remaining = subs_col
                .count_documents(doc! { "walletAddress": &address })
                .await?;

            if remaining == 0 {
                let wallets_col = db.collection::<Wallet>("wallets");
                wallets_col
                    .find_one_and_delete(doc! { "address": &address })
                    .await?;
                info!("🗑️ Carteira {} removida (0 inscritos)", &address[..8]);
            }

            ctx.say(format!(
                "✅ **Rastreamento Removido!**\n\n\
                 Este canal não receberá mais alertas de:\n\
                 `{}...{}`",
                &address[..6],
                &address[address.len() - 4..]
            ))
            .await?;
        }
    }

    Ok(())
}

/// Lista todas as carteiras rastreadas neste canal
#[poise::command(slash_command, guild_only, rename = "list")]
pub async fn list_wallets(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer().await?;

    let db = &ctx.data().db;
    let http_client = &ctx.data().http_client;
    let channel_id = ctx.channel_id().get().to_string();

    let subs_col = db.collection::<Subscription>("subscriptions");
    let wallets_col = db.collection::<Wallet>("wallets");

    let subs: Vec<Subscription> = subs_col
        .find(doc! { "channelId": &channel_id })
        .await?
        .try_collect()
        .await?;

    if subs.is_empty() {
        ctx.say(
            "ℹ️ **Nenhuma carteira rastreada neste canal.**\n\n\
             Use `/track <endereço>` para começar a rastrear.",
        )
        .await?;
        return Ok(());
    }

    let mut embed = CreateEmbed::new()
        .title("📋 Carteiras Rastreadas")
        .description(format!(
            "Este canal está rastreando **{}** carteira(s):",
            subs.len()
        ))
        .color(0x5865f2u32);

    for sub in &subs {
        let wallet = wallets_col
            .find_one(doc! { "address": &sub.wallet_address })
            .await?;

        let username =
            get_username_from_address(http_client, &sub.wallet_address).await;
        let display_name = username.map(|u| format!("@{u}"));

        let mut field_name = String::new();
        if let Some(ref dn) = display_name {
            field_name.push_str(&format!("User: {dn}\n"));
        }
        field_name.push_str(&format!("Carteira: {}", sub.wallet_address));

        let last_check = if let Some(ref w) = wallet {
            // Normaliza: wallets migradas podem ter ms
            let ts_secs = if w.last_timestamp > 1_000_000_000_000 {
                w.last_timestamp / 1000
            } else {
                w.last_timestamp
            };
            chrono::DateTime::from_timestamp(ts_secs, 0)
                .map(|dt| dt.format("%d/%m/%Y %H:%M:%S").to_string())
                .unwrap_or_else(|| "Nunca".into())
        } else {
            "Nunca".to_string()
        };

        let mut field_value = format!(
            "[Ver perfil](https://polymarket.com/profile/{}) • Última checagem: {last_check}",
            sub.wallet_address
        );

        if let Some(ref uid) = sub.user_id {
            field_value.push_str(&format!("\n👤 Tracking criado por: <@{uid}>"));
        }

        embed = embed.field(field_name, field_value, false);
    }

    ctx.send(poise::CreateReply::default().embed(embed)).await?;

    Ok(())
}

/// Configura filtros de notificação para uma carteira rastreada neste canal
#[poise::command(slash_command, guild_only)]
pub async fn filter(
    ctx: Context<'_>,
    #[description = "Endereço 0x ou @username da carteira"] input: String,
    #[description = "Palavra-chave para filtrar mercados (ex: Trump)"] keyword: Option<String>,
    #[description = "Valor mínimo em USD para notificar (0 = sem filtro)"] min_usd: Option<f64>,
    #[description = "Limpar todos os filtros desta carteira"] limpar: Option<bool>,
) -> Result<(), Error> {
    ctx.defer().await?;

    let db = &ctx.data().db;
    let http_client = &ctx.data().http_client;
    let channel_id = ctx.channel_id().get().to_string();

    let address = match resolve_user(http_client, &input).await {
        Some(a) => a,
        None => {
            ctx.say(format!("❌ Carteira não encontrada para **{input}**"))
                .await?;
            return Ok(());
        }
    };

    let subs_col = db.collection::<Subscription>("subscriptions");

    let sub = match subs_col
        .find_one(doc! {
            "channelId": &channel_id,
            "walletAddress": &address
        })
        .await?
    {
        Some(s) => s,
        None => {
            ctx.say(format!(
                "⚠️ Este canal não está rastreando a carteira: {input}\n\
                 Use `/track` primeiro."
            ))
            .await?;
            return Ok(());
        }
    };

    // Limpar todos os filtros
    if limpar.unwrap_or(false) {
        subs_col
            .update_one(
                doc! { "channelId": &channel_id, "walletAddress": &address },
                doc! { "$set": { "filters": { "keywords": [], "minUsd": 0.0 } } },
            )
            .await?;
        ctx.say(format!(
            "✅ Filtros removidos para `{}`!",
            &address[..8]
        ))
        .await?;
        return Ok(());
    }

    let current = sub.filters.unwrap_or_default();
    let mut new_keywords = current.keywords.unwrap_or_default();
    let mut new_min_usd = current.min_usd.unwrap_or(0.0);
    let mut updates: Vec<String> = vec![];

    if let Some(usd) = min_usd {
        new_min_usd = usd;
        updates.push(format!("• Mínimo USD: **${usd}**"));
    }

    if let Some(ref kw) = keyword {
        if !new_keywords.contains(kw) {
            new_keywords.push(kw.clone());
            updates.push(format!("• Keyword adicionada: **\"{kw}\"**"));
        } else {
            updates.push(format!("• Keyword já existe: \"{kw}\""));
        }
    }

    if updates.is_empty() {
        ctx.say(format!(
            "ℹ️ Nenhum filtro alterado.\n\nFiltros atuais:\n\
             • Min USD: ${new_min_usd}\n\
             • Keywords: {}",
            if new_keywords.is_empty() {
                "(nenhuma)".into()
            } else {
                new_keywords.join(", ")
            }
        ))
        .await?;
        return Ok(());
    }

    // Atualiza via doc! — Vec<String> é convertido para BSON array automaticamente
    let keywords_bson = mongodb::bson::to_bson(&new_keywords)?;
    subs_col
        .update_one(
            doc! { "channelId": &channel_id, "walletAddress": &address },
            doc! { "$set": {
                "filters.keywords": keywords_bson,
                "filters.minUsd": new_min_usd
            }},
        )
        .await?;

    ctx.say(format!(
        "✅ **Filtros Atualizados!**\nCarteira: `{}`\n\n{}\n\nFiltros ativos:\n\
         • Min USD: ${new_min_usd}\n\
         • Keywords: {}",
        &address[..8],
        updates.join("\n"),
        if new_keywords.is_empty() {
            "(nenhuma)".into()
        } else {
            new_keywords.join(", ")
        }
    ))
    .await?;

    Ok(())
}

/// Mostra a ajuda do bot
#[poise::command(slash_command)]
pub async fn help(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer().await?;

    let embed = CreateEmbed::new()
        .title("🤖 Polymarket Tracker - Ajuda")
        .description(
            "Bot para rastrear operações em tempo real na Polymarket.\n\n\
             **Como funciona:**\n\
             O bot monitora carteiras via API oficial da Polymarket e envia alertas \
             quando novas operações (compras e vendas) são detectadas.",
        )
        .color(0x5865f2u32)
        .field(
            "📡 `/track <endereço>`",
            "Começa a rastrear uma carteira.\n\
             Aceita: `0x123...abc` ou `@username`",
            false,
        )
        .field(
            "🚫 `/untrack <endereço>`",
            "Para de rastrear uma carteira neste canal.",
            false,
        )
        .field(
            "📋 `/list`",
            "Lista todas as carteiras rastreadas neste canal.",
            false,
        )
        .field(
            "🔍 `/filter`",
            "Configura filtros (valor mínimo, palavras-chave).\n\
             Uso: `/filter input:@user keyword:Trump min_usd:100`",
            false,
        )
        .footer(CreateEmbedFooter::new(
            "💡 Dica: Use @username para facilitar (ex: @GCR)",
        ));

    ctx.send(poise::CreateReply::default().embed(embed)).await?;

    Ok(())
}
