mod bot;
mod models;
mod polymarket;
mod tracker;

use std::sync::Arc;

use dotenvy::dotenv;
use mongodb::Client as MongoClient;
use poise::serenity_prelude as serenity;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    // LOG_ENABLED=false desliga todos os logs (economiza memoria/CPU na VPS)
    // Nivel controlado por RUST_LOG quando ligado (default: info)
    let log_enabled = std::env::var("LOG_ENABLED")
        .map(|v| v.to_lowercase() != "false")
        .unwrap_or(true);

    let filter = if log_enabled {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "polymarket_tracker=info".into())
    } else {
        tracing_subscriber::EnvFilter::new("off")
    };

    tracing_subscriber::fmt().with_env_filter(filter).init();

    let token = std::env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN must be set");
    let mongo_uri = std::env::var("MONGODB_URI").expect("MONGODB_URI must be set");
    let db_name = std::env::var("MONGODB_DB").unwrap_or_else(|_| "polymarket_tracker".into());

    // ─── MongoDB ──────────────────────────────────────────────────────────────
    let mongo_client = MongoClient::with_uri_str(&mongo_uri).await?;
    let db = mongo_client.database(&db_name);
    info!("✅ MongoDB conectado: {db_name}");

    // ─── HTTP client compartilhado ─────────────────────────────────────────
    let http_client = reqwest::Client::builder()
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        )
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    // ─── Poise framework ──────────────────────────────────────────────────────
    let db_for_setup = db.clone();
    let http_client_for_setup = http_client.clone();

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                bot::track(),
                bot::untrack(),
                bot::list_wallets(),
                bot::filter(),
                bot::help(),
            ],
            on_error: |error| Box::pin(on_error(error)),
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            let db = db_for_setup.clone();
            let http_client = http_client_for_setup.clone();
            Box::pin(async move {
                info!("✅ Bot online como {}", ready.user.name);
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(bot::BotData { db, http_client })
            })
        })
        .build();

    let intents = serenity::GatewayIntents::GUILDS | serenity::GatewayIntents::GUILD_MESSAGES;

    let mut client = serenity::ClientBuilder::new(&token, intents)
        .framework(framework)
        .await?;

    // Clona o Http do serenity ANTES de consumir o client com .start()
    let serenity_http = Arc::clone(&client.http);
    let tracker_db = db.clone();
    let tracker_http = http_client.clone();

    tokio::spawn(async move {
        tracker::start_tracker_loop(serenity_http, tracker_db, tracker_http).await;
    });

    info!("🔥 Iniciando bot Discord...");
    client.start().await?;

    Ok(())
}

async fn on_error(error: poise::FrameworkError<'_, bot::BotData, bot::Error>) {
    match error {
        poise::FrameworkError::Setup { error, .. } => {
            panic!("❌ Erro fatal ao iniciar framework: {error:?}");
        }
        poise::FrameworkError::Command { error, ctx, .. } => {
            tracing::error!("❌ Erro no comando '{}': {error:?}", ctx.command().name);
            let _ = ctx.say("❌ Ocorreu um erro ao executar o comando.").await;
        }
        error => {
            if let Err(e) = poise::builtins::on_error(error).await {
                tracing::error!("❌ Erro ao tratar erro do framework: {e}");
            }
        }
    }
}
