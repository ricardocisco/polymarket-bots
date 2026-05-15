# BTC 6am Bot

Bot em Rust para a estratégia inferida do arquivo `polybacktest-free-money-on-polymarket-at-6am-2038807374060990916.md`.

## Leitura da estratégia

O post fala em:

- `9.075` mercados BTC de `5 minutos`
- `384` trades
- uma regra ligada às `06:00 UTC`
- taxa de acerto de `57,8%`

A inferência operacional mais consistente é:

1. operar **todos** os mercados BTC de 5 minutos iniciados entre `06:00:00` e `06:59:59 UTC`
2. comprar sempre o mesmo lado direcional
3. repetir isso ao longo dos dias

Como o arquivo não traz explicitamente se o lado é `UP` ou `DOWN`, o projeto usa `UP` por padrão, mas isso é configurável via `TRADE_DIRECTION`.

Observacao: para testes locais, o default atual de `TARGET_HOUR_UTC` foi alterado para `7`.

## Endpoints usados

Baseado na documentação MCP da Polymarket:

- Gamma `GET /events/keyset` para descoberta histórica e live
- Gamma `GET /markets/{id}` para reconciliação/settle
- CLOB `GET /price` para melhor ask do token escolhido
- CLOB `GET /prices-history` para backtest histórico
- SDK Rust `polymarket-client-sdk` para autenticação e envio de ordens

## Estrutura

```text
src/
├── backtest.rs
├── config/
│   ├── env.rs
│   └── mod.rs
├── execution/
│   ├── claim.rs
│   ├── exchange.rs
│   └── mod.rs
├── feed/
│   ├── mod.rs
│   ├── orderbook.rs
│   └── underlying.rs
├── storage/
│   ├── mod.rs
│   └── paper_trades.rs
├── strategy/
│   ├── base.rs
│   ├── mod.rs
│   └── six_am.rs
├── bin/
│   ├── backtest.rs
│   ├── settle_open_trades.rs
│   └── sweep.rs
├── lib.rs
├── main.rs
└── types.rs
```

## Uso

```bash
cp .env.example .env
cargo build
cargo run --bin backtest -- --days 30
cargo run --bin settle_open_trades
cargo run --bin sweep -- --days 7
cargo run --release --bin bot
```

O binario `backtest` agora varre automaticamente as horas `00:00` ate `23:00 UTC` e imprime uma linha por hora.

## Variáveis principais

- `TRADE_DIRECTION=up|down`
- `EXPECTED_WIN_RATE=0.578`
- `MAX_ENTRY_PRICE=0.55`
- `POSITION_SIZE_USDC=5.0`
- `ENTRY_WINDOW_SECS=300`
- `DRY_RUN=true`
- `PAPER_TRADES_PATH=data/paper_trades.json`
- `DATABASE_URL=` reservado para uma próxima camada Postgres/Supabase

## Limitações

- o post não contém direção explícita; isso virou configuração
- o backtest usa `prices-history` do token para estimar a entrada, não o book histórico
- o paper trading atual considera fill imediato no preço sinalizado
- a persistência do template foi implementada localmente em JSON; o gancho para `DATABASE_URL` já existe, mas a camada Postgres ainda não foi conectada
