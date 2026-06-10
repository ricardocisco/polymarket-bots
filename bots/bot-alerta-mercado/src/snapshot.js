import { polymarketMarketUrl } from './polymarket.js';

const PLACEHOLDER_PATTERN = /^(Person [A-Z]|Other)$/i;

function parseMaybeJson(value, fallback = null) {
  if (Array.isArray(value)) {
    return value;
  }

  if (typeof value !== 'string' || value === '') {
    return fallback;
  }

  try {
    return JSON.parse(value);
  } catch {
    return fallback;
  }
}

function candidateName(market) {
  return market.groupItemTitle || market.question || market.slug || market.id;
}

function normalizeMarket(market, eventSlug) {
  const outcomePrices = parseMaybeJson(market.outcomePrices, []);

  return {
    id: String(market.id),
    conditionId: market.conditionId || '',
    slug: market.slug || '',
    question: market.question || '',
    candidate: candidateName(market),
    isPlaceholder: PLACEHOLDER_PATTERN.test(candidateName(market)),
    active: Boolean(market.active),
    closed: Boolean(market.closed),
    archived: Boolean(market.archived),
    acceptingOrders: Boolean(market.acceptingOrders),
    enableOrderBook: Boolean(market.enableOrderBook),
    bestBid: market.bestBid ?? null,
    bestAsk: market.bestAsk ?? null,
    lastTradePrice: market.lastTradePrice ?? null,
    outcomePrices,
    updatedAt: market.updatedAt || null,
    url: market.slug ? polymarketMarketUrl(eventSlug, market.slug) : '',
  };
}

export function buildSnapshot(event) {
  const markets = [...event.markets]
    .map((market) => normalizeMarket(market, event.slug))
    .sort((a, b) => a.id.localeCompare(b.id, undefined, { numeric: true }));

  return {
    eventId: String(event.id),
    eventSlug: event.slug,
    eventTitle: event.title,
    eventUpdatedAt: event.updatedAt || null,
    checkedAt: new Date().toISOString(),
    marketCount: markets.length,
    markets: Object.fromEntries(markets.map((market) => [market.id, market])),
  };
}

function marketLabel(market) {
  return market.candidate || market.question || market.slug || market.id;
}

function changed(previous, current, field) {
  return previous[field] !== current[field];
}

export function diffSnapshots(previous, current) {
  if (!previous) {
    return [];
  }

  const changes = [];
  const previousIds = new Set(Object.keys(previous.markets || {}));
  const currentIds = new Set(Object.keys(current.markets || {}));

  for (const id of currentIds) {
    const market = current.markets[id];
    if (!previousIds.has(id) && !market.isPlaceholder) {
      changes.push({
        type: 'candidate_added',
        severity: 'high',
        market,
        title: `Novo candidato adicionado: ${marketLabel(market)}`,
      });
    }
  }

  for (const id of currentIds) {
    if (!previousIds.has(id)) {
      continue;
    }

    const before = previous.markets[id];
    const after = current.markets[id];

    if (
      changed(before, after, 'candidate') ||
      changed(before, after, 'question') ||
      changed(before, after, 'slug')
    ) {
      if (before.isPlaceholder && !after.isPlaceholder) {
        changes.push({
          type: 'candidate_added',
          severity: 'high',
          market: after,
          before,
          title: `Novo candidato adicionado: ${marketLabel(after)}`,
        });
      }
    }
  }

  return changes;
}

function formatPrice(value) {
  if (value === null || value === undefined || value === '') {
    return 'n/d';
  }

  const number = Number(value);
  if (!Number.isFinite(number)) {
    return String(value);
  }

  return `${(number * 100).toFixed(number < 0.01 ? 2 : 1)}%`;
}

function describeChange(change) {
  const market = change.market;
  const lines = [
    `Tipo: ${change.type}`,
    `Ativo: ${market.active ? 'sim' : 'nao'} | Ordens: ${market.acceptingOrders ? 'sim' : 'nao'}`,
    `Bid/Ask: ${formatPrice(market.bestBid)} / ${formatPrice(market.bestAsk)}`,
  ];

  if (change.before) {
    lines.push(`Antes: ${marketLabel(change.before)}`);
  }

  if (market.url) {
    lines.push(market.url);
  }

  return lines.join('\n');
}

export function buildDiscordPayload(snapshot, changes, options = {}) {
  const highestSeverity = changes.some((change) => change.severity === 'high') ? 'high' : 'medium';
  const visibleChanges = changes.slice(0, 10);
  const suffix =
    changes.length > visibleChanges.length
      ? `\nMais ${changes.length - visibleChanges.length} candidato(s) omitido(s).`
      : '';

  return {
    content: [options.discordMention, 'Alerta Polymarket: novo candidato adicionado. Conferir oportunidade de compra.']
      .filter(Boolean)
      .join(' '),
    embeds: [
      {
        title: `${snapshot.eventTitle}: novo candidato`,
        url: `https://polymarket.com/event/${snapshot.eventSlug}`,
        description: `${changes.length} candidato(s) novo(s) detectado(s).${suffix}`,
        color: highestSeverity === 'high' ? 0xdc2626 : 0xf59e0b,
        fields: visibleChanges.map((change) => ({
          name: change.title.slice(0, 256),
          value: describeChange(change).slice(0, 1024),
          inline: false,
        })),
        footer: {
          text: `Evento ${snapshot.eventSlug} | checado em ${snapshot.checkedAt}`,
        },
      },
    ],
  };
}
