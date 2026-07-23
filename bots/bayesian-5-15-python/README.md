# Polymarket Bayesian 5/15 Bot

Bot para mercados cripto `Up or Down` de 5m e 15m na Polymarket. A mecanica principal foi portada para Rust; os arquivos Python ficaram como legado/referencia.

## Como funciona

O engine Rust monitora BTC, ETH, SOL e XRP. Para cada mercado ativo, ele:

- busca candles de 1 minuto da Binance;
- calcula sinais de RSI, EMA, volume, volatilidade, distancia do strike e momentum curto;
- combina os sinais com Naive Bayes;
- valida edge, faixa de preco e liquidez do order book;
- usa Kelly fracionario para validar o risco;
- grava paper trade ou envia ordem real, conforme configuracao.

## Techs

- Rust 2021
- Tokio
- Reqwest
- Serde/serde_json
- dotenvy
- polymarket-client-sdk
- Binance API
- Gamma/CLOB APIs da Polymarket

## Instalar

```bash
cd bots/bayesian-5-15-python
copy .env.example .env
cargo build
```

No Linux/macOS, use `cp .env.example .env`.

## Configurar conta

Principais variaveis:

```env
DRY_RUN=true
ALLOW_LIVE_TRADING=false
BAYESIAN_MODE=CONSERVATIVE
BAYESIAN_BANKROLL=20
BAYESIAN_FLAT_STAKE_USDC=0
PAPER_TRADES_PATH=data/paper_trades.json

BAYESIAN_MIN_BUY_PRICE=0.50
BAYESIAN_MAX_BUY_PRICE=0.58
BAYESIAN_MIN_ASK_SIZE_USD=5.0

# Apenas para trading real
POLYMARKET_PRIVATE_KEY=
POLYMARKET_SIGNATURE_TYPE=proxy
```

Use `proxy` para a maioria das contas criadas pelo site da Polymarket. Use `eoa` para wallet direta e `gnosis` para Safe quando aplicavel.

## Rodar

Primeiro em dry-run:

```bash
cargo run --bin bot
```

Para trading real, valide antes e depois configure:

```env
DRY_RUN=false
ALLOW_LIVE_TRADING=true
```

Via Docker, a partir de `bots/`:

```bash
docker compose --profile bayesian up --build bayesian-bot
```

## Backtest

O backtest usa dados publicos da Polymarket/CLOB e candles historicos da Binance. Nao envia ordens.

```bash
cargo run --bin backtest -- --days 3
cargo run --bin backtest -- --days 7 --asset BTC --interval 5 --trades
cargo run --bin backtest -- --days 14 --stake 1
cargo run --bin sweep -- --days 3 --asset ETH --interval 15
```

Resolver paper trades abertos:

```bash
cargo run --bin settle_open_trades
```
