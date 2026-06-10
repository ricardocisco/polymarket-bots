import { getConfig } from '../src/config.js';
import { sendDiscordWebhook } from '../src/discord.js';
import { fetchEventBySlug } from '../src/polymarket.js';
import { buildDiscordPayload, buildSnapshot, diffSnapshots } from '../src/snapshot.js';

const config = getConfig();

const event = await fetchEventBySlug(config.eventSlug, {
  bypassCache: true,
  userAgent: config.userAgent,
});

const snapshot = buildSnapshot(event);
const syntheticEvent = structuredClone(event);
const testCandidateName = process.env.TEST_CANDIDATE_NAME || 'Teste Novo Candidato';
const safeSlugName = testCandidateName
  .normalize('NFD')
  .replace(/[\u0300-\u036f]/g, '')
  .toLowerCase()
  .replace(/[^a-z0-9]+/g, '-')
  .replace(/^-|-$/g, '');

syntheticEvent.markets.push({
  id: `test-${Date.now()}`,
  slug: `will-${safeSlugName}-win-the-2026-brazilian-presidential-election`,
  question: `Will ${testCandidateName} win the 2026 Brazilian presidential election?`,
  groupItemTitle: testCandidateName,
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

if (changes.length === 0) {
  throw new Error('A simulacao nao gerou candidate_added. Verifique a regra de diff.');
}

const payload = buildDiscordPayload(syntheticSnapshot, changes, {
  discordMention: config.discordMention,
});

console.log(`Simulacao: ${changes.length} candidato(s) novo(s).`);
console.log(`Candidato: ${testCandidateName}`);
console.log(`Webhook configurado: ${config.discordWebhookUrl ? 'sim' : 'nao'}`);

if (config.dryRun) {
  console.log('DRY_RUN=true. Payload que seria enviado:');
  console.log(JSON.stringify(payload, null, 2));
} else {
  await sendDiscordWebhook(config.discordWebhookUrl, payload);
  console.log('Mensagem de teste enviada ao Discord.');
}
