import { extractEventSlug } from '../src/config.js';
import { fetchEventBySlug } from '../src/polymarket.js';
import { buildDiscordPayload, buildSnapshot, diffSnapshots } from '../src/snapshot.js';

const DEFAULT_URL = 'https://polymarket.com/event/brazil-presidential-election';

const eventUrl = process.argv[2] || DEFAULT_URL;
const slug = extractEventSlug(eventUrl);

const event = await fetchEventBySlug(slug, { bypassCache: true });
const snapshot = buildSnapshot(event);
const markets = Object.values(snapshot.markets);
const candidates = markets.filter((market) => !market.isPlaceholder);
const placeholders = markets.filter((market) => market.isPlaceholder);

console.log(`URL: ${eventUrl}`);
console.log(`Slug extraido: ${slug}`);
console.log(`Evento: ${snapshot.eventTitle}`);
console.log(`Total de mercados retornados: ${markets.length}`);
console.log(`Candidatos disponiveis: ${candidates.length}`);
console.log(`Placeholders: ${placeholders.length}`);
console.log('');
console.log('Candidatos atuais:');

for (const [index, market] of candidates.entries()) {
  const bid = market.bestBid ?? 'n/d';
  const ask = market.bestAsk ?? 'n/d';
  console.log(`${index + 1}. ${market.candidate} | active=${market.active} | bid=${bid} | ask=${ask}`);
}

const syntheticEvent = structuredClone(event);
syntheticEvent.markets.push({
  id: '999999999',
  slug: 'will-teste-novo-candidato-win-the-2026-brazilian-presidential-election',
  question: 'Will Teste Novo Candidato win the 2026 Brazilian presidential election?',
  groupItemTitle: 'Teste Novo Candidato',
  active: true,
  closed: false,
  archived: false,
  acceptingOrders: true,
  enableOrderBook: true,
  bestBid: 0.01,
  bestAsk: 0.02,
  lastTradePrice: 0.01,
  updatedAt: new Date().toISOString(),
});

const syntheticSnapshot = buildSnapshot(syntheticEvent);
const changes = diffSnapshots(snapshot, syntheticSnapshot);
const payload = changes.length > 0 ? buildDiscordPayload(syntheticSnapshot, changes) : null;

console.log('');
console.log('Simulacao de candidato novo:');
console.log(`Alertaria? ${changes.length > 0 ? 'SIM' : 'NAO'}`);
console.log(`Mudancas detectadas: ${changes.length}`);

for (const change of changes) {
  console.log(`- ${change.type}: ${change.market.candidate}`);
}

if (payload) {
  console.log('');
  console.log('Preview da mensagem Discord:');
  console.log(payload.content);
  console.log(payload.embeds[0].title);
  console.log(payload.embeds[0].description);
}
