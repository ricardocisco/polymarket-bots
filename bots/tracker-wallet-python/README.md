# Polymarket Tracker Bot

Bot Discord para rastrear carteiras da Polymarket e avisar quando novas posicoes ou alteracoes aparecem. Este projeto nao executa trades.

## Como funciona

O bot salva carteiras monitoradas no MongoDB. Em loop, ele consulta dados publicos da Polymarket, compara com o ultimo estado conhecido e envia alertas no canal Discord configurado.

Comandos principais:

- `/track <endereco_ou_username>`
- `/untrack <endereco_ou_username>`
- `/list`
- `/filter`
- `/help`

## Techs

- TypeScript
- Node.js 20
- Discord.js
- MongoDB/Mongoose
- Elysia
- Axios
- Rust port experimental em `rust/`

## Instalar

```bash
cd bots/tracker-wallet-python
npm install
npm run build
```

## Configurar conta

Este bot precisa de credenciais do Discord e MongoDB, nao de chave privada da Polymarket.

Crie `.env`:

```env
DISCORD_TOKEN=
CLIENT_ID=
MONGO_URI=mongodb://localhost:27017/polybot
PORT=3000
LOG_LEVEL=INFO
```

No Discord Developer Portal, crie uma aplicacao, adicione um bot, copie `DISCORD_TOKEN` e `CLIENT_ID`, e convide o bot para seu servidor com permissao de slash commands.

## Rodar

Registrar comandos slash:

```bash
node dist/deploy-command.js
```

Iniciar bot:

```bash
npm start
```

Via Docker, a partir de `bots/`:

```bash
docker compose --profile tracker up --build tracker-bot
```

## Backtest

Nao ha backtest porque este projeto nao faz trading. Para testar sem risco:

1. Rode MongoDB local ou via Docker.
2. Registre os comandos.
3. Use `/track` em uma carteira conhecida.
4. Acompanhe logs com `LOG_LEVEL=DEBUG`.
5. Use `/untrack` para remover a carteira.

