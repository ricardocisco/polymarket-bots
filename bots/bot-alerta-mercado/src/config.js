import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const DEFAULT_EVENT_URL =
  'https://polymarket.com/event/brazil-presidential-election/will-renan-santos-win-the-2026-brazilian-presidential-election';

const rootDir = path.resolve(fileURLToPath(new URL('..', import.meta.url)));

export function loadDotEnv(filePath = path.join(rootDir, '.env')) {
  if (!fs.existsSync(filePath)) {
    return;
  }

  const lines = fs.readFileSync(filePath, 'utf8').split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) {
      continue;
    }

    const equalsIndex = trimmed.indexOf('=');
    if (equalsIndex === -1) {
      continue;
    }

    const key = trimmed.slice(0, equalsIndex).trim();
    let value = trimmed.slice(equalsIndex + 1).trim();
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1);
    }

    if (key && process.env[key] === undefined) {
      process.env[key] = value;
    }
  }
}

export function extractEventSlug(input) {
  if (!input) {
    throw new Error('EVENT_URL ou EVENT_SLUG precisa ser configurado.');
  }

  if (!input.includes('://')) {
    return input.trim().replace(/^\/+|\/+$/g, '');
  }

  const url = new URL(input);
  const parts = url.pathname.split('/').filter(Boolean);
  const eventIndex = parts.indexOf('event');
  if (eventIndex === -1 || !parts[eventIndex + 1]) {
    throw new Error(`Nao consegui extrair o slug do evento de: ${input}`);
  }

  return parts[eventIndex + 1];
}

function booleanEnv(name, defaultValue) {
  const value = process.env[name];
  if (value === undefined || value === '') {
    return defaultValue;
  }

  return ['1', 'true', 'yes', 'sim', 'y'].includes(value.toLowerCase());
}

function numberEnv(name, defaultValue) {
  const value = Number(process.env[name]);
  if (!Number.isFinite(value) || value <= 0) {
    return defaultValue;
  }

  return value;
}

export function getConfig() {
  loadDotEnv();

  const eventUrl = process.env.EVENT_URL || DEFAULT_EVENT_URL;
  const eventSlug = process.env.EVENT_SLUG || extractEventSlug(eventUrl);
  const stateFile = path.resolve(rootDir, process.env.STATE_FILE || 'data/state.json');
  const pollIntervalSeconds = Math.max(numberEnv('POLL_INTERVAL_SECONDS', 60), 10);

  return {
    rootDir,
    eventUrl,
    eventSlug,
    stateFile,
    pollIntervalMs: pollIntervalSeconds * 1000,
    bypassCache: booleanEnv('BYPASS_CACHE', true),
    dryRun: booleanEnv('DRY_RUN', false),
    notifyOnFirstRun: booleanEnv('NOTIFY_ON_FIRST_RUN', false),
    discordWebhookUrl: process.env.DISCORD_WEBHOOK_URL || '',
    discordMention: process.env.DISCORD_MENTION || '',
    userAgent:
      process.env.USER_AGENT ||
      'polymarket-discord-alert-bot/1.0 (+https://docs.polymarket.com)',
  };
}
