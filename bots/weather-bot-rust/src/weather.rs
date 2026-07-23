// src/weather.rs
//! Cliente de clima multi-fonte:
//! - Open-Meteo (GFS, ICON, ECMWF, Ensemble) para previsão global
//! - Aviation Weather TAF — previsão oficial de aeroporto
//! - Wunderground forecast — mesma fonte da resolução Polymarket
//! - METAR (Aviation Weather) para observações intradiárias

use anyhow::{Context, Result};
use chrono::{Datelike, Local, NaiveDate, Timelike, Utc};
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::markets::{bias_correction_celsius, TempMarket, TempUnit};
pub use crate::types::Forecast;
use crate::types::{HourlySnapshot, SourceForecast, WeatherSource};

#[derive(Deserialize)]
struct ApiResponse {
    daily: DailyData,
}

#[derive(Deserialize)]
struct DailyData {
    time: Vec<String>,
    temperature_2m_max: Vec<Option<f64>>,
}

#[derive(Deserialize)]
struct HourlyApiResponse {
    hourly: HourlyData,
}

#[derive(Deserialize)]
struct HourlyData {
    time: Vec<String>,
    temperature_2m: Vec<Option<f64>>,
}

pub struct WeatherClient {
    http: reqwest::Client,
    weather_com_api_key: Option<String>,
}

impl WeatherClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("polymarket-weather-bot/2.0")
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .context("Falha ao criar HTTP client de clima")?;
        let weather_com_api_key = std::env::var("WEATHER_COM_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty());
        Ok(Self {
            http,
            weather_com_api_key,
        })
    }

    // ── Métodos de fetch multi-fonte ───────────────────────────────────────

    /// Busca previsões de TODAS as fontes disponíveis para o mercado em paralelo.
    /// Retorna as fontes que respondem com sucesso (falhas individuais são ignoradas).
    pub async fn fetch_all_sources(&self, market: &TempMarket) -> Vec<SourceForecast> {
        let today = Local::now().date_naive();
        let target_date = market.target_date.unwrap_or(today);
        let days_ahead = (target_date - today).num_days().max(0) as usize;

        // Lança todos os fetches em paralelo
        let gfs_fut = self.fetch_open_meteo_model(market, "gfs_seamless", days_ahead);
        let icon_fut = self.fetch_open_meteo_model(market, "icon_seamless", days_ahead);
        let ecmwf_fut = self.fetch_open_meteo_model(market, "ecmwf_ifs04", days_ahead);
        let ensemble_fut = self.fetch_open_meteo_ensemble(market, days_ahead);
        let taf_fut = self.fetch_taf_forecast(&market.icao, target_date, market.unit);
        let wg_fut = self.fetch_wunderground_forecast(
            &market.icao,
            market.station_lat,
            market.station_lon,
            target_date,
            market.unit,
        );

        let (gfs, icon, ecmwf, ensemble, taf, wg) =
            tokio::join!(gfs_fut, icon_fut, ecmwf_fut, ensemble_fut, taf_fut, wg_fut);

        let mut sources = Vec::new();
        if let Ok(Some(s)) = gfs {
            sources.push(s);
        }
        if let Ok(Some(s)) = icon {
            sources.push(s);
        }
        if let Ok(Some(s)) = ecmwf {
            sources.push(s);
        }
        if let Ok(Some(s)) = ensemble {
            sources.push(s);
        }
        if let Ok(Some(s)) = taf {
            sources.push(s);
        }
        if let Ok(Some(s)) = wg {
            sources.push(s);
        }

        info!(
            "[ICAO={}] Fontes obtidas: {} ({}) para D+{}",
            market.icao,
            sources.len(),
            sources
                .iter()
                .map(|s| s.source.name())
                .collect::<Vec<_>>()
                .join(", "),
            days_ahead
        );

        sources
    }

    /// Busca previsão do Open-Meteo para um modelo específico.
    async fn fetch_open_meteo_model(
        &self,
        market: &TempMarket,
        model: &str,
        days_ahead: usize,
    ) -> Result<Option<SourceForecast>> {
        let today = Local::now().date_naive();
        let target_date = market.target_date.unwrap_or(today);
        let forecast_days = (days_ahead + 2).max(3).min(16);

        let url = format!(
            "https://api.open-meteo.com/v1/forecast\
             ?latitude={lat}&longitude={lon}\
             &daily=temperature_2m_max\
             &temperature_unit={unit}\
             &timezone=auto\
             &forecast_days={days}\
             &models={model}",
            lat = market.station_lat,
            lon = market.station_lon,
            unit = market.unit.open_meteo_str(),
            days = forecast_days,
            model = model,
        );

        debug!("[{}] Open-Meteo/{}: {}", market.icao, model, url);

        let resp = match self.http.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                warn!(
                    "[{}] Open-Meteo/{} status {}",
                    market.icao,
                    model,
                    r.status()
                );
                return Ok(None);
            }
            Err(e) => {
                warn!("[{}] Open-Meteo/{} erro: {}", market.icao, model, e);
                return Ok(None);
            }
        };

        let parsed: ApiResponse = resp.json().await.with_context(|| {
            format!("Falha ao parsear Open-Meteo/{} para {}", model, market.icao)
        })?;

        let target_str = target_date.format("%Y-%m-%d").to_string();
        let idx = parsed
            .daily
            .time
            .iter()
            .position(|d| d.starts_with(&target_str))
            .unwrap_or(0);

        let raw_temp = match parsed.daily.temperature_2m_max.get(idx).copied().flatten() {
            Some(t) => t,
            None => return Ok(None),
        };

        let bias_c = bias_correction_celsius(&market.icao);
        let bias = if market.unit == TempUnit::Fahrenheit {
            bias_c * 9.0 / 5.0
        } else {
            bias_c
        };

        let source_kind = match model {
            "gfs_seamless" => WeatherSource::OpenMeteoGfs,
            "icon_seamless" => WeatherSource::OpenMeteoIcon,
            "ecmwf_ifs04" => WeatherSource::OpenMeteoEcmwf,
            _ => WeatherSource::OpenMeteoGfs,
        };

        // Incerteza intrínseca cresce com o horizonte de previsão
        let uncertainty = match days_ahead {
            0 => 0.5,
            1 => 1.0,
            2 => 1.5,
            _ => 2.0,
        };

        Ok(Some(SourceForecast {
            source: source_kind,
            predicted_max: raw_temp + bias,
            uncertainty,
        }))
    }

    /// Busca previsão Ensemble do Open-Meteo (fornece percentis P25/P75 para spread).
    async fn fetch_open_meteo_ensemble(
        &self,
        market: &TempMarket,
        days_ahead: usize,
    ) -> Result<Option<SourceForecast>> {
        let today = Local::now().date_naive();
        let target_date = market.target_date.unwrap_or(today);
        let forecast_days = (days_ahead + 2).max(3).min(16);
        let target_str = target_date.format("%Y-%m-%d").to_string();

        let url = format!(
            "https://ensemble-api.open-meteo.com/v1/ensemble\
             ?latitude={lat}&longitude={lon}\
             &daily=temperature_2m_max\
             &temperature_unit={unit}\
             &timezone=auto\
             &forecast_days={days}\
             &models=gfs025_ensemble",
            lat = market.station_lat,
            lon = market.station_lon,
            unit = market.unit.open_meteo_str(),
            days = forecast_days,
        );

        debug!("[{}] Open-Meteo/Ensemble: {}", market.icao, url);

        let resp = match self.http.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                warn!("[{}] Ensemble status {}", market.icao, r.status());
                return Ok(None);
            }
            Err(e) => {
                warn!("[{}] Ensemble erro: {}", market.icao, e);
                return Ok(None);
            }
        };

        let body = resp.text().await?;
        let parsed: Value = serde_json::from_str(&body)?;

        let Some(daily) = parsed.get("daily") else {
            return Ok(None);
        };
        let Some(times) = daily.get("time").and_then(|v| v.as_array()) else {
            return Ok(None);
        };
        let Some(idx) = times.iter().position(|t| {
            t.as_str()
                .map(|s| s.starts_with(&target_str))
                .unwrap_or(false)
        }) else {
            return Ok(None);
        };

        // A resposta ensemble tem arrays nomeados por membro: temperature_2m_max_member01, etc.
        // Coletamos todos os valores para calcular a média e spread
        let mut member_temps: Vec<f64> = Vec::new();
        if let Some(obj) = daily.as_object() {
            for (key, val) in obj {
                if key.starts_with("temperature_2m_max") && key != "time" {
                    if let Some(arr) = val.as_array() {
                        if let Some(t) = arr.get(idx).and_then(|v| v.as_f64()) {
                            member_temps.push(t);
                        }
                    }
                }
            }
        }

        if member_temps.is_empty() {
            return Ok(None);
        }

        let mean = member_temps.iter().sum::<f64>() / member_temps.len() as f64;
        let spread = {
            let max = member_temps
                .iter()
                .cloned()
                .fold(f64::NEG_INFINITY, f64::max);
            let min = member_temps.iter().cloned().fold(f64::INFINITY, f64::min);
            (max - min).max(0.0)
        };

        let bias_c = bias_correction_celsius(&market.icao);
        let bias = if market.unit == TempUnit::Fahrenheit {
            bias_c * 9.0 / 5.0
        } else {
            bias_c
        };

        Ok(Some(SourceForecast {
            source: WeatherSource::OpenMeteoEnsemble,
            predicted_max: mean + bias,
            // Spread do ensemble como estimativa de incerteza (clampado entre 0.5 e 3.0)
            uncertainty: (spread / 2.0).clamp(0.5, 3.0),
        }))
    }

    /// Busca previsão de temperatura do TAF (Terminal Aerodrome Forecast) para o aeroporto.
    ///
    /// O TAF é o forecast oficial emitido pela estação meteorológica do aeroporto —
    /// a mesma estação usada para resolução dos mercados Polymarket.
    /// Nota: temperatura em TAFs é opcional; nem todos os aeroportos incluem.
    async fn fetch_taf_forecast(
        &self,
        icao: &str,
        target_date: NaiveDate,
        unit: TempUnit,
    ) -> Result<Option<SourceForecast>> {
        let url =
            format!("https://aviationweather.gov/api/data/taf?ids={icao}&format=json&metar=false");

        debug!("[{}] TAF forecast: {}", icao, url);

        let resp = match self.http.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                warn!("[{}] TAF status {}", icao, r.status());
                return Ok(None);
            }
            Err(e) => {
                warn!("[{}] TAF erro: {}", icao, e);
                return Ok(None);
            }
        };

        let body = resp.text().await?;
        let raw: Value = serde_json::from_str(&body).unwrap_or(Value::Null);

        // O TAF JSON pode ser um array ou um objeto com campo "data"
        let tafs = if let Some(arr) = raw.as_array() {
            arr.clone()
        } else if let Some(arr) = raw.get("data").and_then(|v| v.as_array()) {
            arr.clone()
        } else {
            return Ok(None);
        };

        // Procura temperaturas nos forecast periods do TAF para a data alvo
        let target_ts_start = target_date
            .and_hms_opt(0, 0, 0)
            .and_then(|dt| dt.and_local_timezone(Utc).single())
            .map(|dt| dt.timestamp())
            .unwrap_or(0);
        let target_ts_end = target_ts_start + 86_400;

        let mut temps_c: Vec<f64> = Vec::new();

        for taf_entry in &tafs {
            let fcsts = taf_entry
                .get("fcsts")
                .or_else(|| taf_entry.get("forecast"))
                .and_then(|v| v.as_array());

            if let Some(fcsts) = fcsts {
                for fcst in fcsts {
                    let time_from = fcst
                        .get("timeFrom")
                        .or_else(|| fcst.get("valid_time_utc"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);

                    // Só considera forecasts dentro do dia alvo
                    if time_from < target_ts_start || time_from >= target_ts_end {
                        continue;
                    }

                    // Temperatura máxima no período
                    if let Some(temp_arr) = fcst.get("temp").and_then(|v| v.as_array()) {
                        for t in temp_arr {
                            if let Some(v) = t.get("temp").and_then(|v| v.as_f64()) {
                                temps_c.push(v);
                            }
                        }
                    }

                    // Alguns TAFs têm temp diretamente no período
                    if let Some(t) = fcst.get("temperature").and_then(|v| v.as_f64()) {
                        temps_c.push(t);
                    }
                    if let Some(t) = fcst.get("maxTemp").and_then(|v| v.as_f64()) {
                        temps_c.push(t);
                    }
                }
            }
        }

        if temps_c.is_empty() {
            debug!("[{}] TAF: sem temperatura para {}", icao, target_date);
            return Ok(None);
        }

        let max_c = temps_c.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let predicted_max = match unit {
            TempUnit::Celsius => max_c,
            TempUnit::Fahrenheit => max_c * 9.0 / 5.0 + 32.0,
        };

        debug!(
            "[{}] TAF max={:.1}{} (de {} leituras)",
            icao,
            predicted_max,
            unit.symbol(),
            temps_c.len()
        );

        Ok(Some(SourceForecast {
            source: WeatherSource::AviationWeatherTaf,
            predicted_max,
            uncertainty: 1.0, // TAF tem horizonte curto, geralmente ±1°C
        }))
    }

    /// Busca previsão do Wunderground para o aeroporto ICAO.
    ///
    /// O Wunderground é a fonte oficial de resolução da Polymarket para mercados de temperatura.
    /// Esta função extrai a previsão de temperatura máxima para o dia seguinte via API interna.
    async fn fetch_wunderground_forecast(
        &self,
        icao: &str,
        lat: f64,
        lon: f64,
        target_date: NaiveDate,
        unit: TempUnit,
    ) -> Result<Option<SourceForecast>> {
        let Some(api_key) = self.weather_com_api_key.as_deref() else {
            debug!(
                "[{}] Wunderground ignorado: WEATHER_COM_API_KEY ausente",
                icao
            );
            return Ok(None);
        };
        // Wunderground usa a API interna do Weather Company (IBM/TWC)
        // A URL usa geocoords e retorna 7-10 dias de forecast
        let unit_str = match unit {
            TempUnit::Celsius => "m",
            TempUnit::Fahrenheit => "e",
        };
        let url = format!(
            "https://api.weather.com/v3/wx/forecast/daily/10day\
             ?geocode={lat:.4},{lon:.4}\
             &format=json\
             &units={unit}\
             &language=en-US\
             &apiKey={api_key}",
            lat = lat,
            lon = lon,
            unit = unit_str,
            api_key = api_key,
        );

        debug!(
            "[{}] Wunderground forecast: lat={:.4} lon={:.4}",
            icao, lat, lon
        );

        let resp = match self
            .http
            .get(&url)
            .header("Referer", "https://www.wunderground.com/")
            .header("Origin", "https://www.wunderground.com")
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                warn!("[{}] Wunderground forecast status {}", icao, r.status());
                return Ok(None);
            }
            Err(e) => {
                warn!("[{}] Wunderground forecast erro: {}", icao, e);
                return Ok(None);
            }
        };

        let body = resp.text().await?;
        let raw: Value = serde_json::from_str(&body).unwrap_or(Value::Null);

        // A resposta tem arrays "temperatureMax", "validTimeUtc", etc.
        let valid_times_local = raw
            .get("validTimeLocal")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let valid_times = raw
            .get("validTimeUtc")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let temps = raw
            .get("temperatureMax")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let today = Local::now().date_naive();
        let days_ahead = (target_date - today).num_days().max(0) as usize;

        // Encontra o índice correspondente ao dia alvo
        let target_ts_start = target_date
            .and_hms_opt(0, 0, 0)
            .and_then(|dt| dt.and_local_timezone(Utc).single())
            .map(|dt| dt.timestamp())
            .unwrap_or(0);
        let target_ts_end = target_ts_start + 86_400;

        let target_day = target_date.format("%Y-%m-%d").to_string();
        let idx = valid_times_local
            .iter()
            .position(|t| {
                t.as_str()
                    .map(|value| value.starts_with(&target_day))
                    .unwrap_or(false)
            })
            .or_else(|| {
                valid_times.iter().position(|t| {
                    t.as_i64()
                        .map(|ts| ts >= target_ts_start && ts < target_ts_end)
                        .unwrap_or(false)
                })
            });

        // Fallback: usa o índice por dias_ahead se não encontrou pelo timestamp
        let idx = idx.unwrap_or(days_ahead);

        let predicted_max = match temps.get(idx).and_then(|v| v.as_f64()) {
            Some(t) => t,
            None => {
                debug!(
                    "[{}] Wunderground: sem temperatura no índice {} (total: {})",
                    icao,
                    idx,
                    temps.len()
                );
                return Ok(None);
            }
        };

        debug!(
            "[{}] Wunderground forecast: {:.1}{} (idx={})",
            icao,
            predicted_max,
            unit.symbol(),
            idx
        );

        Ok(Some(SourceForecast {
            source: WeatherSource::WundergroundForecast,
            predicted_max,
            uncertainty: match days_ahead {
                0 => 0.5,
                1 => 1.0,
                _ => 1.5,
            },
        }))
    }

    // ── Métodos legados (Open-Meteo padrão + METAR) ────────────────────────

    /// Busca a maxima prevista para a data alvo usando Open-Meteo (modelo padrão).
    /// Mantido para compatibilidade com o pipeline existente de trend/intraday.
    pub async fn fetch_for_market(&self, market: &TempMarket) -> Result<Option<Forecast>> {
        let today = Local::now().date_naive();
        let target_date = market.target_date.unwrap_or(today);
        let days_ahead = (target_date - today).num_days().max(0) as usize;
        let forecast_days = (days_ahead + 2).max(3).min(16);

        let url = format!(
            "https://api.open-meteo.com/v1/forecast\
             ?latitude={lat}&longitude={lon}\
             &daily=temperature_2m_max\
             &temperature_unit={unit}\
             &timezone=auto\
             &forecast_days={days}",
            lat = market.station_lat,
            lon = market.station_lon,
            unit = market.unit.open_meteo_str(),
            days = forecast_days,
        );

        debug!("[{}] Forecast (D+{}): {}", market.icao, days_ahead, url);

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Falha Open-Meteo forecast ICAO={}", market.icao))?
            .error_for_status()?
            .json::<ApiResponse>()
            .await?;

        let target_str = target_date.format("%Y-%m-%d").to_string();
        let idx = resp
            .daily
            .time
            .iter()
            .position(|d| d.starts_with(&target_str))
            .unwrap_or(0);

        let raw_temp = match resp.daily.temperature_2m_max.get(idx).copied().flatten() {
            Some(t) => t,
            None => {
                warn!(
                    "[{}] Temp prevista nao disponivel para {}",
                    market.icao, target_str
                );
                return Ok(None);
            }
        };

        let bias_c = bias_correction_celsius(&market.icao);
        let bias = if market.unit == TempUnit::Fahrenheit {
            bias_c * 9.0 / 5.0
        } else {
            bias_c
        };
        let max_temp = raw_temp + bias;
        let confidence = horizon_confidence(days_ahead);

        info!(
            "[ICAO={}] D+{} | Prev={:.1}{} (raw={:.1}{}+bias{:+.1}) | Conf={:.1}% | coords=({:.4},{:.4}) | data={}",
            market.icao,
            days_ahead,
            max_temp,
            market.unit.symbol(),
            raw_temp,
            market.unit.symbol(),
            bias,
            confidence * 100.0,
            market.station_lat,
            market.station_lon,
            target_str,
        );

        Ok(Some(Forecast {
            icao: market.icao.clone(),
            max_temp,
            unit: market.unit,
            confidence,
            uncertainty: match days_ahead {
                0 => 0.5,
                1 => 1.0,
                2 => 1.5,
                _ => 2.0,
            },
        }))
    }

    /// Busca observacoes horarias ja ocorridas do dia atual via Open-Meteo.
    pub async fn fetch_hourly_today(&self, market: &TempMarket) -> Result<Vec<HourlySnapshot>> {
        let today = Local::now().date_naive();
        let d = today.format("%Y-%m-%d").to_string();
        let current_hour_utc = Utc::now().hour() as u8;

        let url = format!(
            "https://api.open-meteo.com/v1/forecast\
             ?latitude={lat}&longitude={lon}\
             &hourly=temperature_2m\
             &temperature_unit={unit}\
             &timezone=UTC\
             &start_date={d}&end_date={d}",
            lat = market.station_lat,
            lon = market.station_lon,
            unit = market.unit.open_meteo_str(),
            d = d,
        );

        debug!("[{}] Hourly today: {}", market.icao, url);

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Falha Open-Meteo hourly ICAO={}", market.icao))?;

        if !resp.status().is_success() {
            warn!(
                "[{}] Open-Meteo intradiario indisponivel (status {}) - fallback METAR oficial",
                market.icao,
                resp.status()
            );
            return self
                .fetch_official_hourly_today(&market.icao, market.unit)
                .await;
        }

        let resp = resp.error_for_status()?.json::<HourlyApiResponse>().await?;

        let bias_c = bias_correction_celsius(&market.icao);
        let bias = if market.unit == TempUnit::Fahrenheit {
            bias_c * 9.0 / 5.0
        } else {
            bias_c
        };

        let mut snapshots = Vec::new();
        for (i, time_str) in resp.hourly.time.iter().enumerate() {
            let hour_utc = time_str
                .split('T')
                .nth(1)
                .and_then(|t| t.split(':').next())
                .and_then(|h| h.parse::<u8>().ok())
                .unwrap_or(255);

            if hour_utc > current_hour_utc {
                break;
            }

            if let Some(Some(raw)) = resp.hourly.temperature_2m.get(i) {
                snapshots.push(HourlySnapshot {
                    hour_utc,
                    temp: raw + bias,
                });
            }
        }

        debug!(
            "[{}] Horario: {} snapshots ate {:02}:00 UTC | max={:.1}{}",
            market.icao,
            snapshots.len(),
            current_hour_utc,
            snapshots
                .iter()
                .map(|s| s.temp)
                .fold(f64::NEG_INFINITY, f64::max),
            market.unit.symbol(),
        );

        Ok(snapshots)
    }

    /// Busca observacoes oficiais METAR da propria estacao ICAO.
    ///
    /// Usado apenas como confirmacao/fallback intradiario quando o Open-Meteo
    /// hourly falha. Como o mercado resolve na estacao do aeroporto, usar o
    /// mesmo ICAO e uma fonte oficial padronizada e confiavel.
    async fn fetch_official_hourly_today(
        &self,
        icao: &str,
        unit: TempUnit,
    ) -> Result<Vec<HourlySnapshot>> {
        let url =
            format!("https://aviationweather.gov/api/data/metar?ids={icao}&format=json&hours=24");

        debug!("[{}] METAR official fallback: {}", icao, url);

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Falha METAR oficial ICAO={}", icao))?;

        if !resp.status().is_success() {
            return Err(anyhow::anyhow!(
                "METAR oficial falhou com status {} para {}",
                resp.status(),
                icao
            ));
        }

        let body = resp
            .text()
            .await
            .with_context(|| format!("Falha ao ler METAR oficial ICAO={}", icao))?;

        let raw: Value = serde_json::from_str(&body)
            .with_context(|| format!("Falha parse METAR oficial ICAO={}", icao))?;

        let today = Utc::now().date_naive();
        let mut snapshots = Vec::new();

        let arr = raw.as_array().cloned().unwrap_or_default();
        for item in arr {
            let obs_time = item
                .get("obsTime")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("observationTime").and_then(|v| v.as_str()))
                .or_else(|| item.get("reportTime").and_then(|v| v.as_str()));

            let Some(obs_time) = obs_time else {
                continue;
            };

            let parsed = chrono::DateTime::parse_from_rfc3339(obs_time)
                .ok()
                .map(|dt| dt.with_timezone(&Utc));
            let Some(parsed) = parsed else {
                continue;
            };

            if parsed.date_naive() != today {
                continue;
            }

            let temp_c = item
                .get("temp")
                .and_then(|v| v.as_f64())
                .or_else(|| item.get("tempC").and_then(|v| v.as_f64()))
                .or_else(|| item.get("temperature").and_then(|v| v.as_f64()));

            let Some(temp_c) = temp_c else {
                continue;
            };

            let temp = match unit {
                TempUnit::Celsius => temp_c,
                TempUnit::Fahrenheit => temp_c * 9.0 / 5.0 + 32.0,
            };

            snapshots.push(HourlySnapshot {
                hour_utc: parsed.hour() as u8,
                temp,
            });
        }

        if snapshots.is_empty() {
            return Err(anyhow::anyhow!(
                "Sem observacoes METAR oficiais do dia para {}",
                icao
            ));
        }

        snapshots.sort_by_key(|s| s.hour_utc);
        Ok(snapshots)
    }

    /// Busca dado historico via Open-Meteo Archive.
    pub async fn fetch_historical(
        &self,
        lat: f64,
        lon: f64,
        date: NaiveDate,
        unit: TempUnit,
        icao: &str,
    ) -> Result<Option<Forecast>> {
        let d = date.format("%Y-%m-%d").to_string();

        let url = format!(
            "https://archive-api.open-meteo.com/v1/archive\
             ?latitude={lat}&longitude={lon}\
             &start_date={d}&end_date={d}\
             &daily=temperature_2m_max\
             &temperature_unit={unit}\
             &timezone=auto",
            lat = lat,
            lon = lon,
            d = d,
            unit = unit.open_meteo_str(),
        );

        debug!("[{}] Archive: {}", icao, url);

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Falha Open-Meteo archive ICAO={}", icao))?
            .error_for_status()?
            .json::<ApiResponse>()
            .await?;

        let max_temp = match resp.daily.temperature_2m_max.into_iter().next().flatten() {
            Some(t) => t,
            None => {
                warn!("[{}] Temp historica nao disponivel para {}", icao, d);
                return Ok(None);
            }
        };

        Ok(Some(Forecast {
            icao: icao.to_string(),
            max_temp,
            unit,
            confidence: 1.0,
            uncertainty: 0.25,
        }))
    }

    /// Busca a maxima oficial do dia no Wunderground, fonte de resolucao da Polymarket.
    pub async fn fetch_wunderground_historical(
        &self,
        history_url: &str,
        date: NaiveDate,
        unit: TempUnit,
        icao: &str,
    ) -> Result<Option<Forecast>> {
        let url = wunderground_daily_url(history_url, date);
        debug!("[{}] Wunderground: {}", icao, url);

        let body = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Falha Wunderground ICAO={}", icao))?
            .error_for_status()?
            .text()
            .await
            .with_context(|| format!("Falha ao ler HTML Wunderground ICAO={}", icao))?;

        let max_temp = match parse_wunderground_max_temp(&body, unit) {
            Some(t) => t,
            None => {
                warn!(
                    "[{}] Maxima oficial nao encontrada no Wunderground para {}",
                    icao,
                    date.format("%Y-%m-%d")
                );
                return Ok(None);
            }
        };

        Ok(Some(Forecast {
            icao: icao.to_string(),
            max_temp,
            unit,
            confidence: 1.0,
            uncertainty: 0.25,
        }))
    }
}

fn horizon_confidence(days_ahead: usize) -> f64 {
    match days_ahead {
        0 => 0.80,
        1 => 0.72,
        2 => 0.64,
        _ => 0.56,
    }
}

fn wunderground_daily_url(history_url: &str, date: NaiveDate) -> String {
    let base = history_url.trim_end_matches('/');
    let date_path = format!("date/{}-{}-{}", date.year(), date.month(), date.day());

    if base.contains("/date/") {
        let prefix = base
            .split("/date/")
            .next()
            .unwrap_or(base)
            .trim_end_matches('/');
        format!("{prefix}/{date_path}")
    } else {
        format!("{base}/{date_path}")
    }
}

fn parse_wunderground_max_temp(body: &str, unit: TempUnit) -> Option<f64> {
    parse_wunderground_summary_value(body)
        .or_else(|| parse_wunderground_observation_max(body))
        .map(|v| round_for_market(v, unit))
}

fn parse_wunderground_summary_value(body: &str) -> Option<f64> {
    for pattern in [
        r#""temperatureMax"\s*:\s*\{\s*"value"\s*:\s*(-?\d+(?:\.\d+)?)"#,
        r#""temperatureMax"\s*:\s*(-?\d+(?:\.\d+)?)"#,
        r#""tempHigh"\s*:\s*(-?\d+(?:\.\d+)?)"#,
        r#""temperatureHigh"\s*:\s*(-?\d+(?:\.\d+)?)"#,
    ] {
        if let Some(v) = capture_first_number(body, pattern) {
            return Some(v);
        }
    }
    None
}

fn parse_wunderground_observation_max(body: &str) -> Option<f64> {
    let mut values = Vec::new();

    for pattern in [
        r#""max_temp"\s*:\s*(-?\d+(?:\.\d+)?)"#,
        r#""temp"\s*:\s*(-?\d+(?:\.\d+)?)"#,
        r#""temperature"\s*:\s*(-?\d+(?:\.\d+)?)"#,
    ] {
        if let Ok(re) = Regex::new(pattern) {
            for cap in re.captures_iter(body) {
                if let Some(v) = cap.get(1).and_then(|m| m.as_str().parse::<f64>().ok()) {
                    values.push(v);
                }
            }
        }
    }

    values.into_iter().reduce(f64::max)
}

fn capture_first_number(body: &str, pattern: &str) -> Option<f64> {
    Regex::new(pattern)
        .ok()
        .and_then(|re| re.captures(body))
        .and_then(|cap| cap.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok())
}

fn round_for_market(value: f64, _unit: TempUnit) -> f64 {
    value.round()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_wunderground_date_url() {
        let d = NaiveDate::from_ymd_opt(2026, 4, 5).unwrap();
        assert_eq!(
            wunderground_daily_url(
                "https://www.wunderground.com/history/daily/cn/shanghai/ZSPD",
                d
            ),
            "https://www.wunderground.com/history/daily/cn/shanghai/ZSPD/date/2026-4-5"
        );
    }

    #[test]
    fn parses_summary_object() {
        let body = r#"{"temperatureMax":{"value":26.4,"unit":"C"}}"#;
        assert_eq!(
            parse_wunderground_max_temp(body, TempUnit::Celsius),
            Some(26.0)
        );
    }

    #[test]
    fn parses_observation_max() {
        let body = r#"{"observations":[{"temp":70},{"temp":74},{"temp":72}]}"#;
        assert_eq!(
            parse_wunderground_max_temp(body, TempUnit::Fahrenheit),
            Some(74.0)
        );
    }
}
