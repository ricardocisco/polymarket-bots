use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::U256;
use alloy_signer_local::PrivateKeySigner;
use anyhow::{Context, Result};
use chrono::{Local, NaiveDate, Utc};
use polymarket_client_sdk::auth::state::{Authenticated, Unauthenticated};
use polymarket_client_sdk::{
    auth::{LocalSigner, Normal, Signer},
    clob::client::Client as ClobClient,
    clob::types::{OrderStatusType, OrderType, Side as PolySide},
    clob::Config as ClobConfig,
    POLYGON,
};
use rust_decimal::Decimal;
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};
use tracing::{info, warn};

use crate::config::Config;
use crate::consensus::{compute_consensus, consensus_to_forecast};
use crate::engine::discovery::{
    fetch_temperature_markets, winner_of_closed_market, DiscoveredMarket,
};
use crate::feed::orderbook::OrderbookFeed;
use crate::markets::TempUnit;
use crate::storage::weather_paper_trades::{WeatherTradeRow, WeatherTradeStore};
use crate::strategy::{evaluate_cross_market_group, evaluate_opportunity_with_trend, Decision};
use crate::trend::analyze_trend;
use crate::types::{ConsensusResult, Forecast, QuoteSnapshot, Side, TrendAnalysis};
use crate::weather::WeatherClient;

type AuthClient = ClobClient<Authenticated<Normal>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotRunMode {
    SimMonitor,
    Bot,
}

pub async fn run_sim_monitor(cfg: Config) -> Result<()> {
    let http = reqwest::Client::builder()
        .user_agent("polymarket-weather-sim-monitor/2.0")
        .timeout(Duration::from_secs(25))
        .build()?;
    let weather = Arc::new(WeatherClient::new()?);
    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL nao configurado. O sim_monitor usa PostgreSQL.")?;
    let db = WeatherTradeStore::connect(&database_url)
        .await
        .context("falha ao conectar no PostgreSQL do sim_monitor")?;
    let mut rows = db
        .list_trades()
        .await
        .context("falha ao carregar historico do banco")?;
    rows.retain(|row| row.execution_mode == "paper");

    print_sim_banner(&cfg);

    let mode = EngineMode::Sim {
        db,
        total_rows: rows.clone(),
    };
    run_engine(cfg, http, weather, mode, rows, BotRunMode::SimMonitor).await
}

pub async fn print_sim_report() -> Result<()> {
    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL nao configurado. O sim_monitor usa PostgreSQL.")?;
    let db = WeatherTradeStore::connect(&database_url)
        .await
        .context("falha ao conectar no PostgreSQL do sim_monitor")?;
    let rows = db
        .list_trades()
        .await
        .context("falha ao carregar historico do banco")?;
    print_book_report(&rows);
    Ok(())
}

pub async fn run_bot(cfg: Config) -> Result<()> {
    let http = reqwest::Client::builder()
        .user_agent("polymarket-weather-bot/2.0")
        .timeout(Duration::from_secs(25))
        .build()?;
    let weather = Arc::new(WeatherClient::new()?);

    let mut existing_rows = Vec::new();
    let mode = if cfg.live_trading_enabled() {
        let database_url = std::env::var("DATABASE_URL")
            .context("DATABASE_URL obrigatorio para persistir ordens live")?;
        let db = WeatherTradeStore::connect(&database_url)
            .await
            .context("falha ao conectar no ledger live")?;
        existing_rows = db
            .list_trades()
            .await
            .context("falha ao carregar posicoes live persistidas")?;
        existing_rows.retain(|row| row.execution_mode == "live");
        let signer = LocalSigner::from_str(&cfg.private_key)
            .context("POLYMARKET_PRIVATE_KEY invalida (hex sem 0x)")?
            .with_chain_id(Some(POLYGON));
        let clob = ClobClient::<Unauthenticated>::new(
            "https://clob.polymarket.com",
            ClobConfig::default(),
        )
        .context("falha ao criar cliente CLOB")?
        .authentication_builder(&signer)
        .authenticate()
        .await
        .context("falha ao autenticar no CLOB")?;
        info!("wallet = {:?}", signer.address());
        EngineMode::Live { clob, signer, db }
    } else {
        EngineMode::DryBot
    };

    print_bot_banner(&cfg);
    run_engine(cfg, http, weather, mode, existing_rows, BotRunMode::Bot).await
}

#[derive(Clone)]
enum EngineMode {
    Sim {
        db: WeatherTradeStore,
        total_rows: Vec<WeatherTradeRow>,
    },
    Live {
        clob: AuthClient,
        signer: PrivateKeySigner,
        db: WeatherTradeStore,
    },
    DryBot,
}

#[derive(Debug, Clone)]
struct PositionRecord {
    market_key: String,
    event_slug: String,
    city: String,
    target_date: NaiveDate,
    icao: String,
    resolution_source: String,
    question: String,
    direction: String,
    token_id: String,
    size_usdc: f64,
    entry_price: f64,
    shares: u32,
    predicted_temp: f64,
    unit: TempUnit,
    confidence: f64,
    effective_confidence: f64,
    expected_value: f64,
    edge_per_share: f64,
    strategy_type: String,
}

#[derive(Debug, Clone, Copy)]
struct EvalSnapshot {
    forecast_temp: f64,
    observed_max: f64,
    yes_buy: f64,
    no_buy: f64,
    peak_confirmed: bool,
}

#[derive(Debug)]
struct MarketRuntime {
    discovered: DiscoveredMarket,
    latest_quote: Option<QuoteSnapshot>,
    latest_forecast: Option<Forecast>,
    latest_trend: TrendAnalysis,
    last_eval: Option<EvalSnapshot>,
    position_open: bool,
    /// Consensus multi-fonte mais recente (None = ainda não calculado)
    latest_consensus: Option<ConsensusResult>,
    /// Quantos ciclos consecutivos o consensus confirmou o mesmo bin
    consensus_confidence_streak: usize,
}

#[derive(Debug)]
enum InternalEvent {
    Quote {
        market_key: String,
        quote: QuoteSnapshot,
    },
    Weather {
        market_key: String,
        forecast: Forecast,
        trend: TrendAnalysis,
    },
    /// Consensus multi-fonte calculado para um grupo ICAO+data
    Consensus {
        /// Chave do mercado que disparou o cálculo (qualquer um do grupo)
        market_key: String,
        consensus: ConsensusResult,
    },
}

async fn run_engine(
    cfg: Config,
    http: reqwest::Client,
    weather: Arc<WeatherClient>,
    mode: EngineMode,
    existing_rows: Vec<WeatherTradeRow>,
    run_mode: BotRunMode,
) -> Result<()> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let mut runtimes: HashMap<String, MarketRuntime> = HashMap::new();
    let mut open_positions = load_open_positions(&existing_rows);

    let mut discovery_tick = interval(Duration::from_secs(cfg.discovery_refresh_secs.max(30)));
    discovery_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut resolution_tick = interval(Duration::from_secs(cfg.resolution_poll_secs.max(30)));
    resolution_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    refresh_discovery(
        &cfg,
        &http,
        weather.clone(),
        &event_tx,
        &mut runtimes,
        &open_positions,
    )
    .await?;

    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                handle_event(&cfg, &http, &mode, &mut runtimes, &mut open_positions, event, run_mode).await?;
            }
            _ = discovery_tick.tick() => {
                if let Err(e) = refresh_discovery(&cfg, &http, weather.clone(), &event_tx, &mut runtimes, &open_positions).await {
                    warn!("[discovery] falhou: {e:#}");
                }
            }
            _ = resolution_tick.tick() => {
                if let Err(e) = sweep_resolutions(&http, weather.clone(), &mode, &mut runtimes, &mut open_positions, run_mode).await {
                    warn!("[resolution] falhou: {e:#}");
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("encerrando weather bot...");
                break;
            }
        }
    }

    Ok(())
}

async fn refresh_discovery(
    cfg: &Config,
    http: &reqwest::Client,
    weather: Arc<WeatherClient>,
    event_tx: &mpsc::UnboundedSender<InternalEvent>,
    runtimes: &mut HashMap<String, MarketRuntime>,
    open_positions: &HashMap<String, PositionRecord>,
) -> Result<()> {
    let discovered = fetch_temperature_markets(http, cfg.extended_horizon_days).await?;
    let mut new_count = 0usize;

    for discovered_market in discovered {
        let market_key = discovered_market.market_key.clone();
        if let Some(runtime) = runtimes.get_mut(&market_key) {
            runtime.discovered = discovered_market;
            runtime.position_open = open_positions.contains_key(&market_key);
            continue;
        }

        let feed = OrderbookFeed::start(
            &discovered_market.market.yes_token_id,
            &discovered_market.market.no_token_id,
        )
        .with_context(|| format!("falha ao iniciar WS para {}", market_key))?;
        let mut quote_rx = feed.subscribe();
        let quote_tx = event_tx.clone();
        let quote_market_key = market_key.clone();
        tokio::spawn(async move {
            loop {
                if quote_rx.changed().await.is_err() {
                    break;
                }
                if let Some(quote) = *quote_rx.borrow() {
                    let _ = quote_tx.send(InternalEvent::Quote {
                        market_key: quote_market_key.clone(),
                        quote,
                    });
                }
            }
        });

        let weather_tx = event_tx.clone();
        let weather_market_key = market_key.clone();
        let market_for_poll = discovered_market.market.clone();
        let cfg_for_poll = cfg.clone();
        let weather_client = weather.clone();
        tokio::spawn(async move {
            loop {
                // ── Fetch forecast padrão (para trend e intradiário) ──────
                match weather_client.fetch_for_market(&market_for_poll).await {
                    Ok(Some(forecast)) => {
                        let trend = if market_for_poll
                            .target_date
                            .map(|d| d == Local::now().date_naive())
                            .unwrap_or(false)
                        {
                            match weather_client.fetch_hourly_today(&market_for_poll).await {
                                Ok(hourly) => analyze_trend(&hourly, market_for_poll.station_lon),
                                Err(_) => analyze_trend(&[], market_for_poll.station_lon),
                            }
                        } else {
                            analyze_trend(&[], market_for_poll.station_lon)
                        };

                        let _ = weather_tx.send(InternalEvent::Weather {
                            market_key: weather_market_key.clone(),
                            forecast,
                            trend,
                        });
                    }
                    Ok(None) => {}
                    Err(e) => warn!("[weather:{}] falha: {}", weather_market_key, e),
                }

                // ── Fetch consensus multi-fonte ───────────────────────────
                let sources = weather_client.fetch_all_sources(&market_for_poll).await;
                if !sources.is_empty() {
                    let today = Local::now().date_naive();
                    let target_date = market_for_poll.target_date.unwrap_or(today);
                    let days_ahead = (target_date - today).num_days().max(0) as usize;
                    let consensus = compute_consensus(&sources, days_ahead);
                    let _ = weather_tx.send(InternalEvent::Consensus {
                        market_key: weather_market_key.clone(),
                        consensus,
                    });
                }

                tokio::time::sleep(Duration::from_secs(weather_poll_secs(
                    &cfg_for_poll,
                    market_for_poll.target_date,
                )))
                .await;
            }
        });

        let position_open = open_positions.contains_key(&market_key);
        runtimes.insert(
            market_key.clone(),
            MarketRuntime {
                discovered: discovered_market,
                latest_quote: if feed.is_ready() {
                    feed.get_quote()
                } else {
                    None
                },
                latest_forecast: None,
                latest_trend: analyze_trend(&[], 0.0),
                last_eval: None,
                position_open,
                latest_consensus: None,
                consensus_confidence_streak: 0,
            },
        );
        new_count += 1;
    }

    if new_count > 0 {
        info!(
            "[discovery] {} mercados ativos em runtime ({} novos)",
            runtimes.len(),
            new_count
        );
    }

    Ok(())
}

async fn handle_event(
    cfg: &Config,
    _http: &reqwest::Client,
    mode: &EngineMode,
    runtimes: &mut HashMap<String, MarketRuntime>,
    open_positions: &mut HashMap<String, PositionRecord>,
    event: InternalEvent,
    run_mode: BotRunMode,
) -> Result<()> {
    match event {
        InternalEvent::Quote { market_key, quote } => {
            let Some(runtime) = runtimes.get_mut(&market_key) else {
                return Ok(());
            };
            runtime.latest_quote = Some(quote);
            maybe_evaluate_market(cfg, mode, runtime, open_positions, run_mode).await?;
        }
        InternalEvent::Weather {
            market_key,
            forecast,
            trend,
        } => {
            let Some(runtime) = runtimes.get_mut(&market_key) else {
                return Ok(());
            };
            runtime.latest_forecast = Some(forecast);
            runtime.latest_trend = trend;
            maybe_evaluate_market(cfg, mode, runtime, open_positions, run_mode).await?;
        }
        InternalEvent::Consensus {
            market_key,
            consensus,
        } => {
            // Armazena consensus no runtime do mercado que disparou o evento
            if let Some(runtime) = runtimes.get_mut(&market_key) {
                // Atualiza streak de confiança: conta ciclos consecutivos com alta confiança
                if consensus.is_reliable(cfg.num_sources_required, cfg.source_agreement_threshold) {
                    runtime.consensus_confidence_streak += 1;
                } else {
                    runtime.consensus_confidence_streak = 0;
                }
                runtime.latest_consensus = Some(consensus);
            }

            // Busca todos os mercados do mesmo ICAO+data para avaliação cross-market
            let Some(trigger_runtime) = runtimes.get(&market_key) else {
                return Ok(());
            };
            let trigger_icao = trigger_runtime.discovered.market.icao.clone();
            let trigger_date = trigger_runtime.discovered.market.target_date;
            let consensus_for_group = trigger_runtime.latest_consensus.clone();
            let streak = trigger_runtime.consensus_confidence_streak;

            let Some(consensus) = consensus_for_group else {
                return Ok(());
            };

            // Só avalia cross-market se consensus for confiável e tiver N+ ciclos consecutivos
            if !consensus.is_reliable(cfg.num_sources_required, cfg.source_agreement_threshold) {
                return Ok(());
            }

            // Coleta todos os mercados do mesmo ICAO+data
            let group_keys: Vec<String> = runtimes
                .iter()
                .filter(|(_, rt)| {
                    rt.discovered.market.icao == trigger_icao
                        && rt.discovered.market.target_date == trigger_date
                        && !rt.position_open
                })
                .map(|(k, _)| k.clone())
                .collect();

            if group_keys.len() < 2 {
                // Sem grupo suficiente para cross-market, usa avaliação individual
                if let Some(runtime) = runtimes.get_mut(&market_key) {
                    let forecast = consensus_to_forecast(
                        &consensus,
                        &runtime.discovered.market.icao,
                        runtime.discovered.market.unit,
                    );
                    runtime.latest_forecast = Some(forecast);
                    maybe_evaluate_market(cfg, mode, runtime, open_positions, run_mode).await?;
                }
                return Ok(());
            }

            info!(
                "[cross-market] ICAO={} | {} mercados no grupo | Consensus={:.1} | Streak={}",
                trigger_icao,
                group_keys.len(),
                consensus.predicted_temp,
                streak,
            );

            // Constrói o grupo de mercados para avaliação cross-market
            // (coleta referências sem borrow duplo — usa snapshots)
            let now_ts = Utc::now().timestamp();
            let group_snapshot: Vec<(String, crate::markets::TempMarket)> = group_keys
                .iter()
                .filter_map(|k| {
                    let rt = runtimes.get(k)?;
                    let quote = rt.latest_quote?;
                    if now_ts.saturating_sub(quote.ts) > cfg.max_quote_age_secs {
                        return None;
                    }
                    let mut market = rt.discovered.market.clone();
                    market.yes_price = quote.best_buy_price(Side::Yes)?;
                    market.no_price = quote.best_buy_price(Side::No)?;
                    Some((rt.discovered.market_key.clone(), market))
                })
                .collect();

            let group_refs: Vec<(&String, &crate::markets::TempMarket)> =
                group_snapshot.iter().map(|(k, m)| (k, m)).collect();

            let cross_decisions = evaluate_cross_market_group(
                &group_refs,
                consensus.predicted_temp,
                consensus.confidence,
                consensus.uncertainty,
                cfg,
            );

            // Executa decisões cross-market para cada mercado
            for cross_decision in cross_decisions {
                if open_positions.len() >= cfg.max_open_positions {
                    warn!("limite global de posicoes abertas atingido");
                    break;
                }
                let market_key_for_exec = cross_decision.market_key.clone();
                let Some(runtime) = runtimes.get_mut(&market_key_for_exec) else {
                    continue;
                };
                if runtime.position_open {
                    continue;
                }

                let Some(quote) = runtime.latest_quote else {
                    continue;
                };
                if Utc::now().timestamp().saturating_sub(quote.ts) > cfg.max_quote_age_secs {
                    continue;
                }

                let (side, token_id, _shares, size_usdc, price, reason, strategy_type) =
                    match &cross_decision.opportunity.decision {
                        Decision::BuyYes {
                            token_id,
                            shares,
                            size_usdc,
                            price,
                            reason,
                            ..
                        } => (
                            Side::Yes,
                            token_id.clone(),
                            *shares,
                            decimal_to_f64(*size_usdc),
                            *price,
                            reason.clone(),
                            "cross_market_yes".to_string(),
                        ),
                        Decision::BuyNo {
                            token_id,
                            shares,
                            size_usdc,
                            price,
                            reason,
                            ..
                        } => (
                            Side::No,
                            token_id.clone(),
                            *shares,
                            decimal_to_f64(*size_usdc),
                            *price,
                            reason.clone(),
                            "cross_market_no_penny".to_string(),
                        ),
                        Decision::Skip(_) => continue,
                    };

                if cross_decision.opportunity.edge_per_share < cfg.edge_min {
                    continue;
                }

                // Usa o preço do orderbook se disponível
                let actual_price = match side {
                    Side::Yes => quote.best_buy_price(Side::Yes).unwrap_or(price),
                    Side::No => quote.best_buy_price(Side::No).unwrap_or(price),
                };
                let actual_edge = cross_decision.opportunity.effective_confidence - actual_price;
                let actual_shares = if actual_price > 0.0 {
                    (size_usdc / actual_price).floor() as u32
                } else {
                    0
                };
                let actual_ev = actual_shares as f64 * actual_edge;
                if actual_edge < cfg.edge_min || actual_ev <= 0.0 || actual_shares == 0 {
                    continue;
                }
                if let Some(spread) = quote.spread(side) {
                    if spread * 100.0 > cfg.max_spread_cents {
                        continue;
                    }
                }

                let position = PositionRecord {
                    market_key: runtime.discovered.market_key.clone(),
                    event_slug: runtime.discovered.market.event_slug.clone(),
                    city: runtime.discovered.city.clone(),
                    target_date: runtime
                        .discovered
                        .market
                        .target_date
                        .unwrap_or_else(|| Local::now().date_naive()),
                    icao: runtime.discovered.market.icao.clone(),
                    resolution_source: runtime.discovered.resolution_source.clone(),
                    question: runtime.discovered.market.question.clone(),
                    direction: if side == Side::Yes {
                        "BUY_YES".into()
                    } else {
                        "BUY_NO".into()
                    },
                    token_id,
                    size_usdc,
                    entry_price: actual_price,
                    shares: actual_shares,
                    predicted_temp: consensus.predicted_temp,
                    unit: runtime.discovered.market.unit,
                    confidence: consensus.confidence,
                    effective_confidence: cross_decision.opportunity.effective_confidence,
                    expected_value: actual_ev,
                    edge_per_share: actual_edge,
                    strategy_type,
                };

                execute_entry(
                    mode,
                    runtime,
                    open_positions,
                    position,
                    &reason,
                    &cross_decision.cross_reason,
                    run_mode,
                )
                .await?;
            }
        }
    }

    Ok(())
}

async fn maybe_evaluate_market(
    cfg: &Config,
    mode: &EngineMode,
    runtime: &mut MarketRuntime,
    open_positions: &mut HashMap<String, PositionRecord>,
    run_mode: BotRunMode,
) -> Result<()> {
    let Some(forecast) = runtime.latest_forecast.clone() else {
        return Ok(());
    };
    let Some(quote) = runtime.latest_quote else {
        return Ok(());
    };
    if Utc::now().timestamp().saturating_sub(quote.ts) > cfg.max_quote_age_secs {
        return Ok(());
    }

    let yes_buy = quote
        .best_buy_price(Side::Yes)
        .unwrap_or(runtime.discovered.market.yes_price);
    let no_buy = quote
        .best_buy_price(Side::No)
        .unwrap_or(runtime.discovered.market.no_price);
    runtime.discovered.market.yes_price = yes_buy;
    runtime.discovered.market.no_price = no_buy;

    let observed_max = if runtime.latest_trend.observed_max.is_finite() {
        runtime.latest_trend.observed_max
    } else {
        forecast.max_temp
    };
    let peak_confirmed = runtime
        .latest_trend
        .hours_since_peak
        .map(|h| h >= 2)
        .unwrap_or(false)
        && runtime.latest_trend.slope_3h < 0.0
        && runtime.latest_trend.local_hour >= 13;

    let next_eval = EvalSnapshot {
        forecast_temp: forecast.max_temp,
        observed_max,
        yes_buy,
        no_buy,
        peak_confirmed,
    };
    let Some(change_reason) = change_reason(cfg, runtime.last_eval, next_eval) else {
        return Ok(());
    };
    runtime.last_eval = Some(next_eval);

    let opportunity = evaluate_opportunity_with_trend(
        &runtime.discovered.market,
        &forecast,
        &runtime.latest_trend,
        cfg,
    );

    if runtime.position_open {
        info!(
            "[{}] thesis-update | {} | prev={:.1}{} yes={:.1}c no={:.1}c",
            runtime.discovered.city,
            change_reason,
            forecast.max_temp,
            forecast.unit.symbol(),
            yes_buy * 100.0,
            no_buy * 100.0
        );
        return Ok(());
    }
    if open_positions.len() >= cfg.max_open_positions {
        info!(
            "[{}] skip | limite global de posicoes abertas",
            runtime.discovered.city
        );
        return Ok(());
    }

    let (side, token_id, shares, size_usdc, price, reason, strategy_type) =
        match &opportunity.decision {
            Decision::BuyYes {
                token_id,
                shares,
                size_usdc,
                price,
                reason,
                ..
            } => (
                Side::Yes,
                token_id.clone(),
                *shares,
                decimal_to_f64(*size_usdc),
                *price,
                reason.clone(),
                opportunity.strategy_kind.as_str().to_string(),
            ),
            Decision::BuyNo {
                token_id,
                shares,
                size_usdc,
                price,
                reason,
                ..
            } => (
                Side::No,
                token_id.clone(),
                *shares,
                decimal_to_f64(*size_usdc),
                *price,
                reason.clone(),
                opportunity.strategy_kind.as_str().to_string(),
            ),
            Decision::Skip(reason) => {
                info!(
                    "[{}] skip | {} | {}",
                    runtime.discovered.city, change_reason, reason
                );
                return Ok(());
            }
        };

    if opportunity.edge_per_share < cfg.edge_min {
        info!(
            "[{}] skip-edge | edge/share={:+.3} < {:.3} | {}",
            runtime.discovered.city, opportunity.edge_per_share, cfg.edge_min, change_reason
        );
        return Ok(());
    }
    if opportunity.expected_value <= 0.0 {
        info!(
            "[{}] skip-ev | EV={:+.2} | {}",
            runtime.discovered.city, opportunity.expected_value, change_reason
        );
        return Ok(());
    }
    if let Some(spread) = quote.spread(side) {
        if spread * 100.0 > cfg.max_spread_cents {
            info!(
                "[{}] skip-spread | spread={:.1}c > {:.1}c | {}",
                runtime.discovered.city,
                spread * 100.0,
                cfg.max_spread_cents,
                change_reason
            );
            return Ok(());
        }
    }

    let position = PositionRecord {
        market_key: runtime.discovered.market_key.clone(),
        event_slug: runtime.discovered.market.event_slug.clone(),
        city: runtime.discovered.city.clone(),
        target_date: runtime
            .discovered
            .market
            .target_date
            .unwrap_or_else(|| Local::now().date_naive()),
        icao: runtime.discovered.market.icao.clone(),
        resolution_source: runtime.discovered.resolution_source.clone(),
        question: runtime.discovered.market.question.clone(),
        direction: if side == Side::Yes {
            "BUY_YES".into()
        } else {
            "BUY_NO".into()
        },
        token_id,
        size_usdc,
        entry_price: price,
        shares,
        predicted_temp: forecast.max_temp,
        unit: forecast.unit,
        confidence: forecast.confidence,
        effective_confidence: opportunity.effective_confidence,
        expected_value: opportunity.expected_value,
        edge_per_share: opportunity.edge_per_share,
        strategy_type,
    };

    execute_entry(
        mode,
        runtime,
        open_positions,
        position,
        &reason,
        &change_reason,
        run_mode,
    )
    .await
}

async fn execute_entry(
    mode: &EngineMode,
    runtime: &mut MarketRuntime,
    open_positions: &mut HashMap<String, PositionRecord>,
    position: PositionRecord,
    decision_reason: &str,
    change_reason: &str,
    run_mode: BotRunMode,
) -> Result<()> {
    match mode {
        EngineMode::Sim { db, total_rows } => {
            db.insert_open_trade(
                &position.market_key,
                &position.city,
                &position.target_date.to_string(),
                &position.icao,
                &position.resolution_source,
                &position.question,
                &position.direction,
                &position.token_id,
                position.size_usdc,
                position.entry_price,
                position.predicted_temp,
                position.unit.symbol(),
                position.confidence,
                Some(position.effective_confidence),
                Some(position.expected_value),
                Some(position.edge_per_share),
                Some(&position.strategy_type),
                "paper",
                None,
                &Local::now().to_rfc3339(),
                Local::now().timestamp(),
            )
            .await?;
            let mut total_rows = total_rows.clone();
            total_rows.push(weather_row_from_position(&position));

            log_entry(&position, decision_reason, change_reason, run_mode, false);
            runtime.position_open = true;
            open_positions.insert(position.market_key.clone(), position);
        }
        EngineMode::DryBot => {
            log_entry(&position, decision_reason, change_reason, run_mode, true);
            runtime.position_open = true;
            open_positions.insert(position.market_key.clone(), position);
        }
        EngineMode::Live { clob, signer, db } => {
            let price_dec = Decimal::from_str(&format!("{:.6}", position.entry_price))
                .context("preco invalido")?;
            let size_dec = Decimal::from_str(&format!("{:.6}", position.size_usdc))
                .context("size_usdc invalido")?;
            let shares = if price_dec > Decimal::ZERO {
                size_dec / price_dec
            } else {
                size_dec
            };
            let token: U256 = position.token_id.parse().context("token_id invalido")?;
            let order = clob
                .limit_order()
                .token_id(token)
                .size(shares)
                .price(price_dec)
                .side(PolySide::Buy)
                .order_type(OrderType::FOK)
                .build()
                .await?;
            let signed = clob.sign(signer, order).await?;
            let resp = clob.post_order(signed).await?;

            if !resp.success || !matches!(resp.status, OrderStatusType::Matched) {
                anyhow::bail!(
                    "ordem FOK nao executada: status={} erro={}",
                    resp.status,
                    resp.error_msg.as_deref().unwrap_or("sem detalhe")
                );
            }

            db.insert_open_trade(
                &position.market_key,
                &position.city,
                &position.target_date.to_string(),
                &position.icao,
                &position.resolution_source,
                &position.question,
                &position.direction,
                &position.token_id,
                position.size_usdc,
                position.entry_price,
                position.predicted_temp,
                position.unit.symbol(),
                position.confidence,
                Some(position.effective_confidence),
                Some(position.expected_value),
                Some(position.edge_per_share),
                Some(&position.strategy_type),
                "live",
                Some(&resp.order_id),
                &Local::now().to_rfc3339(),
                Local::now().timestamp(),
            )
            .await?;

            log_entry(&position, decision_reason, change_reason, run_mode, false);
            info!(
                "[{}] order-posted | order_id={:?}",
                position.city, resp.order_id
            );
            runtime.position_open = true;
            open_positions.insert(position.market_key.clone(), position);
        }
    }

    Ok(())
}

async fn sweep_resolutions(
    http: &reqwest::Client,
    weather: Arc<WeatherClient>,
    mode: &EngineMode,
    runtimes: &mut HashMap<String, MarketRuntime>,
    open_positions: &mut HashMap<String, PositionRecord>,
    run_mode: BotRunMode,
) -> Result<()> {
    let keys: Vec<String> = open_positions.keys().cloned().collect();

    for market_key in keys {
        let Some(position) = open_positions.get(&market_key).cloned() else {
            continue;
        };
        let Some(winner) = winner_of_closed_market(http, &position.event_slug).await? else {
            continue;
        };

        let won = if position.direction == "BUY_YES" {
            winner == position.question
        } else {
            winner != position.question
        };
        let pnl = if won {
            position.size_usdc * (1.0 / position.entry_price - 1.0)
        } else {
            -position.size_usdc
        };
        let actual_temp = weather
            .fetch_wunderground_historical(
                &position.resolution_source,
                position.target_date,
                position.unit,
                &position.icao,
            )
            .await
            .ok()
            .flatten()
            .map(|f| f.max_temp);

        if let EngineMode::Sim { db, .. } = mode {
            db.settle_trade(
                &position.market_key,
                if won { "won" } else { "lost" },
                actual_temp,
                pnl,
                &winner,
                &Local::now().to_rfc3339(),
            )
            .await?;
        }

        log_resolution(&position, won, pnl, actual_temp, run_mode);
        open_positions.remove(&market_key);
        if let Some(runtime) = runtimes.get_mut(&market_key) {
            runtime.position_open = false;
        }
    }

    Ok(())
}

fn change_reason(cfg: &Config, prev: Option<EvalSnapshot>, next: EvalSnapshot) -> Option<String> {
    let Some(prev) = prev else {
        return Some("primeira tese pronta".into());
    };

    let forecast_delta = (next.forecast_temp - prev.forecast_temp).abs();
    if forecast_delta >= cfg.forecast_change_trigger_degrees {
        return Some(format!(
            "forecast {:.1} -> {:.1} (delta {:.1})",
            prev.forecast_temp, next.forecast_temp, forecast_delta
        ));
    }

    let obs_delta = (next.observed_max - prev.observed_max).abs();
    if obs_delta >= cfg.forecast_change_trigger_degrees {
        return Some(format!(
            "observado {:.1} -> {:.1} (delta {:.1})",
            prev.observed_max, next.observed_max, obs_delta
        ));
    }

    let yes_delta_cents = (next.yes_buy - prev.yes_buy).abs() * 100.0;
    if yes_delta_cents >= cfg.implied_move_trigger_cents {
        return Some(format!(
            "yes {:.1}c -> {:.1}c",
            prev.yes_buy * 100.0,
            next.yes_buy * 100.0
        ));
    }

    let no_delta_cents = (next.no_buy - prev.no_buy).abs() * 100.0;
    if no_delta_cents >= cfg.implied_move_trigger_cents {
        return Some(format!(
            "no {:.1}c -> {:.1}c",
            prev.no_buy * 100.0,
            next.no_buy * 100.0
        ));
    }

    if next.peak_confirmed && !prev.peak_confirmed {
        return Some("pico intradiario confirmado".into());
    }

    None
}

fn weather_poll_secs(cfg: &Config, target_date: Option<NaiveDate>) -> u64 {
    let today = Local::now().date_naive();
    let days = target_date.map(|d| (d - today).num_days()).unwrap_or(0);
    match days {
        d if d <= 0 => cfg.weather_intraday_poll_secs,
        1 => cfg.weather_poll_d1_secs,
        2 => cfg.weather_poll_d2_secs,
        _ => cfg.weather_poll_d3_secs,
    }
    .max(60)
}

fn load_open_positions(rows: &[WeatherTradeRow]) -> HashMap<String, PositionRecord> {
    rows.iter()
        .filter(|row| row.status == "pending")
        .filter_map(|row| {
            let target_date = NaiveDate::parse_from_str(&row.target_date, "%Y-%m-%d").ok()?;
            let unit = if row.temp_unit.contains('F') {
                TempUnit::Fahrenheit
            } else {
                TempUnit::Celsius
            };
            Some((
                row.entry_id.clone(),
                PositionRecord {
                    market_key: row.entry_id.clone(),
                    event_slug: row.entry_id.split(':').next().unwrap_or("").to_string(),
                    city: row.city.clone(),
                    target_date,
                    icao: row.icao.clone(),
                    resolution_source: row.resolution_source.clone(),
                    question: row.question.clone(),
                    direction: row.direction.clone(),
                    token_id: row.token_id.clone(),
                    size_usdc: row.size_usdc,
                    entry_price: row.entry_price,
                    shares: if row.entry_price > 0.0 {
                        (row.size_usdc / row.entry_price).round() as u32
                    } else {
                        0
                    },
                    predicted_temp: row.predicted_temp,
                    unit,
                    confidence: row.confidence,
                    effective_confidence: row.effective_confidence.unwrap_or(row.confidence),
                    expected_value: row.expected_value.unwrap_or(0.0),
                    edge_per_share: row.edge_per_share.unwrap_or(0.0),
                    strategy_type: row.strategy_type.clone().unwrap_or_else(|| "legacy".into()),
                },
            ))
        })
        .collect()
}

fn log_entry(
    position: &PositionRecord,
    decision_reason: &str,
    change_reason: &str,
    run_mode: BotRunMode,
    dry_run: bool,
) {
    let prefix = match run_mode {
        BotRunMode::SimMonitor => "ENTRY",
        BotRunMode::Bot => "ORDER",
    };
    let dry = if dry_run { " [DRY RUN]" } else { "" };
    println!(
        "-> {prefix} [{date}] {city:<18} {dir:<7} {shares:>4} @ ${price:.4} (${cost:.2}){dry}",
        date = position.target_date,
        city = truncate(&position.city, 18),
        dir = if position.direction == "BUY_YES" {
            "BUY YES"
        } else {
            "BUY NO"
        },
        shares = position.shares,
        price = position.entry_price,
        cost = position.size_usdc,
        dry = dry,
    );
    println!(
        "          Prev={:.1}{} ConfBase={:.1}% ConfEff={:.1}% EV={:+.2}$ Edge/share={:+.3} [{}]",
        position.predicted_temp,
        position.unit.symbol(),
        position.confidence * 100.0,
        position.effective_confidence * 100.0,
        position.expected_value,
        position.edge_per_share,
        position.strategy_type,
    );
    println!(
        "          change={} | {}",
        change_reason,
        truncate(decision_reason, 110)
    );
}

fn log_resolution(
    position: &PositionRecord,
    won: bool,
    pnl: f64,
    actual_temp: Option<f64>,
    run_mode: BotRunMode,
) {
    let prefix = match run_mode {
        BotRunMode::SimMonitor => "RESOLVED",
        BotRunMode::Bot => "LIQUIDATED",
    };
    let actual = actual_temp
        .map(|t| format!("  Real={:.1}{}", t, position.unit.symbol()))
        .unwrap_or_default();
    if won {
        println!(
            "✅ {prefix} [{date}] {city:<18} WON  +${pnl:.2} ({shares} shares) \"{q}\"{actual}",
            date = position.target_date,
            city = truncate(&position.city, 18),
            pnl = pnl,
            shares = position.shares,
            q = truncate(&position.question, 55),
            actual = actual,
        );
    } else {
        println!(
            "❌ {prefix} [{date}] {city:<18} LOST -${cost:.2} ({shares} shares) \"{q}\"{actual}",
            date = position.target_date,
            city = truncate(&position.city, 18),
            cost = position.size_usdc,
            shares = position.shares,
            q = truncate(&position.question, 55),
            actual = actual,
        );
    }
}

fn print_sim_banner(cfg: &Config) {
    println!("-------------------------------------------------------");
    println!("  Polymarket Weather Sim-Monitor | EVENT-DRIVEN");
    println!(
        "  Confianca min: {:.0}% | Horizonte: {}d | Edge min: {:.1}c",
        cfg.min_confidence * 100.0,
        cfg.extended_horizon_days,
        cfg.edge_min * 100.0
    );
    println!(
        "  Discovery {}s | Weather D3/D2/D1/D0 = {}/{}/{}/{}s",
        cfg.discovery_refresh_secs,
        cfg.weather_poll_d3_secs,
        cfg.weather_poll_d2_secs,
        cfg.weather_poll_d1_secs,
        cfg.weather_intraday_poll_secs,
    );
    println!("-------------------------------------------------------");
}

fn print_bot_banner(cfg: &Config) {
    println!("-------------------------------------------------------");
    println!("  Polymarket Weather Bot | EVENT-DRIVEN");
    println!(
        "  live_trading={} | confianca min: {:.0}% | edge min: {:.1}c",
        cfg.live_trading_enabled(),
        cfg.min_confidence * 100.0,
        cfg.edge_min * 100.0
    );
    println!(
        "  discovery {}s | intraday {}s | spread max {:.1}c",
        cfg.discovery_refresh_secs, cfg.weather_intraday_poll_secs, cfg.max_spread_cents,
    );
    println!("-------------------------------------------------------");
}

fn weather_row_from_position(position: &PositionRecord) -> WeatherTradeRow {
    WeatherTradeRow {
        entry_id: position.market_key.clone(),
        city: position.city.clone(),
        target_date: position.target_date.to_string(),
        icao: position.icao.clone(),
        resolution_source: position.resolution_source.clone(),
        question: position.question.clone(),
        direction: position.direction.clone(),
        token_id: position.token_id.clone(),
        size_usdc: position.size_usdc,
        entry_price: position.entry_price,
        predicted_temp: position.predicted_temp,
        temp_unit: position.unit.symbol().to_string(),
        confidence: position.confidence,
        effective_confidence: Some(position.effective_confidence),
        expected_value: Some(position.expected_value),
        edge_per_share: Some(position.edge_per_share),
        strategy_type: Some(position.strategy_type.clone()),
        execution_mode: "paper".into(),
        order_id: None,
        status: "pending".into(),
        actual_temp: None,
        pnl: None,
        resolved_at: None,
        winning_question: None,
        registered_at: Local::now().to_rfc3339(),
        created_at: Local::now().timestamp(),
    }
}

fn print_book_report(rows: &[WeatherTradeRow]) {
    let pending = rows.iter().filter(|e| e.status == "pending").count();
    let resolved: Vec<&WeatherTradeRow> = rows
        .iter()
        .filter(|e| e.status == "won" || e.status == "lost")
        .collect();
    let wins = resolved.iter().filter(|e| e.status == "won").count();
    let total_pnl: f64 = resolved.iter().filter_map(|e| e.pnl).sum();
    let total_ev: f64 = rows.iter().filter_map(|e| e.expected_value).sum();

    println!("Trades totais: {}", rows.len());
    println!("Pendentes: {}", pending);
    println!("Resolvidos: {}", resolved.len());
    println!("Wins: {}", wins);
    if !resolved.is_empty() {
        println!(
            "Win rate: {:.1}%",
            wins as f64 * 100.0 / resolved.len() as f64
        );
    }
    println!("PnL total: {:+.2} USDC", total_pnl);
    println!("EV acumulado na entrada: {:+.2} USDC", total_ev);

    let mut by_strategy: HashMap<String, (usize, usize, f64)> = HashMap::new();
    for row in rows {
        let strategy = row.strategy_type.clone().unwrap_or_else(|| "legacy".into());
        let entry = by_strategy.entry(strategy).or_insert((0, 0, 0.0));
        entry.0 += 1;
        if row.status == "won" {
            entry.1 += 1;
        }
        entry.2 += row.pnl.unwrap_or(0.0);
    }

    let mut items: Vec<_> = by_strategy.into_iter().collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    for (strategy, (trades, wins, pnl)) in items {
        let win_rate = if trades > 0 {
            wins as f64 * 100.0 / trades as f64
        } else {
            0.0
        };
        println!(
            "  {:18} {:4} trades  {:5.1}% win  PnL {:+8.2}",
            strategy, trades, win_rate, pnl
        );
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!(
            "{}...",
            s.chars().take(max.saturating_sub(3)).collect::<String>()
        )
    }
}

fn decimal_to_f64(value: Decimal) -> f64 {
    value.to_string().parse::<f64>().unwrap_or(0.0)
}
