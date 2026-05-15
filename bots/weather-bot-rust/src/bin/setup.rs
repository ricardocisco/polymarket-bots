// src/bin/setup.rs  (binário: setup)
//! Verificação inicial — rode ANTES de ligar o bot real.
//!
//! O que verifica:
//!   ✓ Chave privada válida e wallet detectada
//!   ✓ Conexão com CLOB API
//!   ✓ Derivação de credenciais L2 (api_key / secret / passphrase)
//!   ✓ Mercados ativos por cidade (com ICAO e coordenadas resolvidas)
//!   ✓ Previsão de temperatura para cada estação de aeroporto
//!
//! Uso: cargo run --bin setup

#[path = "../cities.rs"]
mod cities;
#[path = "../config.rs"]
mod config;
#[path = "../markets.rs"]
mod markets;
#[path = "../strategy.rs"]
mod strategy;
#[path = "../types.rs"]
mod types;
#[path = "../weather.rs"]
mod weather;

use anyhow::{Context, Result};
use polymarket_client_sdk::{
    auth::state::Unauthenticated,
    auth::{LocalSigner, Signer},
    clob::{Client, Config as ClobConfig},
    POLYGON,
};
use std::str::FromStr;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, EnvFilter};

use cities::all_slugs;
use config::Config;
use markets::GammaClient;
use weather::WeatherClient;

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::new("info"))
        .with_target(false)
        .pretty()
        .init();

    info!("╔══════════════════════════════════════════════════════════╗");
    info!("║       Polymarket Weather Bot — Setup & Verificação       ║");
    info!("╚══════════════════════════════════════════════════════════╝\n");

    // ── Config ───────────────────────────────────────────────
    let cfg = Config::from_env()
        .context("Configure o .env com POLYMARKET_PRIVATE_KEY (veja .env.example)")?;

    // ── Wallet ───────────────────────────────────────────────
    let signer = LocalSigner::from_str(&cfg.private_key)
        .context("POLYMARKET_PRIVATE_KEY inválida (formato hex sem 0x)")?
        .with_chain_id(Some(POLYGON));
    info!("🔑 Wallet: {:?}", signer.address());

    // ── CLOB API (não autenticado — só testa conectividade) ───
    info!("\n📡 Testando CLOB API...");
    let unauth =
        Client::<Unauthenticated>::new("https://clob.polymarket.com", ClobConfig::default())
            .context("Falha ao criar cliente CLOB")?;

    match unauth.ok().await {
        Ok(msg) => info!("  ✅ CLOB respondendo: {}", msg),
        Err(e) => error!(
            "  ❌ CLOB inacessível: {}\n     Verifique sua conexão e se a Polymarket está online.",
            e
        ),
    }

    // ── Autenticação L2 (EIP-712) ─────────────────────────────
    info!("\n🔐 Autenticando (EIP-712)...");
    let clob =
        match Client::<Unauthenticated>::new("https://clob.polymarket.com", ClobConfig::default())
            .context("Falha ao criar cliente CLOB para autenticação")?
            .authentication_builder(&signer)
            .authenticate()
            .await
        {
            Ok(c) => {
                info!("  ✅ Autenticado com sucesso");
                Some(c)
            }
            Err(e) => {
                error!("  ❌ Falha ao autenticar: {}", e);
                error!(
                    "     A wallet precisa ter USDC e ter feito pelo menos 1 trade na Polymarket."
                );
                None
            }
        };

    if let Some(c) = &clob {
        match c.api_keys().await {
            Ok(keys) => info!("  ✅ Chaves API obtidas: {:?}", keys),
            Err(e) => warn!("  ⚠️  Não foi possível listar chaves API: {}", e),
        }
    }

    // ── Mercados e previsões por cidade ───────────────────────
    info!("\n{}", "─".repeat(60));
    info!("🌍 Verificando mercados ativos e previsões...");
    info!("{}\n", "─".repeat(60));

    let gamma = GammaClient::new()?;
    let weather = WeatherClient::new()?;

    for slug in all_slugs() {
        info!("📍 {} (keyword: '{}')", slug.name, slug.keyword);

        let markets = match gamma.fetch_markets(slug.keyword).await {
            Ok(m) if m.is_empty() => {
                info!("  📊 Nenhum mercado ativo hoje\n");
                continue;
            }
            Ok(m) => m,
            Err(e) => {
                error!("  ❌ Erro: {}\n", e);
                continue;
            }
        };

        // Agrupa por ICAO (pode haver múltiplos mercados para o mesmo aeroporto)
        let mut seen_icao = std::collections::HashSet::new();
        for mkt in &markets {
            if seen_icao.insert(&mkt.icao) {
                // Exibe info da estação uma vez por ICAO
                info!(
                    "  ✈️  Estação ICAO: {} | coords: ({:.4}, {:.4}) | unidade: {}",
                    mkt.icao,
                    mkt.station_lat,
                    mkt.station_lon,
                    mkt.unit.symbol()
                );

                match weather.fetch_for_market(mkt).await {
                    Ok(Some(f)) => info!(
                        "  🌡️  Previsão hoje: {:.1}{} | Confiança: {:.1}%",
                        f.max_temp,
                        f.unit.symbol(),
                        f.confidence * 100.0
                    ),
                    Ok(None) => warn!("  ⚠️  Sem previsão disponível"),
                    Err(e) => error!("  ❌ Erro previsão: {}", e),
                }
            }

            info!(
                "  📊 Mercado: '{}' | YES={:.3} | NO={:.3} | Range=[{:?}–{:?}]",
                mkt.question.chars().take(60).collect::<String>().as_str(),
                mkt.yes_price,
                mkt.no_price,
                mkt.range_min,
                mkt.range_max
            );
        }
        info!("");
    }

    info!("{}", "═".repeat(60));
    info!("✅ Setup concluído!");
    info!("");
    info!("  Para iniciar o bot:");
    info!("    cargo run --release --bin bot");
    info!("");
    info!("  Para rodar backtesting:");
    info!("    cargo run --bin backtest");
    info!("    cargo run --bin backtest -- --days 60 --min-confidence 0.95");
    info!("{}", "═".repeat(60));

    Ok(())
}
