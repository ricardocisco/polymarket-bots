const GAMMA_BASE_URL = 'https://gamma-api.polymarket.com';

export function polymarketEventUrl(eventSlug) {
  return `https://polymarket.com/event/${eventSlug}`;
}

export function polymarketMarketUrl(eventSlug, marketSlug) {
  return `${polymarketEventUrl(eventSlug)}/${marketSlug}`;
}

export async function fetchEventBySlug(eventSlug, options = {}) {
  const url = new URL(`/events/slug/${eventSlug}`, GAMMA_BASE_URL);
  if (options.bypassCache) {
    url.searchParams.set('_', String(Date.now()));
  }

  const response = await fetch(url, {
    headers: {
      accept: 'application/json',
      'cache-control': 'no-cache',
      'user-agent': options.userAgent || 'polymarket-discord-alert-bot/1.0',
    },
  });

  if (!response.ok) {
    const body = await response.text();
    throw new Error(`Gamma API retornou HTTP ${response.status}: ${body.slice(0, 300)}`);
  }

  const data = await response.json();
  const event = Array.isArray(data) ? data[0] : data;
  if (!event || !Array.isArray(event.markets)) {
    throw new Error(`Resposta inesperada da Gamma API para o evento ${eventSlug}.`);
  }

  return event;
}
