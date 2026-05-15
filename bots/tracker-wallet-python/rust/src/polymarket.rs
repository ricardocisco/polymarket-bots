use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

const DATA_API_URL: &str = "https://data-api.polymarket.com";

// ─── Structs da API ──────────────────────────────────────────────────────────

/// Resposta bruta de GET /activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEvent {
    #[serde(rename = "proxyWallet")]
    pub proxy_wallet: Option<String>,
    pub timestamp: i64,
    #[serde(rename = "conditionId")]
    pub condition_id: Option<String>,
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    /// Quantidade de shares
    pub size: Option<f64>,
    /// Valor em USDC
    #[serde(rename = "usdcSize")]
    pub usdc_size: Option<f64>,
    #[serde(rename = "transactionHash")]
    pub transaction_hash: Option<String>,
    pub price: Option<f64>,
    /// clobTokenId
    pub asset: Option<String>,
    /// "BUY" | "SELL"
    pub side: Option<String>,
    #[serde(rename = "outcomeIndex")]
    pub outcome_index: Option<i64>,
    pub title: Option<String>,
    pub slug: Option<String>,
    /// URL da imagem do mercado
    pub icon: Option<String>,
    #[serde(rename = "eventSlug")]
    pub event_slug: Option<String>,
    pub outcome: Option<String>,
    /// Nome de exibição do trader
    pub name: Option<String>,
    pub pseudonym: Option<String>,
}

// ─── Struct processada (para o tracker) ──────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)] // condition_id e slugs serão usados quando enriquecimento for habilitado
pub struct PolyActivity {
    /// Identificador único: transactionHash ou fallback
    pub id: String,
    /// Unix timestamp em segundos
    pub timestamp: i64,
    pub market_title: String,
    pub outcome: String,
    /// "BUY" | "SELL"
    pub side: String,
    pub price: f64,
    pub shares: f64,
    /// Valor total em USD
    pub usdc_size: f64,
    pub event_slug: String,
    pub market_slug: String,
    pub condition_id: String,
    /// URL da imagem (campo `icon` da API — sem scraping)
    pub market_image_url: String,
    /// URL construída estaticamente a partir de event_slug + slug
    pub market_url: String,
    /// Nome do trader vindo direto da resposta da API (name ou pseudonym)
    pub display_name: Option<String>,
}

// ─── Funções públicas ─────────────────────────────────────────────────────────

/// Resolve endereço 0x a partir de input:
/// - Se começar com 0x: retorna direto
/// - Se for username: scraping da página polymarket.com/@slug (igual ao TypeScript)
pub async fn resolve_user(client: &reqwest::Client, input: &str) -> Option<String> {
    let clean = input.trim();

    // Endereço 0x direto
    if clean.starts_with("0x") && clean.len() == 42 {
        return Some(clean.to_lowercase());
    }

    // Limpa username
    let slug = clean
        .trim_start_matches("https://polymarket.com/@")
        .trim_start_matches("https://polymarket.com/profile/")
        .trim_start_matches('@')
        .split('?')
        .next()
        .unwrap_or(clean);

    // Scraping da página de perfil — mesmo método que funcionava no TypeScript
    let profile_url = format!("https://polymarket.com/@{slug}");
    let resp = match client
        .get(&profile_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .header("Accept", "text/html,application/xhtml+xml")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!("⚠️ Erro de rede ao resolver @{slug}: {e}");
            return None;
        }
    };

    if !resp.status().is_success() {
        warn!("⚠️ HTTP {} ao resolver @{slug}", resp.status());
        return None;
    }

    let html = match resp.text().await {
        Ok(h) => h,
        Err(e) => {
            warn!("⚠️ Erro ao ler HTML de @{slug}: {e}");
            return None;
        }
    };

    // Extrai proxyWallet do JSON embebido na página (igual ao TypeScript)
    if let Some(addr) = extract_address_from_html(&html) {
        debug!("✅ Resolvido @{slug} → {addr}");
        return Some(addr);
    }

    warn!("⚠️ Não encontrei endereço para @{slug}");
    None
}

/// Extrai endereço 0x do HTML da página de perfil da Polymarket
fn extract_address_from_html(html: &str) -> Option<String> {
    // Padrões em ordem de confiabilidade (igual ao TypeScript)
    let prefixes = ["\"proxyWallet\":\"", "\"address\":\""];
    for prefix in &prefixes {
        if let Some(pos) = html.find(prefix) {
            let start = pos + prefix.len();
            if start + 42 <= html.len() {
                let candidate = &html[start..start + 42];
                if candidate.starts_with("0x")
                    && candidate[2..].chars().all(|c| c.is_ascii_hexdigit())
                {
                    return Some(candidate.to_lowercase());
                }
            }
        }
    }
    None
}

/// Obtém nome de exibição para uma carteira via scraping da página de perfil
pub async fn get_username_from_address(client: &reqwest::Client, address: &str) -> Option<String> {
    let url = format!("https://polymarket.com/profile/{address}");
    let resp = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .header("Accept", "text/html,application/xhtml+xml")
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let html = resp.text().await.ok()?;

    // Extrai "pseudonym" ou "name" do JSON embebido na página
    for prefix in &["\"pseudonym\":\"", "\"name\":\""] {
        if let Some(pos) = html.find(prefix) {
            let start = pos + prefix.len();
            if let Some(end) = html[start..].find('"') {
                let name = &html[start..start + end];
                if !name.is_empty() && name != "null" {
                    return Some(name.to_string());
                }
            }
        }
    }

    None
}

/// Busca atividades recentes via GET /activity (substitui o scraping por snapshot/diff)
pub async fn fetch_user_activity(
    client: &reqwest::Client,
    address: &str,
    since_timestamp: i64,
) -> Vec<PolyActivity> {
    // Normaliza para segundos: wallets migradas do TypeScript têm milissegundos
    let since_secs = if since_timestamp > 1_000_000_000_000 {
        since_timestamp / 1000
    } else if since_timestamp <= 0 {
        // Fallback de segurança: evita trazer todo o histórico
        chrono::Utc::now().timestamp() - 60
    } else {
        since_timestamp
    };

    let result = client
        .get(format!("{DATA_API_URL}/activity"))
        .query(&[
            ("user", address.to_string()),
            ("start", since_secs.to_string()),
            ("type", "TRADE".to_string()),
            ("limit", "100".to_string()),
            ("sortBy", "TIMESTAMP".to_string()),
            ("sortDirection", "ASC".to_string()),
        ])
        .header("Accept", "application/json")
        .send()
        .await;

    match result {
        Ok(resp) if resp.status().is_success() => match resp.json::<Vec<ActivityEvent>>().await {
            Ok(events) => events
                .into_iter()
                .filter(|e| e.timestamp > since_secs)
                .filter_map(|e| {
                    let side = e.side.clone().unwrap_or_default().to_uppercase();
                    if side != "BUY" && side != "SELL" {
                        return None;
                    }

                    let condition_id = e.condition_id.clone().unwrap_or_default();
                    let id = e
                        .transaction_hash
                        .clone()
                        .unwrap_or_else(|| format!("{}-{}", condition_id, e.timestamp));

                    let event_slug = e.event_slug.clone().unwrap_or_default();
                    let market_slug = e.slug.clone().unwrap_or_default();
                    let market_url = build_market_url(&event_slug, &market_slug);

                    Some(PolyActivity {
                        id,
                        timestamp: e.timestamp,
                        market_title: e.title.unwrap_or_default(),
                        outcome: e.outcome.unwrap_or_default(),
                        side,
                        price: e.price.unwrap_or(0.0),
                        shares: e.size.unwrap_or(0.0),
                        usdc_size: e.usdc_size.unwrap_or(0.0),
                        event_slug,
                        market_slug,
                        condition_id,
                        market_image_url: e.icon.unwrap_or_default(),
                        market_url,
                        // Nome do trader vindo direto da API — mais confiável que scraping
                        display_name: e
                            .pseudonym
                            .or(e.name)
                            .filter(|n| !n.is_empty() && n != "null"),
                    })
                })
                .collect(),
            Err(e) => {
                warn!("⚠️ Erro ao parsear activity de {}: {e}", &address[..8]);
                vec![]
            }
        },
        Ok(resp) => {
            warn!(
                "⚠️ HTTP {} ao buscar activity de {}",
                resp.status(),
                &address[..8]
            );
            vec![]
        }
        Err(e) => {
            warn!("⚠️ Erro de rede ao buscar activity: {e}");
            vec![]
        }
    }
}

/// Constrói URL estática do mercado a partir dos slugs (sem scraping HTML)
pub fn build_market_url(event_slug: &str, market_slug: &str) -> String {
    fn is_valid_slug(s: &str) -> bool {
        !s.is_empty()
            && !s.starts_with("0x")
            && s.len() > 4
            && !s.chars().all(|c| c.is_ascii_hexdigit())
    }

    let has_event = is_valid_slug(event_slug);
    let has_market = is_valid_slug(market_slug);

    if has_event && has_market {
        format!("https://polymarket.com/event/{event_slug}/{market_slug}")
    } else if has_event {
        format!("https://polymarket.com/event/{event_slug}")
    } else if has_market {
        format!("https://polymarket.com/event/{market_slug}")
    } else {
        String::new()
    }
}

// ─── PNL ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ClosedPosition {
    #[serde(rename = "realizedPnl")]
    realized_pnl: Option<f64>,
}

/// Busca realizedPnl de uma posição fechada via GET /closed-positions
pub async fn fetch_closed_position(
    client: &reqwest::Client,
    address: &str,
    condition_id: &str,
) -> Option<f64> {
    let resp = client
        .get(format!("{DATA_API_URL}/closed-positions"))
        .query(&[("user", address), ("market", condition_id)])
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let positions = resp.json::<Vec<ClosedPosition>>().await.ok()?;
    positions.into_iter().next()?.realized_pnl
}
