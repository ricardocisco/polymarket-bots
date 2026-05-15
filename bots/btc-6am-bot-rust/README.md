# BTC 6am Bot

Bot Rust para operar mercados BTC de 5 minutos em uma hora UTC configuravel. O nome vem da ideia original de operar perto de 06:00 UTC, mas a hora real vem de `TARGET_HOUR_UTC`.

## Como funciona

O bot busca mercados BTC de 5m na janela configurada, escolhe `UP` ou `DOWN`, valida preco/edge/liquidez e registra paper trade ou envia ordem real.

Por padrao:

- `DRY_RUN=true`
- `ALLOW_LIVE_TRADING=false`
- `TARGET_HOUR_UTC=7`
- `TRADE_DIRECTION=up`

## Techs

- Rust 2021
- Tokio
- Reqwest
- Serde/serde_json
- rust_decimal
- dotenvy
- polymarket-client-sdk
- Gamma/CLOB APIs da Polymarket

## Instalar

```bash
cd bots/btc-6am-bot-rust
copy .env.example .env
cargo build
```

No Linux/macOS, use `cp .env.example .env`.

## Configurar conta

Principais variaveis:

```env
POLYMARKET_PRIVATE_KEY=
POLYMARKET_SIGNATURE_TYPE=eoa
DRY_RUN=true
ALLOW_LIVE_TRADING=false
PAPER_TRADES_PATH=data/paper_trades.json

TARGET_HOUR_UTC=7
TRADE_DIRECTION=up
POSITION_SIZE_USDC=5.0
MAX_ENTRY_PRICE=0.55
MIN_EDGE=0.02
```

Para trading real, use uma wallet dedicada com pouco saldo, teste antes, depois configure `DRY_RUN=false` e `ALLOW_LIVE_TRADING=true`.

## Rodar

```bash
cargo run --bin bot
```

Resolver paper trades abertos:

```bash
cargo run --bin settle_open_trades
```

Via Docker, a partir de `bots/`:

```bash
docker compose --profile btc6am up --build btc-6am-bot
```

## Backtest

```bash
cargo run --bin backtest -- --days 30
cargo run --bin sweep -- --days 7
```

O `backtest` varre horas UTC e mostra resultado por hora. O `sweep` usa a configuracao atual do `.env`.

