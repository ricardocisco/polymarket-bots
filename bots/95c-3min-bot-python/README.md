# Polymarket Sniper 95c

Bot sniper para mercados cripto da Polymarket. A versao atual roda em Rust; os arquivos Python antigos ficaram como referencia.

## Como funciona

O bot procura mercados BTC/ETH/XRP de 5m ou 15m perto do fechamento. Ele tenta entrar quando:

- faltam poucos minutos para resolver;
- o preco do lado escolhido esta na faixa configurada, por padrao `0.95` a `0.995`;
- indicadores simples de candles da Binance confirmam o lado;
- ainda nao existe trade aberto naquele mercado.

Por padrao ele grava paper trades em `data/paper_trades.json`. Trading real so acontece se `DRY_RUN=false` e `ALLOW_LIVE_TRADING=true`.

## Techs

- Rust 2021
- Tokio
- Reqwest
- Serde/serde_json
- dotenvy
- polymarket-client-sdk
- Binance HTTP API
- Gamma/CLOB APIs da Polymarket

## Instalar

```bash
cd bots/95c-3min-bot-python
cargo build
```

## Configurar conta

Crie um `.env` local:

```env
DRY_RUN=true
ALLOW_LIVE_TRADING=false
BANKROLL=20
POSITION_SIZE_USDC=2
LOOP_INTERVAL=5
MIN_EDGE=0.04
MIN_ENTRY_PRICE=0.95
MAX_ENTRY_PRICE=0.995
PAPER_TRADES_PATH=data/paper_trades.json

# Apenas para trading real
POLYMARKET_PRIVATE_KEY=
POLYMARKET_SIGNATURE_TYPE=proxy
```

Use `proxy` para a maioria das contas criadas pelo site da Polymarket. Use `eoa` se for uma wallet direta.

## Rodar

```bash
cargo run --bin bot
```

Para reconciliar paper trades abertos depois de reiniciar:

```bash
cargo run --bin settle_open_trades
```

Via Docker, a partir de `bots/`:

```bash
docker compose --profile sniper up --build sniper-bot
```

## Backtest

O backtest usa dados publicos da Polymarket/CLOB e candles da Binance. Nao envia ordens.

```bash
cargo run --bin backtest -- --days 3
cargo run --bin backtest -- --days 7 --asset BTC --interval 5 --trades
cargo run --bin sweep -- --days 3 --asset ETH --interval 15
```

