import axios from "axios";
import { logger } from "./logger.js";

const DATA_API_URL = "https://data-api.polymarket.com";
const GAMMA_API_URL = "https://gamma-api.polymarket.com";

const BROWSER_HEADERS = {
  "User-Agent":
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
  Origin: "https://polymarket.com",
  Referer: "https://polymarket.com/",
  Accept: "application/json",
  "Accept-Language": "en-US,en;q=0.9",
  "Cache-Control": "no-cache"
};

export interface PolyActivity {
  id: string;
  timestamp: number; // Unix em segundos (da /activity)
  type: string;
  marketTitle: string;
  outcome: string;
  side: string;
  price: number;
  amount: number; // shares
  usdcSize: number; // valor em USD
  eventSlug: string;
  marketSlug: string;
  conditionId?: string;
  assetId?: string;
  marketImageUrl?: string;
  marketUrl?: string;
  displayName?: string; // nome do trader, da resposta da /activity
  realizedPnl?: number; // P&L realizado (vendas), via /closed-positions
}

// Cache de usernames (usado no /list)
const usernameCache = new Map<string, { username: string; timestamp: number }>();
const USERNAME_CACHE_TTL = 3600000; // 1 hora

// Helper para validar slug (não hexadecimal, não vazio)
function isValidSlug(slug: string | undefined): boolean {
  return !!slug && slug.length > 0 && !slug.startsWith("0x") && !/^[a-f0-9]{8,}$/i.test(slug);
}

// Constrói URL estática do mercado a partir dos slugs (sem scraping HTML)
function buildMarketUrl(eventSlug: string, marketSlug: string): string {
  const hasEvent = isValidSlug(eventSlug);
  const hasMarket = isValidSlug(marketSlug);
  if (hasEvent && hasMarket) return `https://polymarket.com/event/${eventSlug}/${marketSlug}`;
  if (hasEvent) return `https://polymarket.com/event/${eventSlug}`;
  if (hasMarket) return `https://polymarket.com/event/${marketSlug}`;
  return "";
}

// Busca realizedPnl via GET /closed-positions (para vendas)
async function fetchClosedPosition(address: string, conditionId: string): Promise<number | null> {
  try {
    const resp = await axios.get(`${DATA_API_URL}/closed-positions`, {
      params: { user: address, market: conditionId },
      headers: { Accept: "application/json" },
      timeout: 5000,
      validateStatus: (s) => s < 500
    });
    if (resp.status === 200 && Array.isArray(resp.data) && resp.data[0]) {
      const pnl = Number(resp.data[0].realizedPnl);
      return isNaN(pnl) ? null : pnl;
    }
  } catch {}
  return null;
}

/*
    const patterns = [
      // Link canônico (mais confiável)
      /<link[^>]*rel=["']canonical["'][^>]*href=["']([^"']+)["']/i,
      // JSON-LD structured data
      /"url":\s*"([^"]*\/event\/[^"]+\?tid=\d+)["']/i,
      // Links com event-group (formato: /event/group/market?tid=)
      /href=["']([^"']*\/event\/[^\/]+\/[^"']+\?tid=\d+)["']/i,
      // Links gerais com tid
      /href=["']([^"']*\/event\/[^"']+\?tid=\d+)["']/i,
      // JavaScript redirects
      /window\.location\.href\s*=\s*["']([^"']*\/event\/[^"']+)["']/i,
      // React Router ou Next.js links
      /"pathname":\s*"([^"]*\/event\/[^"]+)"/i
    ];

    for (const pattern of patterns) {
      const match = html.match(pattern);
      if (match && match[1]) {
        let foundUrl = match[1];
        // Remove caracteres de escape se houver
        foundUrl = foundUrl.replace(/\\\//g, "/").replace(/\\"/g, "");

        // Garante que é uma URL completa
        if (foundUrl.startsWith("/")) {
          foundUrl = `https://polymarket.com${foundUrl}`;
        } else if (!foundUrl.startsWith("http")) {
          foundUrl = `https://polymarket.com/${foundUrl}`;
        }

        // Verifica se tem o formato correto e prioriza links com event-group
        if (foundUrl.includes("/event/") && foundUrl.includes("polymarket.com")) {
          // Prioriza links com formato /event/group/market?tid=
          if (foundUrl.match(/\/event\/[^\/]+\/[^\/?]+\?tid=/)) {
            console.log(`   🔗 Link completo encontrado (com event-group): ${foundUrl}`);
            return foundUrl;
          }
        }
      }
    }

    // Se não encontrou link com event-group, tenta qualquer link com tid
    const marketLinkPattern = /href=["']([^"']*\/event\/[^"']+\?tid=\d+)["']/gi;
    const matches = [...html.matchAll(marketLinkPattern)];
    for (const match of matches) {
      let foundUrl = match[1];
      if (foundUrl.startsWith("/")) {
        foundUrl = `https://polymarket.com${foundUrl}`;
      } else if (!foundUrl.startsWith("http")) {
        foundUrl = `https://polymarket.com/${foundUrl}`;
      }
      if (foundUrl.includes("/event/") && foundUrl.includes("polymarket.com")) {
        console.log(`   🔗 Link encontrado (com tid): ${foundUrl}`);
        return foundUrl;
      }
    }

    // Última tentativa: procura na URL da resposta (se houver redirect)
    // Mas axios não expõe isso facilmente, então vamos tentar buscar no HTML
    // por qualquer referência ao marketSlug com event-group
    const eventGroupPattern = new RegExp(
      `(https?://[^"']*polymarket\\.com/event/[^/]+/${marketSlug.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}[^"']*)`,
      "i"
    );
    const eventGroupMatch = html.match(eventGroupPattern);
    if (eventGroupMatch && eventGroupMatch[1]) {
      console.log(`   🔗 Link encontrado via event-group pattern: ${eventGroupMatch[1]}`);
      return eventGroupMatch[1];
    }

    // Se não encontrou nos padrões, verifica se houve redirect
    // A URL final após redirect pode ter o formato correto
    // Mas isso é difícil de capturar sem seguir redirects manualmente

    return null;
  } catch (error: any) {
    // Se der erro, retorna null (não é crítico)
    return null;
  }
}

// NOVA FUNÇÃO: Busca informações detalhadas do mercado
// Retorna eventSlug, marketSlug, title, imageUrl e marketUrl completo
async function getMarketInfo(
  conditionId: string,
  assetId?: string
): Promise<{
  eventSlug: string;
  marketSlug: string;
  title: string;
  imageUrl: string;
  marketUrl?: string;
} | null> {
  // Verifica cache
  const cached = marketInfoCache.get(conditionId);
  if (cached && Date.now() - cached.timestamp < MARKET_CACHE_TTL) {
    return {
      eventSlug: cached.eventSlug,
      marketSlug: cached.marketSlug,
      title: cached.title,
      imageUrl: cached.imageUrl,
      marketUrl: cached.marketUrl
    };
  }

  let foundTitle = "";
  let foundEventSlug = "";
  let foundMarketSlug = "";
  let foundImageUrl = "";
  let foundMarketUrl: string | undefined = undefined;

  // Tenta múltiplas estratégias
  try {
    // ESTRATÉGIA 1: Busca direta via conditionId no DATA API
    try {
      const directResponse = await axios.get(`${DATA_API_URL}/markets/${conditionId}`, {
        headers: BROWSER_HEADERS,
        timeout: 5000,
        validateStatus: (status) => status < 500
      });

      if (directResponse.status === 200 && directResponse.data) {
        const d = directResponse.data;
        const title = d.question || d.title || "";
        // marketSlug é o slug do sub-mercado (limpa possíveis IDs numéricos)
        let marketSlug = d.slug || d.market_slug || "";
        // eventSlug é o slug do evento pai - tenta vários campos possíveis
        const eventSlug =
          d.groupItemSlug ||
          d.eventSlug ||
          d.event_slug ||
          d.parentSlug ||
          d.groupSlug ||
          (d.event && d.event.slug) ||
          (d.group && d.group.slug) ||
          "";
        // Imagem do mercado
        const imageUrl = d.image || d.icon || d.imageUrl || "";

        if (title && !foundTitle) foundTitle = title;
        if (eventSlug && !foundEventSlug) foundEventSlug = eventSlug;
        if (marketSlug && !foundMarketSlug) foundMarketSlug = marketSlug;
        if (imageUrl && !foundImageUrl) foundImageUrl = imageUrl;

        // DEBUG: Log para ver o que a API retorna
        if (!eventSlug) {
          console.log(`   🔍 DATA API campos disponíveis: ${Object.keys(d).join(", ")}`);
        }

        if (foundTitle && (foundEventSlug || foundMarketSlug)) {
          marketInfoCache.set(conditionId, {
            eventSlug: foundEventSlug,
            marketSlug: foundMarketSlug,
            title: foundTitle,
            imageUrl: foundImageUrl,
            marketUrl: undefined,
            timestamp: Date.now()
          });
          return {
            eventSlug: foundEventSlug,
            marketSlug: foundMarketSlug,
            title: foundTitle,
            imageUrl: foundImageUrl,
            marketUrl: undefined
          };
        }
      }
    } catch (e) {
      // Continua para próxima estratégia
    }

    // ESTRATÉGIA 2: Busca via CLOB markets
    try {
      const clobResponse = await axios.get(`${CLOB_API_URL}/markets/${conditionId}`, {
        headers: BROWSER_HEADERS,
        timeout: 3000,
        validateStatus: (status) => status < 500
      });

      if (clobResponse.status === 200 && clobResponse.data) {
        const d = clobResponse.data;
        const title = d.question || d.description || "";
        const marketSlug = d.slug || d.market_slug || "";
        const eventSlug =
          d.groupItemSlug ||
          d.eventSlug ||
          d.event_slug ||
          d.parentSlug ||
          d.groupSlug ||
          (d.event && d.event.slug) ||
          (d.group && d.group.slug) ||
          "";
        const imageUrl = d.image || d.icon || d.imageUrl || "";

        if (title && !foundTitle) foundTitle = title;
        if (eventSlug && !foundEventSlug) foundEventSlug = eventSlug;
        if (marketSlug && !foundMarketSlug) foundMarketSlug = marketSlug;
        if (imageUrl && !foundImageUrl) foundImageUrl = imageUrl;

        if (foundTitle && (foundEventSlug || foundMarketSlug)) {
          marketInfoCache.set(conditionId, {
            eventSlug: foundEventSlug,
            marketSlug: foundMarketSlug,
            title: foundTitle,
            imageUrl: foundImageUrl,
            marketUrl: undefined,
            timestamp: Date.now()
          });
          return {
            eventSlug: foundEventSlug,
            marketSlug: foundMarketSlug,
            title: foundTitle,
            imageUrl: foundImageUrl,
            marketUrl: undefined
          };
        }
      }
    } catch (e) {
      // Continua
    }

    // ESTRATÉGIA 3: Busca via Gamma por condition_id (com busca de evento)
    try {
      const gammaResponse = await axios.get(`${GAMMA_API_URL}/markets`, {
        params: { condition_id: conditionId },
        headers: BROWSER_HEADERS,
        timeout: 5000,
        validateStatus: (status) => status < 500
      });

      if (gammaResponse.data && Array.isArray(gammaResponse.data) && gammaResponse.data[0]) {
        const market = gammaResponse.data[0];
        const title = market.question || market.title || "";
        const marketSlug = market.slug || market.market_slug || "";
        // Captura a imagem do mercado
        const imageUrl = market.image || market.icon || market.imageUrl || "";

        // Captura o ID do evento para buscar o eventSlug
        const eventId = market.event_id || market.eventId || market.group_id || "";

        // Tenta múltiplos campos para eventSlug
        let eventSlug =
          market.groupItemSlug ||
          market.eventSlug ||
          market.event_slug ||
          market.parentSlug ||
          market.groupSlug ||
          (market.events && market.events[0] && market.events[0].slug) ||
          (market.event && market.event.slug) ||
          "";

        // Se não encontrou eventSlug mas tem eventId, busca o evento
        if (!eventSlug && eventId) {
          try {
            const eventResponse = await axios.get(`${GAMMA_API_URL}/events/${eventId}`, {
              headers: BROWSER_HEADERS,
              timeout: 3000,
              validateStatus: (status) => status < 500
            });

            if (eventResponse.status === 200 && eventResponse.data) {
              eventSlug = eventResponse.data.slug || "";
              console.log(`   🎯 Evento encontrado via ID: ${eventSlug}`);
            }
          } catch (e) {
            // Tenta busca alternativa por slug
          }
        }

        // Se ainda não tem eventSlug, tenta buscar evento pelo slug do mercado
        if (!eventSlug && marketSlug) {
          try {
            const eventsResponse = await axios.get(`${GAMMA_API_URL}/events`, {
              params: { slug: marketSlug, limit: 1 },
              headers: BROWSER_HEADERS,
              timeout: 3000,
              validateStatus: (status) => status < 500
            });

            if (eventsResponse.data && Array.isArray(eventsResponse.data) && eventsResponse.data[0]) {
              eventSlug = eventsResponse.data[0].slug || "";
              console.log(`   🎯 Evento encontrado via busca: ${eventSlug}`);
            }
          } catch (e) {
            // Continua
          }
        }

        if (title && !foundTitle) foundTitle = title;
        if (eventSlug && !foundEventSlug) foundEventSlug = eventSlug;
        if (marketSlug && !foundMarketSlug) foundMarketSlug = marketSlug;
        if (imageUrl && !foundImageUrl) foundImageUrl = imageUrl;

        // DEBUG: Se não encontrou eventSlug, mostra campos disponíveis
        if (!eventSlug && !foundEventSlug) {
          console.log(`   🔍 Gamma market campos: ${Object.keys(market).join(", ")}`);
          if (eventId) console.log(`   📌 Event ID: ${eventId}`);
        }

        if (foundTitle && (foundEventSlug || foundMarketSlug)) {
          marketInfoCache.set(conditionId, {
            eventSlug: foundEventSlug,
            marketSlug: foundMarketSlug,
            title: foundTitle,
            imageUrl: foundImageUrl,
            marketUrl: undefined,
            timestamp: Date.now()
          });
          return {
            eventSlug: foundEventSlug,
            marketSlug: foundMarketSlug,
            title: foundTitle,
            imageUrl: foundImageUrl,
            marketUrl: undefined
          };
        }
      }
    } catch (e) {
      // Continua
    }

    // ESTRATÉGIA 4: Busca via Gamma usando clob_token_ids (assetId)
    if (assetId) {
      try {
        const gammaTokenResponse = await axios.get(`${GAMMA_API_URL}/markets`, {
          params: { clob_token_ids: assetId },
          headers: BROWSER_HEADERS,
          timeout: 3000,
          validateStatus: (status) => status < 500
        });

        if (gammaTokenResponse.data && Array.isArray(gammaTokenResponse.data) && gammaTokenResponse.data[0]) {
          const market = gammaTokenResponse.data[0];
          const title = market.question || market.title || "";
          const marketSlug = market.slug || market.market_slug || "";
          const eventSlug =
            market.groupItemSlug ||
            market.eventSlug ||
            market.event_slug ||
            market.parentSlug ||
            market.groupSlug ||
            (market.events && market.events[0] && market.events[0].slug) ||
            (market.event && market.event.slug) ||
            "";
          const imageUrl = market.image || market.icon || market.imageUrl || "";

          if (title && !foundTitle) foundTitle = title;
          if (eventSlug && !foundEventSlug) foundEventSlug = eventSlug;
          if (marketSlug && !foundMarketSlug) foundMarketSlug = marketSlug;
          if (imageUrl && !foundImageUrl) foundImageUrl = imageUrl;
        }
      } catch (e) {
        // Continua
      }
    }

    // ESTRATÉGIA 5: Se temos marketSlug mas não eventSlug, usa marketSlug como eventSlug
    // Alguns mercados simples (não multi-outcome) têm o mesmo slug para ambos
    if (foundTitle && !foundEventSlug && foundMarketSlug) {
      // Verifica se o marketSlug parece válido (não tem números longos)
      const cleanSlug = foundMarketSlug.replace(/-\d{3,}/g, "").replace(/-+$/, "");
      if (cleanSlug.length > 5 && !cleanSlug.match(/^\d+$/)) {
        foundEventSlug = cleanSlug;
        console.log(`   📝 Usando marketSlug limpo como eventSlug: ${cleanSlug}`);
      }
    }

    // Log se ainda não encontrou
    if (foundTitle && !foundEventSlug) {
      console.log(`   ⚠️ Sem eventSlug da API para ${conditionId.slice(0, 8)}`);

      // Se tem marketSlug mas não tem eventSlug, tenta buscar link completo na página
      if (foundMarketSlug && isValidSlug(foundMarketSlug)) {
        console.log(`   🔍 Buscando link completo na página HTML...`);
        const fullUrl = await getMarketUrlFromPage(foundMarketSlug);
        if (fullUrl) {
          foundMarketUrl = fullUrl;
          // Tenta extrair eventSlug do link completo
          const urlMatch = fullUrl.match(/\/event\/([^\/]+)\/([^\/?]+)/);
          if (urlMatch && urlMatch[1] && urlMatch[1] !== foundMarketSlug) {
            foundEventSlug = urlMatch[1];
            console.log(`   ✨ EventSlug extraído do link: ${foundEventSlug}`);
          }
        }
      }
    }

    // Salva o que encontrou (mesmo que parcial)
    if (foundTitle) {
      marketInfoCache.set(conditionId, {
        eventSlug: foundEventSlug,
        marketSlug: foundMarketSlug,
        title: foundTitle,
        imageUrl: foundImageUrl,
        marketUrl: foundMarketUrl,
        timestamp: Date.now()
      });
      return {
        eventSlug: foundEventSlug,
        marketSlug: foundMarketSlug,
        title: foundTitle,
        imageUrl: foundImageUrl,
        marketUrl: foundMarketUrl
      };
    }

    console.warn(`⚠️ Todas estratégias falharam para ${conditionId.slice(0, 8)}`);
    return null;
  } catch (error: any) {
    console.warn(`⚠️ Erro geral ao buscar ${conditionId.slice(0, 8)}:`, error.message);
    return null;
  }
}
*/

// 1. RESOLVER USUÁRIO
export async function resolveUser(input: string): Promise<string | null> {
  const cleanInput = input.trim();

  if (/^0x[a-fA-F0-9]{40}$/i.test(cleanInput)) {
    return cleanInput.toLowerCase();
  }

  let slug = cleanInput
    .replace("https://polymarket.com/@", "")
    .replace("https://polymarket.com/profile/", "")
    .replace("@", "")
    .split("?")[0];

  try {
    const profileUrl = `https://polymarket.com/@${slug}`;
    const { data: html } = await axios.get(profileUrl, {
      headers: BROWSER_HEADERS,
      timeout: 8000
    });

    const patterns = [
      /"proxyWallet":"(0x[a-fA-F0-9]{40})"/i,
      /"address":"(0x[a-fA-F0-9]{40})"/i,
      /wallet["|']:\s*["|'](0x[a-fA-F0-9]{40})["|']/i
    ];

    for (const pattern of patterns) {
      const match = html.match(pattern);
      if (match) {
        console.log(`✅ Resolvido @${slug} → ${match[1]}`);
        return match[1].toLowerCase();
      }
    }

    console.warn(`⚠️ Não encontrei endereço para @${slug}`);
    return null;
  } catch (error: any) {
    console.error(`❌ Erro ao resolver @${slug}:`, error.message);
    return null;
  }
}

/*
async function getQuotePrice(assetId: string, side: "BUY" | "SELL"): Promise<number> {
  const sidePref = side === "SELL" ? "bid" : "ask";

  // 1) Tenta bid/ask
  try {
    const res = await axios.get(`${CLOB_API_URL}/price`, {
      params: { token_id: assetId, side: sidePref },
      headers: BROWSER_HEADERS,
      timeout: 2000,
      validateStatus: (status) => status < 500
    });
    const px = Number(res.data?.price || 0);
    if (px > 0) return px;
  } catch {}

  // 2) Fallback para mid
  try {
    const res = await axios.get(`${CLOB_API_URL}/price`, {
      params: { token_id: assetId, side: "mid" },
      headers: BROWSER_HEADERS,
      timeout: 2000,
      validateStatus: (status) => status < 500
    });
    const px = Number(res.data?.price || 0);
    if (px > 0) return px;
  } catch {}

  return 0;
}

// Helper: obtém outcomes e clobTokenIds do mercado (CLOB) para montar tid
async function getClobMeta(conditionId: string): Promise<{
  outcomes: string[];
  clobTokenIds: Array<string | number>;
} | null> {
  const cached = clobMetaCache.get(conditionId);
  if (cached && Date.now() - cached.timestamp < CLOB_META_CACHE_TTL) {
    return {
      outcomes: cached.outcomes,
      clobTokenIds: cached.clobTokenIds
    };
  }

  try {
    const resp = await axios.get(`${CLOB_API_URL}/markets/${conditionId}`, {
      headers: BROWSER_HEADERS,
      timeout: 4000,
      validateStatus: (status) => status < 500
    });

    if (resp.status !== 200 || !resp.data) return null;

    const outcomes: string[] = (Array.isArray(resp.data.outcomes) ? resp.data.outcomes : []) || [];
    const clobTokenIds: Array<string | number> =
      (Array.isArray(resp.data.clobTokenIds) ? resp.data.clobTokenIds : []) || [];

    if (outcomes.length === 0 && clobTokenIds.length === 0) return null;

    clobMetaCache.set(conditionId, {
      outcomes,
      clobTokenIds,
      timestamp: Date.now()
    });

    return { outcomes, clobTokenIds };
  } catch {
    return null;
  }
}

// 2. BUSCAR ATIVIDADE (Monitora mudanças no portfolio)
export async function fetchRecentActivity(address: string): Promise<PolyActivity[]> {
  try {
    const currentPositions = await fetchPortfolioRaw(address);

    if (currentPositions.length === 0) {
      console.log(`   ℹ️ Portfolio vazio`);
      return [];
    }

    // Cria Map com as posições atuais
    // IMPORTANTE: A chave DEVE incluir o conditionId para diferenciar mercados
    const currentMap = new Map<string, PolyPosition>();
    currentPositions.forEach((pos) => {
      const key = `${pos.conditionId}-${pos.outcome}-${pos.assetId}`;
      currentMap.set(key, pos);
    });

    // Busca snapshot anterior
    const snapshot = portfolioSnapshots.get(address);

    if (!snapshot) {
      console.log(`📸 Snapshot inicial salvo (${currentPositions.length} posições)`);

      portfolioSnapshots.set(address, {
        positions: currentMap,
        timestamp: Date.now()
      });
      return [];
    }

    // Compara com snapshot anterior
    const activities: PolyActivity[] = [];
    const previousMap = snapshot.positions;

    // 1. DETECTA NOVAS POSIÇÕES
    for (const [key, current] of currentMap.entries()) {
      if (!previousMap.has(key)) {
        activities.push({
          id: `${key}-new-${Date.now()}`,
          timestamp: Date.now(),
          type: "Trade",
          marketTitle: current.title,
          outcome: current.outcome,
          side: "BUY",
          price: current.entryPrice,
          amount: current.size,
          eventSlug: current.eventSlug,
          marketSlug: current.marketSlug,
          conditionId: current.conditionId,
          assetId: current.assetId,
          marketImageUrl: current.imageUrl
        });
        console.log(
          `   🆕 Nova: ${current.title.slice(0, 40)} - ${current.outcome} | CondId: ${current.conditionId.slice(
            0,
            8
          )} (${current.size.toFixed(1)} shares)`
        );
      }
    }

    // 2. DETECTA AUMENTOS/DIMINUIÇÕES
    for (const [key, current] of currentMap.entries()) {
      const previous = previousMap.get(key);

      if (previous) {
        const sizeDiff = current.size - previous.size;

        if (sizeDiff > 0.5) {
          const prevInvested = previous.size * previous.entryPrice;
          const currInvested = current.size * current.entryPrice;
          const investmentDiff = currInvested - prevInvested;
          const avgPrice = investmentDiff / sizeDiff;

          activities.push({
            id: `${key}-increase-${Date.now()}`,
            timestamp: Date.now(),
            type: "Trade",
            marketTitle: current.title,
            outcome: current.outcome,
            side: "BUY",
            price: avgPrice > 0 ? avgPrice : current.currentPrice,
            amount: sizeDiff,
            eventSlug: current.eventSlug,
            marketSlug: current.marketSlug,
            conditionId: current.conditionId,
            assetId: current.assetId,
            marketImageUrl: current.imageUrl
          });
          console.log(
            `   📈 Aumentou: ${current.title.slice(0, 40)} - ${
              current.outcome
            } | CondId: ${current.conditionId.slice(0, 8)} +${sizeDiff.toFixed(1)} shares @ ${avgPrice.toFixed(3)}`
          );
        } else if (sizeDiff < -0.5) {
          // Para venda, tenta usar o bid atual como proxy do preço de execução
          let sellPrice = current.currentPrice;
          if (current.assetId) {
            const px = await getQuotePrice(current.assetId, "SELL");
            if (px > 0) sellPrice = px;
          }

          const sharesTraded = Math.abs(sizeDiff);
          const profitUsdPartial = (sellPrice - previous.entryPrice) * sharesTraded;
          const profitPercentPartial = previous.entryPrice > 0
            ? ((sellPrice - previous.entryPrice) / previous.entryPrice) * 100
            : 0;

          activities.push({
            id: `${key}-decrease-${Date.now()}`,
            timestamp: Date.now(),
            type: "Trade",
            marketTitle: current.title,
            outcome: current.outcome,
            side: "SELL",
            price: sellPrice,
            amount: sharesTraded,
            eventSlug: current.eventSlug,
            marketSlug: current.marketSlug,
            conditionId: current.conditionId,
            assetId: current.assetId,
            marketImageUrl: current.imageUrl,
            profitUsd: profitUsdPartial,
            profitPercent: profitPercentPartial,
            avgBuyPrice: previous.entryPrice
          });
          console.log(
            `   📉 Vendeu: ${current.title.slice(0, 40)} - ${
              current.outcome
            } | CondId: ${current.conditionId.slice(0, 8)} ${sizeDiff.toFixed(1)} shares @ ${sellPrice.toFixed(3)}`
          );
        }
      }
    }

    // 3. DETECTA POSIÇÕES FECHADAS
    for (const [key, previous] of previousMap.entries()) {
      if (!currentMap.has(key)) {
        // Usa bid como proxy do preço de execução no fechamento
        let closePrice = previous.currentPrice;
        if (previous.assetId) {
          const px = await getQuotePrice(previous.assetId, "SELL");
          if (px > 0) closePrice = px;
        }

        const profitUsdClose = (closePrice - previous.entryPrice) * previous.size;
        const profitPercentClose = previous.entryPrice > 0
          ? ((closePrice - previous.entryPrice) / previous.entryPrice) * 100
          : 0;

        activities.push({
          id: `${key}-close-${Date.now()}`,
          timestamp: Date.now(),
          type: "Trade",
          marketTitle: previous.title,
          outcome: previous.outcome,
          side: "SELL",
          price: closePrice,
          amount: previous.size,
          eventSlug: previous.eventSlug,
          marketSlug: previous.marketSlug,
          conditionId: previous.conditionId,
          assetId: previous.assetId,
          marketImageUrl: previous.imageUrl,
          profitUsd: profitUsdClose,
          profitPercent: profitPercentClose,
          avgBuyPrice: previous.entryPrice
        });
        console.log(
          `   🔴 Fechou: ${previous.title.slice(0, 40)} - ${previous.outcome} | CondId: ${previous.conditionId.slice(
            0,
            8
          )} (${previous.size.toFixed(1)} shares)`
        );
      }
    }

    // ATUALIZA O SNAPSHOT
    portfolioSnapshots.set(address, {
      positions: currentMap,
      timestamp: Date.now()
    });

    // MELHORIA: Enriquece atividades com informações faltantes
    // IMPORTANTE: Processa cada conditionId único apenas uma vez
    const processedConditions = new Set<string>();

    for (const activity of activities) {
      // Verifica se precisa enriquecer E se ainda não processou esse conditionId
      const needsEnrichment =
        activity.marketTitle.startsWith("Market ") || (!activity.eventSlug && !activity.marketSlug);

      if (needsEnrichment && activity.conditionId && !processedConditions.has(activity.conditionId)) {
        const conditionId = activity.conditionId;
        const marketInfo = await getMarketInfo(conditionId, activity.assetId);
        if (marketInfo) {
          // Atualiza TODAS as atividades com o mesmo conditionId
          for (const act of activities) {
            if (act.conditionId === conditionId) {
              act.marketTitle = marketInfo.title;
              act.eventSlug = marketInfo.eventSlug;
              act.marketSlug = marketInfo.marketSlug;
              if (marketInfo.imageUrl) {
                act.marketImageUrl = marketInfo.imageUrl;
              }
              if (marketInfo.marketUrl) {
                act.marketUrl = marketInfo.marketUrl;
              }
            }
          }

          // Tenta obter clob meta para definir tid por outcome
          const meta = await getClobMeta(conditionId);
          if (meta && meta.clobTokenIds.length > 0) {
            for (const act of activities) {
              if (act.conditionId !== conditionId) continue;

              const outcome = (act.outcome || "").toLowerCase().trim();
              let tid: string | number | undefined;

              if (meta.outcomes.length === meta.clobTokenIds.length && meta.outcomes.length > 0) {
                // Mapeia por nome de outcome
                const idx = meta.outcomes.findIndex((o) => (o || "").toLowerCase().trim() === outcome);
                if (idx >= 0) tid = meta.clobTokenIds[idx];
              }

              // Fallback binário Yes/No: assume [Yes, No]
              if (!tid && meta.clobTokenIds.length === 2) {
                tid = outcome === "yes" ? meta.clobTokenIds[0] : meta.clobTokenIds[1];
              }

              if (tid) {
                act.marketTid = tid;
              }
            }
          }

          processedConditions.add(conditionId);
          console.log(`   ✨ Enriquecido (${conditionId.slice(0, 8)}): ${marketInfo.title.slice(0, 50)}`);
        }
        // Pequeno delay para não sobrecarregar
        await new Promise((r) => setTimeout(r, 100));
      }
    }

    if (activities.length > 0) {
      console.log(`   ✅ Detectou ${activities.length} mudança(s)`);
    }

    return activities;
  } catch (error: any) {
    console.error(`❌ Erro ao monitorar ${address.slice(0, 8)}:`, error.message);
    return [];
  }
}

// 3. BUSCAR PORTFOLIO RAW (melhorado)
async function fetchPortfolioRaw(address: string): Promise<PolyPosition[]> {
  try {
    const response = await axios.get(`${DATA_API_URL}/positions`, {
      params: {
        user: address,
        size_gt: 0.01
      },
      headers: BROWSER_HEADERS,
      timeout: 10000,
      validateStatus: (status) => status < 500
    });

    if (response.status !== 200 || !Array.isArray(response.data)) {
      return [];
    }

    const positions: PolyPosition[] = [];

    for (const pos of response.data) {
      const size = Number(pos.size || 0);
      if (size < 0.01) continue;

      // IMPORTANTE: Captura conditionId PRIMEIRO (é o identificador único do mercado)
      const conditionId = pos.conditionId || pos.condition_id || "";
      const outcome = pos.outcome || "Unknown";
      const assetId = pos.asset || pos.assetId || "";

      // Validação: Pula se não tiver conditionId (não conseguiremos identificar o mercado)
      if (!conditionId) {
        console.warn(`⚠️ Posição sem conditionId, pulando...`);
        continue;
      }

      let title = "Unknown Market";
      let eventSlug = "";
      let marketSlug = "";
      let imageUrl = "";

      // DEBUG: Vamos ver o que a API está retornando
      const marketData = pos.market || {};

      // SEMPRE busca via API usando conditionId (mais confiável)
      if (conditionId) {
        const marketInfo = await getMarketInfo(conditionId, assetId);
        if (marketInfo && marketInfo.title && marketInfo.title.length > 0) {
          title = marketInfo.title;
          eventSlug = marketInfo.eventSlug;
          marketSlug = marketInfo.marketSlug;
          imageUrl = marketInfo.imageUrl || "";
        } else {
          // Fallback: Tenta dados que vieram na posição
          if (marketData.question && marketData.question.length > 0) {
            title = marketData.question;
            marketSlug = marketData.slug || "";
            console.log(`   ⚠️ Fallback question: ${title.slice(0, 50)}`);
          } else if (marketData.title && marketData.title.length > 0) {
            title = marketData.title;
            marketSlug = marketData.slug || "";
            console.log(`   ⚠️ Fallback title: ${title.slice(0, 50)}`);
          } else if (marketData.slug && marketData.slug.length > 0) {
            title = marketData.slug
              .split("-")
              .map((word: string) => word.charAt(0).toUpperCase() + word.slice(1))
              .join(" ");
            marketSlug = marketData.slug;
            console.log(`   ⚠️ Fallback slug: ${title.slice(0, 50)}`);
          } else {
            // Último recurso
            title = `Market ${conditionId.slice(0, 8)}`;
            console.warn(`   ❌ SEM DADOS para ${conditionId.slice(0, 8)}`);
          }
        }
      } else {
        title = `Market ${assetId.slice(0, 8)}`;
        console.warn(`   ❌ Posição sem conditionId!`);
      }

      const entryPrice = Number(pos.avgPrice || 0);
      let currentPrice = entryPrice;

      // Busca preço atual
      try {
        const priceReq = await axios.get(`${CLOB_API_URL}/price`, {
          params: { token_id: assetId, side: "mid" },
          headers: BROWSER_HEADERS,
          timeout: 2000,
          validateStatus: (status) => status < 500
        });

        const fetchedPrice = Number(priceReq.data?.price || 0);
        if (fetchedPrice > 0) {
          currentPrice = fetchedPrice;
        }
      } catch {
        if (pos.market?.outcomePrices && Array.isArray(pos.market.outcomePrices)) {
          const prices = pos.market.outcomePrices.map((p: any) => Number(p));
          if (outcome.toLowerCase() === "yes" && prices[0]) {
            currentPrice = prices[0];
          } else if (outcome.toLowerCase() === "no" && prices[1]) {
            currentPrice = prices[1];
          }
        }
      }

      const invested = size * entryPrice;
      const currentValue = size * currentPrice;
      const pnl = currentValue - invested;
      const pnlPercent = invested > 0 ? (pnl / invested) * 100 : 0;

      positions.push({
        title,
        outcome,
        size,
        entryPrice,
        currentPrice,
        pnl,
        pnlPercent,
        currentValue,
        eventSlug,
        marketSlug,
        assetId,
        conditionId,
        imageUrl: imageUrl || undefined
      });

      await new Promise((r) => setTimeout(r, 100)); // Aumentado para 100ms entre requisições
    }

    // Log resumido no final
    console.log(`   ✅ ${positions.length} posições carregadas`);
    if (positions.length > 0) {
      const uniqueMarkets = new Set(positions.map((p) => p.conditionId)).size;
      console.log(`   📊 ${uniqueMarkets} mercados únicos`);
    }

    return positions;
  } catch (error: any) {
    console.error(`❌ Erro ao buscar portfolio:`, error.message);
    return [];
  }
}

// Fetch de portfolio removido — comando `/portfolio` descontinuado.

// 5-7. Funções auxiliares (inalteradas)
// clearCache removida — não é mais necessária após descontinuação do comando /portfolio.

export async function getUsernameFromAddress(address: string): Promise<string | null> {
  const cached = usernameCache.get(address);
  if (cached && Date.now() - cached.timestamp < USERNAME_CACHE_TTL) {
    return cached.username;
  }

  // Estratégia 1: API oficial de perfil público
  try {
    const { data } = await axios.get(`${GAMMA_API_URL}/public-profile`, {
      params: { address },
      headers: BROWSER_HEADERS,
      timeout: 5000,
      validateStatus: (status) => status < 500
    });

    logger.debug(`   👤 public-profile [${address.slice(0, 8)}]: name="${data.name}" pseudonym="${data.pseudonym}"`);

    const displayName = (data.name && data.name.trim()) || (data.pseudonym && data.pseudonym.trim()) || null;

    if (displayName) {
      usernameCache.set(address, { username: displayName, timestamp: Date.now() });
      return displayName;
    }
  } catch (err: any) {
    logger.warn(`   ⚠️ public-profile falhou para ${address.slice(0, 8)}: ${err.message}`);
  }

  // Estratégia 2 (fallback): Scraping HTML do perfil
  try {
    const profileUrl = `https://polymarket.com/profile/${address}`;
    const { data: html } = await axios.get(profileUrl, {
      headers: BROWSER_HEADERS,
      timeout: 5000
    });

    const patterns = [/"username":"([^"]+)"/i, /"name":"([^"]+)"/i, /<title>([^<|]+)/i];

    for (const pattern of patterns) {
      const match = html.match(pattern);
      if (match && match[1] && !match[1].includes("Polymarket")) {
        const username = match[1].trim();
        usernameCache.set(address, { username, timestamp: Date.now() });
        return username;
      }
    }

    return null;
  } catch (error) {
    return null;
  }
}

export async function testAPIConnection(address: string): Promise<void> {
  console.log(`\n🧪 TESTANDO APIs PARA ${address.slice(0, 8)}...\n`);

  console.log(`1️⃣ Testando DATA /positions...`);
  try {
    const data = await axios.get(`${DATA_API_URL}/positions`, {
      params: { user: address },
      headers: BROWSER_HEADERS,
      timeout: 5000,
      validateStatus: () => true
    });
    console.log(`   Status: ${data.status}`);
    console.log(`   Dados: ${Array.isArray(data.data) ? `${data.data.length} items` : "formato inválido"}`);

    if (data.data && data.data[0]) {
      const sample = data.data[0];
      console.log(`\n📋 SAMPLE DA PRIMEIRA POSIÇÃO:`);
      console.log(`   conditionId: ${sample.conditionId || "N/A"}`);
      console.log(`   outcome: ${sample.outcome || "N/A"}`);
      console.log(`   size: ${sample.size || "N/A"}`);
      console.log(`   market.question: ${sample.market?.question || "N/A"}`);
      console.log(`   market.title: ${sample.market?.title || "N/A"}`);
      console.log(`   market.slug: ${sample.market?.slug || "N/A"}`);

      // Testa buscar info desse mercado
      if (sample.conditionId) {
        console.log(`\n🔍 Testando getMarketInfo para ${sample.conditionId}...`);
        const assetId = sample.asset || sample.assetId || "";
        const info = await getMarketInfo(sample.conditionId, assetId);
        if (info) {
          console.log(`   ✅ Título: ${info.title}`);
          console.log(`   ✅ EventSlug: ${info.eventSlug || "(vazio)"}`);
          console.log(`   ✅ MarketSlug: ${info.marketSlug || "(vazio)"}`);
        } else {
          console.log(`   ❌ Não conseguiu buscar informações`);
        }
      }
    }
  } catch (e: any) {
    console.log(`   ❌ Erro: ${e.message}`);
  }

  console.log(`\n💡 FUNCIONAMENTO ATUAL:`);
  console.log(`   O bot monitora mudanças comparando snapshots do portfolio.`);
  console.log(`   Detecta: novas posições, aumentos, vendas e fechamentos.`);
  console.log(`   ⏱️ Verificação a cada 30s.\n`);
}
*/

// 2. BUSCAR ATIVIDADE via endpoint oficial GET /activity
export async function fetchRecentActivity(address: string, sinceTimestamp: number): Promise<PolyActivity[]> {
  // Normaliza para segundos: wallets TypeScript armazenam em ms
  const sinceSecs =
    sinceTimestamp > 1_000_000_000_000
      ? Math.floor(sinceTimestamp / 1000)
      : sinceTimestamp > 0
        ? sinceTimestamp
        : Math.floor(Date.now() / 1000) - 60;

  try {
    const resp = await axios.get(`${DATA_API_URL}/activity`, {
      params: {
        user: address,
        start: sinceSecs,
        type: "TRADE",
        limit: 100,
        sortBy: "TIMESTAMP",
        sortDirection: "ASC"
      },
      headers: { Accept: "application/json" },
      timeout: 10000,
      validateStatus: (s) => s < 500
    });

    if (resp.status !== 200 || !Array.isArray(resp.data)) {
      if (resp.status !== 200) {
        logger.warn(`⚠️ HTTP ${resp.status} ao buscar /activity de ${address.slice(0, 8)}`);
      }
      return [];
    }

    const activities: PolyActivity[] = [];

    for (const e of resp.data) {
      if (e.timestamp <= sinceSecs) continue;
      const side = (e.side || "").toUpperCase();
      if (side !== "BUY" && side !== "SELL") continue;

      const conditionId = e.conditionId || "";
      const id = e.transactionHash || `${conditionId}-${e.timestamp}`;
      const eventSlug = e.eventSlug || "";
      const marketSlug = e.slug || "";
      const marketUrl = buildMarketUrl(eventSlug, marketSlug);

      let realizedPnl: number | undefined;
      if (side === "SELL" && conditionId) {
        const pnl = await fetchClosedPosition(address, conditionId);
        if (pnl !== null) realizedPnl = pnl;
      }

      const rawName = e.pseudonym || e.name || "";
      const displayName = rawName && rawName !== "null" ? rawName : undefined;

      activities.push({
        id,
        timestamp: e.timestamp,
        type: e.type || "TRADE",
        marketTitle: e.title || "",
        outcome: e.outcome || "",
        side,
        price: Number(e.price) || 0,
        amount: Number(e.size) || 0,
        usdcSize: Number(e.usdcSize) || 0,
        eventSlug,
        marketSlug,
        conditionId,
        assetId: e.asset || "",
        marketImageUrl: e.icon || "",
        marketUrl,
        displayName,
        realizedPnl
      });
    }

    if (activities.length > 0) {
      logger.info(`   ✅ ${activities.length} operação(ões) detectada(s)`);
    } else {
      logger.debug(`   ✓ Sem novas operações`);
    }

    return activities;
  } catch (error: any) {
    logger.error(`❌ Erro ao buscar /activity de ${address.slice(0, 8)}:`, error.message);
    return [];
  }
}

// 3. BUSCAR NOME DO USUÁRIO via GET /public-profile (sem scraping)
export async function getUsernameFromAddress(address: string): Promise<string | null> {
  const cached = usernameCache.get(address);
  if (cached && Date.now() - cached.timestamp < USERNAME_CACHE_TTL) {
    return cached.username;
  }

  try {
    const { data } = await axios.get(`${GAMMA_API_URL}/public-profile`, {
      params: { address },
      headers: { Accept: "application/json" },
      timeout: 5000,
      validateStatus: (s) => s < 500
    });
    const displayName = data.name?.trim() || data.pseudonym?.trim() || null;
    if (displayName) {
      usernameCache.set(address, { username: displayName, timestamp: Date.now() });
      return displayName;
    }
  } catch (err: any) {
    logger.warn(`   ⚠️ public-profile falhou para ${address.slice(0, 8)}: ${err.message}`);
  }

  return null;
}
