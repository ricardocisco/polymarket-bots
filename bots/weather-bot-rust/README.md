# Polymarket Weather Bot

Bot Rust para mercados de temperatura diaria na Polymarket. Ele usa dados meteorologicos, consenso entre fontes e regras de risco para decidir entradas em mercados `YES/NO`.

## Como funciona

O bot descobre mercados de temperatura, identifica cidade/estacao, busca previsoes e compara com os ranges do mercado. Quando existe edge suficiente, ele registra simulacao/dry-run ou envia ordem real.

Tambem existe um monitor de simulacao (`sim_monitor`) para acompanhar oportunidades sem operar.

## Techs

- Rust 2021
- Tokio
- Reqwest
- Serde/serde_json
- rust_decimal
- dotenvy
- tokio-postgres opcional
- Open-Meteo API
- Gamma/CLOB APIs da Polymarket

## Instalar

```bash
cd bots/weather-bot-rust
copy .env.example .env
cargo build --release
```

No Linux/macOS, use `cp .env.example .env`.

## Configurar conta

Principais variaveis:

```env
POLYMARKET_PRIVATE_KEY=
DRY_RUN=true
ALLOW_LIVE_TRADING=false
MIN_CONFIDENCE=0.60
BANKROLL_USDC=100.0
MAX_POSITION_SIZE_USDC=10.0
MIN_ORDER_SIZE_USDC=1.0
RUN_INTERVAL_SECS=3600
DATABASE_URL=
WEATHER_COM_API_KEY=
MAX_QUOTE_AGE_SECS=15
MAX_OPEN_POSITIONS=10
RUST_LOG=info,warn
```

`POLYMARKET_PRIVATE_KEY` so e necessaria para trading real. Para dry-run, simulacao e backtest, deixe `DRY_RUN=true`.

## Rodar

Validar ambiente:

```bash
cargo run --bin setup
```

Rodar bot:

```bash
cargo run --release --bin bot
```

Monitorar simulacao:

```bash
cargo run --release --bin sim_monitor
cargo run --release --bin sim_monitor -- --interval 300
cargo run --release --bin sim_monitor -- --report
```

Via Docker, a partir de `bots/`:

```bash
docker compose --profile weather up --build weather-bot
```

## Backtest

```bash
cargo run --bin backtest -- --allow-hindsight
cargo run --bin backtest -- --allow-hindsight --days 30
cargo run --bin backtest -- --allow-hindsight --days 60 --min-confidence 0.95
cargo run --bin backtest -- --allow-hindsight --days 14 --min-confidence 0.90 --max-position 5.0
```

Observacao: este comando usa temperatura e precos resolvidos, portanto e apenas uma auditoria hindsight. Ele exige `--allow-hindsight` e nao deve ser interpretado como retorno historico executavel. Para validar a estrategia, use snapshots de previsao/order book gravados em tempo real.
