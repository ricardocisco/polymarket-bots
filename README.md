# polymarket-bots

Colecao de bots e ferramentas para Polymarket. O repositorio mistura bots de trading, backtests e monitoramento de carteiras.

Por seguranca, rode tudo primeiro em `dry-run` ou backtest. Arquivos `.env`, logs, builds e dados gerados ficam fora do git pelo `.gitignore`.

## Projetos

| Projeto | O que faz | Tech principal |
| --- | --- | --- |
| `bots/95c-3min-bot-python` | Sniper para mercados cripto perto do fechamento, buscando entradas caras de alta probabilidade. | Rust atual, Python legado |
| `bots/bayesian-5-15-python` | Bot Bayes + Kelly para mercados cripto de 5m e 15m. | Python |
| `bots/btc-6am-bot-rust` | Estrategia BTC 5m em uma hora UTC configuravel. | Rust |
| `bots/weather-bot-rust` | Bot para mercados de temperatura usando previsoes e historico climatico. | Rust |
| `bots/tracker-wallet-python` | Bot Discord para rastrear carteiras Polymarket. Nao executa trades. | TypeScript/Node |

## Requisitos gerais

- Rust + Cargo para os bots Rust.
- Python 3.11+ para os bots Python.
- Node.js 20+ ou Bun para o tracker.
- Docker opcional para rodar via `docker compose`.
- Conta Polymarket com USDC na Polygon apenas para trading real.

## Configuracao da conta Polymarket

Para bots de trading real, use uma wallet separada e com pouco saldo.

Variaveis comuns:

```env
POLYMARKET_PRIVATE_KEY=
POLYMARKET_FUNDER=
POLYMARKET_SIGNATURE_TYPE=proxy
DRY_RUN=true
ALLOW_LIVE_TRADING=false
```

Notas:

- `POLYMARKET_PRIVATE_KEY`: chave privada da wallet. Nunca commite.
- `POLYMARKET_FUNDER`: endereco publico da conta Polymarket, quando o bot pedir.
- `POLYMARKET_SIGNATURE_TYPE`: em geral `proxy` ou `1` para conta criada via Polymarket/Google/email; `eoa` ou `0` para wallet direta; `gnosis` ou `2` para Safe.
- Para operar real, valide antes em backtest e dry-run. Depois mude `DRY_RUN=false` e, nos bots Rust que usam trava extra, `ALLOW_LIVE_TRADING=true`.

## Rodando com Docker

A partir da pasta `bots/`:

```bash
cd bots
docker compose --profile sniper up --build sniper-bot
docker compose --profile bayesian up --build bayesian-bot
docker compose --profile btc6am up --build btc-6am-bot
docker compose --profile weather up --build weather-bot
docker compose --profile tracker up --build tracker-bot
```

Cada servico le o `.env` do respectivo projeto se ele existir.

## Fluxo recomendado

1. Entre na pasta do bot.
2. Leia o README do projeto.
3. Crie o `.env` a partir do `.env.example` quando existir.
4. Rode backtest.
5. Rode em `dry-run`.
6. So depois considere trading real.

