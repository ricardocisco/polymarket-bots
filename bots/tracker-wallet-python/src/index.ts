import "dotenv/config";
import { createRequire } from "module";
import dns from "dns";
import { Elysia } from "elysia";
import mongoose from "mongoose";
import { client } from "./bot.js";
import { startTrackerLoop } from "./tracker.js";

// Configura o 'require' manual para ler JSON
const require = createRequire(import.meta.url);
const { version } = require("../package.json");

// Configuração
const PORT = process.env.PORT || 3000;
const MONGO_URI = requireEnv("MONGO_URI");
const DISCORD_TOKEN = requireEnv("DISCORD_TOKEN");

const DEFAULT_SRV_DNS_SERVERS = ["1.1.1.1", "8.8.8.8"];

function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    console.error(`Missing required environment variable: ${name}`);
    process.exit(1);
  }
  return value;
}

function getMongoSrvHost(uri: string): string | null {
  const match = uri.match(/^mongodb\+srv:\/\/(?:[^@]+@)?([^/?]+)/i);
  return match?.[1] ?? null;
}

function parseDnsServers(value: string | undefined): string[] {
  return (value ?? "")
    .split(",")
    .map((server) => server.trim())
    .filter(Boolean);
}

function configureMongoSrvDns(uri: string): void {
  if (!uri.toLowerCase().startsWith("mongodb+srv://")) return;

  const configuredServers = parseDnsServers(process.env.DNS_SERVERS);
  const currentServers = dns.getServers();
  const usesLocalDnsOnly =
    currentServers.length > 0 && currentServers.every((server) => server === "127.0.0.1" || server === "::1");

  if (configuredServers.length > 0) {
    dns.setServers(configuredServers);
    console.log(`MongoDB SRV DNS servers: ${configuredServers.join(", ")}`);
    return;
  }

  if (usesLocalDnsOnly) {
    dns.setServers(DEFAULT_SRV_DNS_SERVERS);
    console.log(`MongoDB SRV DNS servers: ${DEFAULT_SRV_DNS_SERVERS.join(", ")}`);
  }
}

function printMongoConnectionError(error: any, uri: string): void {
  const srvHost = getMongoSrvHost(uri);

  if (error?.syscall === "querySrv" || error?.message?.includes("querySrv")) {
    console.error("Failed to resolve the MongoDB Atlas SRV DNS record.");
    if (srvHost) {
      console.error(`Host: ${srvHost}`);
      console.error(`SRV record: _mongodb._tcp.${srvHost}`);
    }
    console.error("Set DNS_SERVERS=1.1.1.1,8.8.8.8 in .env, or use a standard mongodb:// Atlas connection string.");
    console.error(`Original error: ${error.code ?? "UNKNOWN"} ${error.message}`);
    return;
  }

  console.error("Failed to connect to MongoDB.");
  console.error(error?.message ?? error);
}

// 1. Conexão MongoDB
configureMongoSrvDns(MONGO_URI);

try {
  await mongoose.connect(MONGO_URI);
  console.log("📦 MongoDB Conectado!");
} catch (error: any) {
  printMongoConnectionError(error, MONGO_URI);
  process.exit(1);
}

// No início do arquivo

// 2. Inicializa o Bot Discord
client.once("ready", () => {
  console.log(`🤖 Bot logado como ${client.user?.tag}`);
  console.log(`🚀 Polymarket Tracker v${version} iniciando...`);

  // Inicia o Worker de Rastreamento
  startTrackerLoop(client);
});

try {
  await client.login(DISCORD_TOKEN);
} catch (error: any) {
  console.error("Failed to login to Discord.");
  console.error(error?.message ?? error);
  client.destroy();
  await mongoose.disconnect().catch(() => undefined);
  process.exitCode = 1;
}

// 3. Servidor Elysia (API Backend)
// Útil para adicionar carteiras via API externa sem usar o Discord
// const app = new Elysia()
//   .get("/", () => "Polymarket Tracker is Running 🚀")

//   .get("/stats", async () => {
//     const walletCount = await Wallet.countDocuments();
//     const subCount = await Subscription.countDocuments();
//     return { wallets_tracked: walletCount, active_channels: subCount };
//   })

//   .post("/api/track", async ({ body }: any) => {
//     const { channelId, walletAddress } = body;
//     // Lógica de API para adicionar tracker externamente
//     if (!channelId || !walletAddress) throw new Error("Dados inválidos");

//     // (Reutilize a lógica de criação do bot.ts aqui se quiser abstrair)
//     return { status: "Feature via API implementada" };
//   })

//   .listen(PORT);

// console.log(`🦊 Elysia Backend rodando em http://localhost:${PORT}`);
// Teste2 para garantir a VPS
