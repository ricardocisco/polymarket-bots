// src/markets.rs
//! Descoberta de mercados de temperatura via Polymarket Gamma API.
//!
//! LÓGICA CENTRAL:
//!   A Polymarket resolve cada mercado pela temperatura da ESTAÇÃO DE AEROPORTO
//!   mencionada na descrição. Este módulo:
//!     1. Busca eventos ativos/fechados na Gamma API
//!     2. Lê a descrição de cada evento
//!     3. Extrai o código ICAO do link Wunderground na descrição
//!        Ex: "wunderground.com/history/daily/br/guarulhos/SBGR" → "SBGR"
//!     4. Resolve as coordenadas do aeroporto via Open-Meteo Geocoding
//!     5. Detecta a unidade (°C ou °F) pela descrição
//!
//! Resultado: TempMarket com todos os dados necessários para previsão e trading.

use anyhow::{Context, Result};
use chrono::NaiveDate;
use serde::Deserialize;
use tracing::{debug, info, warn};

// ── Deserializador para campos que chegam como string JSON ou array ──────────

fn deser_str_or_vec<'de, D>(d: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let raw: Option<serde_json::Value> = Option::deserialize(d)?;
    match raw {
        None => Ok(None),
        Some(serde_json::Value::Array(arr)) => arr
            .into_iter()
            .map(|v| match v {
                serde_json::Value::String(s) => Ok(s),
                other => Ok(other.to_string()),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(serde_json::Value::String(s)) => serde_json::from_str::<Vec<String>>(&s)
            .map(Some)
            .map_err(D::Error::custom),
        Some(other) => Err(D::Error::custom(format!(
            "expected array or string, got {}",
            other
        ))),
    }
}

// ── Tipo público: mercado de temperatura pronto para análise ──

#[derive(Debug, Clone)]
pub struct TempMarket {
    /// Token ID do YES no CLOB (necessário para colocar ordem)
    pub yes_token_id: String,
    /// Token ID do NO no CLOB
    pub no_token_id: String,
    /// Texto da pergunta, ex: "Will the highest temp in London be 15°C on March 9?"
    pub question: String,
    /// Slug do evento pai, ex: "highest-temperature-in-london-on-march-9-2026"
    pub event_slug: String,
    /// Preço atual do YES (0.0–1.0), ex: 0.97 = 97%
    pub yes_price: f64,
    /// Preço atual do NO (0.0–1.0)
    pub no_price: f64,
    /// Limite inferior do range de temperatura (None = sem limite)
    pub range_min: Option<f64>,
    /// Limite superior do range de temperatura (None = sem limite)
    pub range_max: Option<f64>,
    /// Tick size do mercado (geralmente "0.01")
    pub tick_size: String,
    /// Flag neg_risk — necessária para assinar ordens corretamente
    pub neg_risk: bool,
    /// Código ICAO da estação de aeroporto extraído da descrição
    pub icao: String,
    /// Latitude da estação (para Open-Meteo)
    pub station_lat: f64,
    /// Longitude da estação (para Open-Meteo)
    pub station_lon: f64,
    /// Unidade de temperatura usada neste mercado
    pub unit: TempUnit,
    /// Data-alvo do mercado (extraída do slug), ex: 2026-03-12
    /// Usada pelo fetcher de clima para buscar a previsão do dia correto.
    pub target_date: Option<NaiveDate>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TempUnit {
    Celsius,
    Fahrenheit,
}

impl TempUnit {
    pub fn symbol(self) -> &'static str {
        match self {
            TempUnit::Celsius => "°C",
            TempUnit::Fahrenheit => "°F",
        }
    }
    pub fn open_meteo_str(self) -> &'static str {
        match self {
            TempUnit::Celsius => "celsius",
            TempUnit::Fahrenheit => "fahrenheit",
        }
    }
}

// ── Structs de deserialização da Gamma API ────────────────────

#[derive(Deserialize)]
struct GammaEvent {
    slug: String,
    description: Option<String>,
    markets: Option<Vec<GammaMarket>>,
}

#[derive(Deserialize)]
struct GammaMarket {
    question: String,
    active: Option<bool>,
    closed: Option<bool>,
    #[serde(
        default,
        rename = "clobTokenIds",
        deserialize_with = "deser_str_or_vec"
    )]
    clob_token_ids: Option<Vec<String>>,
    #[serde(
        default,
        rename = "outcomePrices",
        deserialize_with = "deser_str_or_vec"
    )]
    outcome_prices: Option<Vec<String>>,
    #[serde(rename = "negRisk")]
    neg_risk: Option<bool>,
}

// ── Cliente ───────────────────────────────────────────────────

pub struct GammaClient {
    http: reqwest::Client,
}

impl GammaClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("polymarket-weather-bot/1.0")
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .context("Falha ao criar HTTP client")?;
        Ok(Self { http })
    }

    /// Busca mercados ATIVOS de temperatura para um slug keyword.
    /// Usado pelo bot real e pelo setup.
    pub async fn fetch_markets(&self, slug_keyword: &str) -> Result<Vec<TempMarket>> {
        self.fetch_events(slug_keyword, true, false).await
    }

    /// Busca mercados RESOLVIDOS (fechados) para backtesting.
    pub async fn fetch_resolved_markets(
        &self,
        slug_keyword: &str,
        limit: u32,
    ) -> Result<Vec<TempMarket>> {
        let url = format!(
            "https://gamma-api.polymarket.com/events?slug={}&closed=true&limit={}",
            slug_keyword, limit
        );
        self.fetch_and_parse(&url, slug_keyword).await
    }

    async fn fetch_events(
        &self,
        slug_keyword: &str,
        active: bool,
        closed: bool,
    ) -> Result<Vec<TempMarket>> {
        let url = format!(
            "https://gamma-api.polymarket.com/events?slug={}&active={}&closed={}&limit=30",
            slug_keyword, active, closed
        );
        self.fetch_and_parse(&url, slug_keyword).await
    }

    async fn fetch_and_parse(&self, url: &str, slug_keyword: &str) -> Result<Vec<TempMarket>> {
        debug!("Gamma API: {}", url);

        let events: Vec<GammaEvent> = self
            .http
            .get(url)
            .send()
            .await
            .context("Falha ao conectar à Gamma API")?
            .error_for_status()
            .context("Gamma API retornou erro HTTP")?
            .json()
            .await
            .context("Falha ao parsear resposta da Gamma API")?;

        let mut result = Vec::new();

        for event in &events {
            if !event.slug.contains(slug_keyword) {
                continue;
            }

            let markets = match &event.markets {
                Some(m) if !m.is_empty() => m,
                _ => continue,
            };

            let description = event.description.as_deref().unwrap_or("");

            let icao = match extract_icao(description) {
                Some(code) => code,
                None => {
                    warn!(
                        "[{}] ICAO não encontrado. Descrição: '{}'",
                        event.slug,
                        description.chars().take(200).collect::<String>()
                    );
                    continue;
                }
            };

            let unit = detect_unit(description, markets);

            info!("[{}] ICAO={} | Unidade={}", event.slug, icao, unit.symbol());

            let (lat, lon) = match self.resolve_icao_coords(&icao).await {
                Ok(Some(c)) => c,
                Ok(None) => {
                    warn!("[{}] Coords não encontradas para ICAO={}", event.slug, icao);
                    continue;
                }
                Err(e) => {
                    warn!("[{}] Erro geocoding: {}", event.slug, e);
                    continue;
                }
            };

            info!("[{}] {} → lat={:.4} lon={:.4}", event.slug, icao, lat, lon);

            for mkt in markets {
                if mkt.active == Some(false) || mkt.closed == Some(true) {
                    continue;
                }

                let tokens = match &mkt.clob_token_ids {
                    Some(t) if t.len() >= 2 => t,
                    _ => {
                        warn!("Mercado sem tokens CLOB: '{}'", mkt.question);
                        continue;
                    }
                };

                let yes_price = mkt
                    .outcome_prices
                    .as_deref()
                    .and_then(|p| p.first())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.5);
                let no_price = mkt
                    .outcome_prices
                    .as_deref()
                    .and_then(|p| p.get(1))
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.5);

                let (range_min, range_max) = parse_temp_range(&mkt.question);

                result.push(TempMarket {
                    yes_token_id: tokens[0].clone(),
                    no_token_id: tokens[1].clone(),
                    question: mkt.question.clone(),
                    event_slug: event.slug.clone(),
                    yes_price,
                    no_price,
                    range_min,
                    range_max,
                    tick_size: "0.01".to_string(),
                    neg_risk: mkt.neg_risk.unwrap_or(false),
                    icao: icao.clone(),
                    station_lat: lat,
                    station_lon: lon,
                    unit,
                    target_date: extract_target_date_from_slug(&event.slug),
                });
            }
        }

        Ok(result)
    }

    async fn resolve_icao_coords(&self, icao: &str) -> Result<Option<(f64, f64)>> {
        // Primeiro tenta tabela estática
        if let Some(c) = icao_static_coords(icao) {
            return Ok(Some(c));
        }

        // Fallback: tenta geocoding pelo termo ICAO via Open-Meteo Geocoding
        let url = format!(
            "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en",
            icao
        );

        #[derive(Deserialize)]
        struct GeoRes {
            results: Option<Vec<GeoItem>>,
        }
        #[derive(Deserialize)]
        struct GeoItem {
            latitude: f64,
            longitude: f64,
            name: Option<String>,
            country: Option<String>,
        }

        let resp: GeoRes = match self.http.get(&url).send().await?.json().await {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };

        if let Some(mut list) = resp.results {
            if let Some(it) = list.pop() {
                return Ok(Some((it.latitude, it.longitude)));
            }
        }

        Ok(None)
    }
}

// ── Extração de ICAO da descrição ─────────────────────────────

/// Extrai o código ICAO do link Wunderground presente na descrição do evento.
///
/// Exemplos reais de descrições Polymarket:
///   "...available here: https://www.wunderground.com/history/daily/br/guarulhos/SBGR.To toggle..."
///   "...available here: https://www.wunderground.com/history/daily/us/fl/miami/KMIA."
///   "...available here: https://www.wunderground.com/history/daily/gb/london/EGLC."
pub fn extract_icao(description: &str) -> Option<String> {
    const MARKER: &str = "wunderground.com/history/daily/";
    extract_icao_and_source(description).map(|(icao, _)| icao)
}

/// Extrai a URL bruta do Wunderground usada na regra de resolução.
pub fn extract_wunderground_history_url(description: &str) -> Option<String> {
    const HTTPS_MARKER: &str = "https://www.wunderground.com/history/daily/";
    let pos = description.find(HTTPS_MARKER)?;
    let rest = &description[pos..];
    let url: String = rest
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'' && *c != ')')
        .collect();

    Some(url.trim_end_matches('.').to_string())
}

/// Extrai o ICAO E a fonte de resolução (aeroporto/cidade) do link Wunderground.
///
/// A URL do Wunderground tem a estrutura:
///   /history/daily/{país}/[{estado}/]{cidade}/{ICAO}
///
/// Retorna `(icao, fonte)` onde `fonte` é ex: `"Guarulhos, BR (SBGR)"`.
pub fn extract_icao_and_source(description: &str) -> Option<(String, String)> {
    const MARKER: &str = "wunderground.com/history/daily/";

    // Localiza o URL do Wunderground na descrição
    let segment = if let Some(pos) = description.find(MARKER) {
        let after = &description[pos + MARKER.len()..];
        // Captura o path completo até o próximo delimitador
        after
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'' && *c != ')' && *c != '.')
            .collect::<String>()
    } else {
        // Fallback: "here: https://.../<ICAO>"
        let words: Vec<&str> = description.split_whitespace().collect();
        let mut found = String::new();
        'outer: for (i, word) in words.iter().enumerate() {
            if *word == "here:" {
                if let Some(url) = words.get(i + 1) {
                    // Extrai o path após /history/daily/ se presente no fallback
                    if let Some(p) = url.find(MARKER) {
                        found = url[p + MARKER.len()..]
                            .chars()
                            .take_while(|c| {
                                !c.is_whitespace() && *c != '"' && *c != '.' && *c != ')'
                            })
                            .collect();
                        break 'outer;
                    }
                }
            }
        }
        found
    };

    if segment.is_empty() {
        // Tentativa alternativa: procura qualquer token ICAO entre colchetes/parênteses
        if let Some(cap) = regex::Regex::new(r"\[([A-Z]{3,4})\]")
            .ok()
            .and_then(|re| re.captures(description).and_then(|c| c.get(1).map(|m| m.as_str().to_string())))
        {
            let icao = cap;
            let city = "".to_string();
            let country = "".to_string();
            let source = format!("{} {}, [{}]", city, country, icao);
            return Some((icao, source));
        }

        // Busca por ICAO bruto (primeiro token de 3-4 letras maiúsculas)
        if let Some(cap) = regex::Regex::new(r"\b([A-Z]{3,4})\b").ok().and_then(|re| {
            re.captures(description)
                .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
        }) {
            let icao = cap;
            let city = "".to_string();
            let country = "".to_string();
            let source = format!("{} {}, [{}]", city, country, icao);
            return Some((icao, source));
        }

        return None;
    }

    // Path: {país}/[{estado}/]{cidade}/{ICAO}
    let parts: Vec<&str> = segment.split('/').collect();
    if parts.len() < 2 {
        return None;
    }

    let icao_raw = parts.last().unwrap_or(&"");
    let icao: String = icao_raw
        .chars()
        .take_while(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_uppercase();

    if !(3..=5).contains(&icao.len()) || !icao.chars().all(|c| c.is_alphanumeric()) {
        return None;
    }

    // País = primeiro segmento
    let country = parts[0].to_uppercase();

    // Cidade = segmento imediatamente antes do ICAO
    let city_raw = if parts.len() >= 2 {
        parts[parts.len() - 2]
    } else {
        parts[0]
    };
    // Capitaliza cada palavra separada por hífen
    let city: String = city_raw
        .split('-')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let source = format!("{}, {} [{}]", city, country, icao);
    Some((icao, source))
}

/// Detecta unidade de temperatura pela descrição do evento.
fn detect_unit(description: &str, markets: &[GammaMarket]) -> TempUnit {
    let lower = description.to_lowercase();
    if lower.contains("degrees celsius") {
        return TempUnit::Celsius;
    }
    if lower.contains("degrees fahrenheit") {
        return TempUnit::Fahrenheit;
    }
    // Fallback: analisa os títulos dos mercados
    for mkt in markets {
        if mkt.question.contains("°C") {
            return TempUnit::Celsius;
        }
        if mkt.question.contains("°F") {
            return TempUnit::Fahrenheit;
        }
    }
    TempUnit::Celsius // default
}

// ── Parser de range de temperatura ────────────────────────────

/// Extrai (min, max) do título de um mercado de temperatura.
///
/// Padrões suportados (todos observados nos mercados reais da Polymarket):
///   "15°C"               → (14.5, 15.5)   range exato ±0.5
///   "16°C"               → (15.5, 16.5)
///   "19°C or higher"     → (19.0, +∞)
///   "36°C or below"      → (-∞,  36.0)
///   "between 8°C and 10°C" → (8.0, 10.0)
///   "60-61°F"            → (60.0, 61.0)   range com hífen
///   "63°F or below"      → (-∞,  63.0)
///   "above 50°F"         → (50.0, +∞)
///   "below 5°C"          → (-∞,   5.0)
pub fn parse_temp_range(q: &str) -> (Option<f64>, Option<f64>) {
    let lower = q.to_lowercase();

    // "between X and Y"  ou  "between X-Y"
    if let Some(pos) = lower.find("between ") {
        let after = &q[pos + 8..];
        if let Some(a) = first_number(after) {
            if let Some(and_pos) = after.to_lowercase().find(" and ") {
                if let Some(b) = first_number(&after[and_pos + 5..]) {
                    return (Some(a), Some(b));
                }
            }
            // Fallback: "between 84-85°F" usa hífen em vez de "and"
            if let Some(range) = parse_hyphen_range(after) {
                return range;
            }
        }
    }

    // "X or higher" / "X or above"
    for kw in &["or higher", "or above"] {
        if let Some(pos) = lower.find(kw) {
            if let Some(n) = last_number_before(&q[..pos]) {
                return (Some(n), None);
            }
        }
    }

    // "X or lower" / "X or below"
    for kw in &["or lower", "or below"] {
        if let Some(pos) = lower.find(kw) {
            if let Some(n) = last_number_before(&q[..pos]) {
                return (None, Some(n));
            }
        }
    }

    // "above X" / "at least X"
    for kw in &["above ", "at least "] {
        if let Some(pos) = lower.find(kw) {
            if let Some(n) = first_number(&q[pos + kw.len()..]) {
                return (Some(n), None);
            }
        }
    }

    // "below X" / "at most X"
    for kw in &["below ", "at most "] {
        if let Some(pos) = lower.find(kw) {
            if let Some(n) = first_number(&q[pos + kw.len()..]) {
                return (None, Some(n));
            }
        }
    }

    // "60-61°F" — range com hífen
    if let Some(range) = parse_hyphen_range(q) {
        return range;
    }

    // Número isolado: "15°C" → exato ±0.5
    if let Some(n) = first_number(q) {
        return (Some(n - 0.5), Some(n + 0.5));
    }

    (None, None)
}

fn parse_hyphen_range(q: &str) -> Option<(Option<f64>, Option<f64>)> {
    let chars: Vec<char> = q.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let mut n1 = String::new();
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                n1.push(chars[i]);
                i += 1;
            }
            if i < chars.len() && chars[i] == '-' {
                i += 1;
                let mut n2 = String::new();
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    n2.push(chars[i]);
                    i += 1;
                }
                if let (Ok(a), Ok(b)) = (n1.parse::<f64>(), n2.parse::<f64>()) {
                    return Some((Some(a), Some(b)));
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

fn first_number(s: &str) -> Option<f64> {
    let tok: String = s
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    tok.parse().ok()
}

fn last_number_before(s: &str) -> Option<f64> {
    let mut last: Option<f64> = None;
    let mut cur = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() || ch == '.' || (ch == '-' && cur.is_empty()) {
            cur.push(ch);
        } else {
            if let Ok(n) = cur.parse::<f64>() {
                last = Some(n);
            }
            cur.clear();
        }
    }
    if let Ok(n) = cur.parse::<f64>() {
        last = Some(n);
    }
    last
}

// ── Testes ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icao_sbgr() {
        let d = "available here: https://www.wunderground.com/history/daily/br/guarulhos/SBGR.To toggle";
        assert_eq!(extract_icao(d), Some("SBGR".into()));
    }
    #[test]
    fn icao_kmia() {
        let d = "here: https://www.wunderground.com/history/daily/us/fl/miami/KMIA.";
        assert_eq!(extract_icao(d), Some("KMIA".into()));
    }
    #[test]
    fn icao_eglc() {
        let d = "here: https://www.wunderground.com/history/daily/gb/london/EGLC.";
        assert_eq!(extract_icao(d), Some("EGLC".into()));
    }
    #[test]
    fn range_exact() {
        assert_eq!(parse_temp_range("15°C"), (Some(14.5), Some(15.5)));
    }
    #[test]
    fn range_hyphen() {
        assert_eq!(parse_temp_range("60-61°F"), (Some(60.0), Some(61.0)));
    }
    #[test]
    fn range_higher() {
        assert_eq!(parse_temp_range("19°C or higher"), (Some(19.0), None));
    }
    #[test]
    fn range_below() {
        assert_eq!(parse_temp_range("36°C or below"), (None, Some(36.0)));
    }
    #[test]
    fn range_between() {
        assert_eq!(
            parse_temp_range("between 8°C and 10°C"),
            (Some(8.0), Some(10.0))
        );
    }
    #[test]
    fn range_f_below() {
        assert_eq!(parse_temp_range("63°F or below"), (None, Some(63.0)));
    }
    #[test]
    fn range_between_hyphen() {
        assert_eq!(
            parse_temp_range("between 84-85°F"),
            (Some(84.0), Some(85.0))
        );
    }
}

// ── Tabela estática ICAO → coordenadas ────────────────────────

/// Retorna (latitude, longitude) para um código ICAO de aeroporto.
///
/// Fonte: OurAirports / OpenAIP.
/// Usar SEMPRE esta tabela em vez do Open-Meteo Geocoding com código ICAO,
/// pois a API de geocoding busca por nome de cidade — não por código de aeroporto.
pub fn icao_static_coords(icao: &str) -> Option<(f64, f64)> {
    match icao {
        // ── América do Norte ──────────────────────────────────
        "KLAX" => Some((33.9425, -118.4081)), // Los Angeles Intl
        "KSFO" => Some((37.6213, -122.3790)), // San Francisco Intl
        "KLAS" => Some((36.0840, -115.1537)), // Las Vegas
        "KPHX" => Some((33.4373, -112.0078)), // Phoenix Sky Harbor
        "KDEN" => Some((39.8561, -104.6737)), // Denver Intl
        "KDFW" | "KDAL" => Some((32.8998, -97.0403)), // Dallas Fort Worth
        "KIAH" | "KHOU" => Some((29.9844, -95.3414)), // Houston Intercontinental
        "KMCO" => Some((28.4312, -81.3081)),  // Orlando Intl
        "KMIA" => Some((25.7959, -80.2870)),  // Miami Intl
        "KATL" => Some((33.6407, -84.4277)),  // Atlanta Hartsfield
        "KORD" => Some((41.9742, -87.9073)),  // Chicago O'Hare
        "KMDW" => Some((41.7868, -87.7522)),  // Chicago Midway
        "KDTW" => Some((42.2162, -83.3554)),  // Detroit Metro
        "KEWR" => Some((40.6925, -74.1687)),  // Newark Liberty
        "KJFK" => Some((40.6413, -73.7781)),  // New York JFK
        "KLGA" => Some((40.7772, -73.8726)),  // New York LaGuardia
        "KBOS" => Some((42.3656, -71.0096)),  // Boston Logan
        "KPHL" => Some((39.8719, -75.2411)),  // Philadelphia
        "KIAD" => Some((38.9531, -77.4565)),  // Washington Dulles
        "KDCA" => Some((38.8521, -77.0377)),  // Washington Reagan
        "KSEA" => Some((47.4502, -122.3088)), // Seattle-Tacoma
        "KPDX" => Some((45.5887, -122.5975)), // Portland
        "KSLC" => Some((40.7884, -111.9778)), // Salt Lake City
        "KMSP" => Some((44.8848, -93.2223)),  // Minneapolis
        "KSTL" => Some((38.7487, -90.3700)),  // St. Louis
        "KCLE" => Some((41.4117, -81.8498)),  // Cleveland
        "KCVG" => Some((39.0488, -84.6678)),  // Cincinnati
        "KPIT" => Some((40.4915, -80.2329)),  // Pittsburgh
        "KRDU" => Some((35.8776, -78.7875)),  // Raleigh-Durham
        "KCLT" => Some((35.2140, -80.9431)),  // Charlotte
        "KMEM" => Some((35.0424, -89.9767)),  // Memphis
        "KMSY" => Some((29.9934, -90.2580)),  // New Orleans
        "KBNA" => Some((36.1245, -86.6782)),  // Nashville
        "KABQ" => Some((35.0402, -106.6090)), // Albuquerque
        "KTUS" => Some((32.1161, -110.9410)), // Tucson
        "KBWI" => Some((39.1754, -76.6683)),  // Baltimore
        "PHNL" => Some((21.3187, -157.9219)), // Honolulu
        // ── Canadá ──────────────────────────────────────────
        "CYYZ" => Some((43.6772, -79.6306)),  // Toronto Pearson
        "CYVR" => Some((49.1939, -123.1844)), // Vancouver
        "CYUL" | "CYMX" => Some((45.4706, -73.7408)), // Montreal
        "CYYC" => Some((51.1139, -114.0200)), // Calgary
        "CYEG" => Some((53.3097, -113.5797)), // Edmonton
        "CYOW" => Some((45.3225, -75.6692)),  // Ottawa
        // ── América do Sul ──────────────────────────────────
        "SAEZ" => Some((-34.8222, -58.5358)), // Buenos Aires Ezeiza
        "SABE" => Some((-34.5592, -58.4156)), // Buenos Aires Jorge Newbery
        "SBGR" => Some((-23.4356, -46.4731)), // São Paulo Guarulhos
        "SBSP" => Some((-23.6261, -46.6553)), // São Paulo Congonhas
        "SBGL" => Some((-22.8100, -43.2506)), // Rio de Janeiro Galeão
        "SBRJ" => Some((-22.9100, -43.1631)), // Rio Santos Dumont
        "SBCF" => Some((-19.6244, -43.9719)), // Belo Horizonte Confins
        "SBCT" => Some((-25.5285, -49.1758)), // Curitiba
        "SBPA" => Some((-29.9944, -51.1714)), // Porto Alegre
        "SBSV" => Some((-12.9086, -38.3225)), // Salvador
        "SBRF" => Some((-8.1260, -34.9228)),  // Recife
        "SBFZ" => Some((-3.7763, -38.5322)),  // Fortaleza
        "SBMN" => Some((-3.1604, -60.0197)),  // Manaus
        "SBKP" => Some((-23.0074, -47.1345)), // Campinas
        "SCEL" => Some((-33.3928, -70.7853)), // Santiago
        "SEQM" => Some((-0.1292, -78.3575)),  // Quito
        "SPJC" => Some((-12.0219, -77.1143)), // Lima
        "MMMX" => Some((19.4363, -99.0721)),  // Cidade do México
        "MMGL" => Some((20.5218, -103.3110)), // Guadalajara
        "MMUN" => Some((21.0365, -86.8771)),  // Cancún
        // ── Europa ──────────────────────────────────────────
        "EGLL" => Some((51.4775, -0.4614)), // Londres Heathrow
        "EGLC" => Some((51.5048, 0.0495)),  // Londres City
        "EGKK" => Some((51.1481, -0.1903)), // Londres Gatwick
        "EGCC" => Some((53.3537, -2.2750)), // Manchester
        "EGBB" => Some((52.4539, -1.7480)), // Birmingham
        "EGPH" => Some((55.9500, -3.3725)), // Edimburgo
        "LFPG" => Some((49.0097, 2.5479)),  // Paris CDG
        "LFPO" => Some((48.7233, 2.3795)),  // Paris Orly
        "LFMN" => Some((43.6584, 7.2159)),  // Nice
        "LFLL" => Some((45.7256, 5.0881)),  // Lyon
        "EDDB" => Some((52.3667, 13.5033)), // Berlim Brandenburg
        "EDDM" => Some((48.3538, 11.7861)), // Munique
        "EDDF" => Some((50.0379, 8.5622)),  // Frankfurt
        "EDDH" => Some((53.6303, 9.9882)),  // Hamburgo
        "EHAM" => Some((52.3105, 4.7683)),  // Amsterdam Schiphol
        "EIDW" => Some((53.4213, -6.2700)), // Dublin
        "EINN" => Some((52.7019, -8.9248)), // Shannon
        "LEMD" => Some((40.4719, -3.5626)), // Madri Barajas
        "LEBL" => Some((41.2971, 2.0785)),  // Barcelona
        "LEPA" => Some((39.5517, 2.7388)),  // Palma de Mallorca
        "LIRF" => Some((41.8003, 12.2389)), // Roma Fiumicino
        "LIML" => Some((45.4654, 9.2760)),  // Milão Linate
        "LSZH" => Some((47.4584, 8.5480)),  // Zurique
        "LSGG" => Some((46.2381, 6.1090)),  // Genebra
        "LOWW" => Some((48.1103, 16.5697)), // Viena
        "EPWA" => Some((52.1657, 20.9671)), // Varsóvia
        "LKPR" => Some((50.1008, 14.2600)), // Praga
        "LHBP" => Some((47.4298, 19.2611)), // Budapeste
        "UUWW" => Some((55.5915, 37.2615)), // Moscou Vnukovo
        "UUEE" => Some((55.9726, 37.4146)), // Moscou Sheremetyevo
        "UKBB" => Some((50.3450, 30.8947)), // Kyiv Boryspil
        "LGAV" => Some((37.9364, 23.9445)), // Atenas
        "LTAC" => Some((40.1281, 32.9951)), // Ancara Esenboğa
        "LTBA" => Some((40.9769, 28.8146)), // Istambul Atatürk
        "LTFM" => Some((41.2753, 28.7519)), // Istambul Novo
        "LPPT" => Some((38.7756, -9.1359)), // Lisboa
        "LPPR" => Some((41.2481, -8.6814)), // Porto
        "EKCH" => Some((55.6180, 12.6560)), // Copenhague
        "ESSA" => Some((59.6519, 17.9186)), // Estocolmo Arlanda
        "ENGM" => Some((60.1939, 11.1004)), // Oslo Gardermoen
        "EFHK" => Some((60.3172, 24.9633)), // Helsinki
        // ── Oriente Médio / África ───────────────────────────
        "LLBG" => Some((31.9929, 34.8870)), // Tel Aviv Ben Gurion
        "OBBE" => Some((26.2708, 50.6336)), // Bahrein
        "OMDB" => Some((25.2528, 55.3644)), // Dubai
        "OMAA" => Some((24.4330, 54.6511)), // Abu Dhabi
        "OERK" => Some((24.9575, 46.6988)), // Riade
        "OTHH" => Some((25.2731, 51.6081)), // Doha
        "OKBK" => Some((29.2266, 47.9689)), // Kuwait
        "OJAM" => Some((31.7226, 35.9932)), // Amã
        "HECA" => Some((30.1219, 31.4056)), // Cairo
        "HAAB" => Some((8.9779, 38.7993)),  // Adis Abeba
        "DNMM" => Some((6.5774, 3.3212)),   // Lagos
        "FACT" => Some((-33.9648, 18.6017)), // Cidade do Cabo
        "FAJS" => Some((-26.1392, 28.2460)), // Joanesburgo O.R. Tambo
        "DTTA" => Some((36.8510, 10.2272)), // Tunis
        "GMMN" => Some((33.3675, -7.5898)), // Casablanca
        // ── Ásia / Pacífico ──────────────────────────────────
        "RJTT" => Some((35.5494, 139.7798)),  // Tóquio Haneda
        "RJAA" => Some((35.7714, 140.3926)),  // Tóquio Narita
        "VHHH" => Some((22.3080, 113.9185)),  // Hong Kong
        "RKSS" => Some((37.5583, 126.7908)),  // Seul Gimpo
        "RKSI" => Some((37.4602, 126.4407)),  // Seul Incheon
        "RCTP" => Some((25.0777, 121.2330)),  // Taipei Taoyuan
        "ZSSS" => Some((31.1979, 121.3363)),  // Xangai Hongqiao
        "ZSPD" => Some((31.1434, 121.8052)),  // Xangai Pudong
        "ZBAA" => Some((40.0799, 116.5846)),  // Pequim Capital
        "ZGGG" => Some((23.3924, 113.2990)),  // Guangzhou
        "VABB" => Some((19.0896, 72.8656)),   // Mumbai
        "VIDP" => Some((28.5665, 77.1031)),   // Nova Delhi IGI
        "VILK" => Some((26.7606, 80.8893)),   // Lucknow Amausi
        "VOCI" => Some((10.1520, 76.4019)),   // Kochi
        "VOMM" => Some((13.0677, 80.1699)),   // Chennai
        "VOHY" => Some((17.2313, 78.4298)),   // Hyderabad
        "VECC" => Some((22.6547, 88.4467)),   // Calcutá
        "VTBS" => Some((13.9132, 100.6070)),  // Bangkok Suvarnabhumi
        "WMKK" => Some((2.7456, 101.7072)),   // Kuala Lumpur
        "WSSS" => Some((1.3644, 103.9915)),   // Singapura Changi
        "WIII" => Some((-6.1256, 106.6558)),  // Jacarta
        "RPLL" => Some((14.5086, 121.0194)),  // Manila
        "VNKT" => Some((27.6966, 85.3591)),   // Katmandu
        "YSSY" => Some((-33.9461, 151.1772)), // Sydney
        "YMML" => Some((-37.6733, 144.8433)), // Melbourne
        "YBBN" => Some((-27.3842, 153.1175)), // Brisbane
        "NZWN" => Some((-41.3272, 174.8051)), // Wellington
        "NZAA" => Some((-37.0082, 174.7850)), // Auckland
        _ => None,
    }
}

/// Correção de viés por estação ICAO (em °C).
///
/// O Open-Meteo interpolado de grade de modelo tende a subestimar a temperatura
/// máxima registrada pela estação ASOS/METAR de aeroportos urbanos.
/// Razões: ilha de calor do asfalto, microclima local vs média de grade de ~25 km².
///
/// Valores calibrados empiricamente. Aplicar ANTES de comparar com o range do mercado.
pub fn bias_correction_celsius(icao: &str) -> f64 {
    match icao {
        // EUA — aeroportos com forte ilha de calor
        "KMIA" => 1.2,                   // Miami: subestimação consistente de 1–3°F
        "KATL" => 1.1,                   // Atlanta: grande área pavimentada
        "KDFW" | "KDAL" => 1.5,          // Dallas: calor extremo + asfalto
        "KORD" | "KMDW" => 0.9,          // Chicago: efeito lago atenua o bias
        "KLAX" => 0.5,                   // Los Angeles: influência marítima
        "KSEA" => 0.4,                   // Seattle: temperado marítimo
        "KMCO" | "KTPA" => 1.2,          // Flórida: subtropical
        "KJFK" | "KEWR" | "KLGA" => 0.8, // Nova York
        "KBOS" => 0.7,                   // Boston
        "KDEN" => 0.9,                   // Denver: altitude modera o bias
        "KIAH" | "KHOU" => 1.3,          // Houston: subtropical + umidade
        "KSFO" => 0.4,                   // San Francisco: neblina e frescor
        // Canadá
        "CYYZ" => 0.7, // Toronto
        // Brasil
        "SBGR" => 1.0, // São Paulo Guarulhos
        "SBSP" => 1.2, // São Paulo Congonhas: área urbana densa
        "SBGL" => 1.1, // Rio de Janeiro Galeão
        // Europa
        "EGLL" | "EGLC" => 0.6, // Londres: temperado oceânico
        "LFPG" | "LFPO" => 0.7, // Paris
        "EDDM" => 0.7,          // Munique
        "EDDF" => 0.7,          // Frankfurt
        "LEMD" => 1.0,          // Madri: continental seco
        "LIRF" => 0.8,          // Roma
        // Oriente Médio
        "OMDB" | "OMAA" => 1.5, // Dubai / Abu Dhabi: deserto + asfalto
        "OERK" => 1.5,          // Riade
        "OTHH" => 1.3,          // Doha
        "LLBG" => 0.8,          // Tel Aviv: mediterrâneo
        // Ásia do Sul (bias alto — calor continental extremo)
        "VILK" => 1.5, // Lucknow: clima monção continental
        "VIDP" => 1.4, // Nova Delhi: ilha de calor severa
        "VABB" => 1.0, // Mumbai: umidade costeira reduz bias
        "VOMM" => 1.2, // Chennai: tropical
        "VOHY" => 1.2, // Hyderabad
        // Leste Asiático
        "VHHH" => 0.8,          // Hong Kong: costeiro
        "RJTT" | "RJAA" => 0.7, // Tóquio
        "RKSS" | "RKSI" => 0.8, // Seul
        // Sudeste Asiático / Oceania
        "WSSS" => 0.6, // Singapura: equatorial com nuvens
        "VTBS" => 1.2, // Bangkok: tropical
        "YSSY" => 0.6, // Sydney: costeiro
        "NZWN" => 0.4, // Wellington: vento oceânico
        // Default: bias moderado para aeroportos não catalogados
        _ => 0.8,
    }
}

/// Extrai a data-alvo de um event slug da Polymarket.
///
/// Formato esperado: `"...highest-temperature-in-london-on-march-11-2026"`.
pub fn extract_target_date_from_slug(slug: &str) -> Option<NaiveDate> {
    let lower = slug.to_lowercase();
    let pos = lower.rfind("-on-")?;
    let parts: Vec<&str> = lower[pos + 4..].splitn(3, '-').collect();
    if parts.len() < 3 {
        return None;
    }
    let month: u32 = match parts[0] {
        "january" | "jan" => 1,
        "february" | "feb" => 2,
        "march" | "mar" => 3,
        "april" | "apr" => 4,
        "may" => 5,
        "june" | "jun" => 6,
        "july" | "jul" => 7,
        "august" | "aug" => 8,
        "september" | "sep" => 9,
        "october" | "oct" => 10,
        "november" | "nov" => 11,
        "december" | "dec" => 12,
        _ => return None,
    };
    NaiveDate::from_ymd_opt(parts[2].parse().ok()?, month, parts[1].parse().ok()?)
}
