import { getConfig } from './config.js';
import { sendDiscordWebhook } from './discord.js';
import { fetchEventBySlug } from './polymarket.js';
import { buildDiscordPayload, buildSnapshot, diffSnapshots } from './snapshot.js';
import { readState, writeState } from './state.js';
import { pathToFileURL } from 'node:url';

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function log(message) {
  console.log(`[${new Date().toISOString()}] ${message}`);
}

async function notify(config, snapshot, changes) {
  const payload = buildDiscordPayload(snapshot, changes, {
    discordMention: config.discordMention,
  });

  if (config.dryRun || !config.discordWebhookUrl) {
    log('DRY_RUN ativo ou webhook ausente. Payload que seria enviado:');
    console.log(JSON.stringify(payload, null, 2));
    return;
  }

  await sendDiscordWebhook(config.discordWebhookUrl, payload);
  log(`Notificacao enviada ao Discord com ${changes.length} mudanca(s).`);
}

export async function checkOnce(config) {
  const event = await fetchEventBySlug(config.eventSlug, {
    bypassCache: config.bypassCache,
    userAgent: config.userAgent,
  });
  const snapshot = buildSnapshot(event);
  const previous = readState(config.stateFile);

  if (!previous) {
    writeState(config.stateFile, snapshot);
    log(`Baseline salvo: ${snapshot.marketCount} mercado(s) em ${snapshot.eventTitle}.`);

    if (config.notifyOnFirstRun) {
      const firstRunChanges = Object.values(snapshot.markets)
        .filter((market) => !market.isPlaceholder)
        .map((market) => ({
          type: 'candidate_added',
          severity: 'high',
          market,
          title: `Candidato atual: ${market.candidate}`,
        }));
      if (firstRunChanges.length > 0) {
        await notify(config, snapshot, firstRunChanges);
      }
    }

    return { snapshot, changes: [] };
  }

  const changes = diffSnapshots(previous, snapshot);
  writeState(config.stateFile, snapshot);

  if (changes.length === 0) {
    log(`Sem candidato novo. ${snapshot.marketCount} mercado(s) monitorado(s).`);
    return { snapshot, changes };
  }

  log(`${changes.length} candidato(s) novo(s) detectado(s).`);
  await notify(config, snapshot, changes);

  return { snapshot, changes };
}

async function main() {
  const config = getConfig();
  const once = process.argv.includes('--once');

  log(`Monitorando evento ${config.eventSlug}. Intervalo: ${config.pollIntervalMs / 1000}s.`);

  if (once) {
    await checkOnce(config);
    return;
  }

  while (true) {
    try {
      await checkOnce(config);
    } catch (error) {
      console.error(`[${new Date().toISOString()}] Erro:`, error);
    }

    await sleep(config.pollIntervalMs);
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
}
