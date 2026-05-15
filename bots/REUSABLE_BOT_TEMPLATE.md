# Template Reutilizavel Para Bots De Trading

Este documento serve como blueprint para criar outros bots com a mesma ideia-base:

- backtest offline
- execucao em `dry_run` com paper trading
- settle automatico no fechamento
- persistencia em Postgres ou Supabase
- utilitario de reconciliacao para trades que ficaram abertos

O objetivo e separar claramente:

1. estrategia
2. runtime
3. execucao
4. persistencia
5. backtest
6. settle

## Principios

- A estrategia nao deve conhecer banco, API externa ou detalhes do executor.
- O runtime orquestra feed, estrategia, execucao e persistencia.
- O backtest deve rodar sem depender do runtime live.
- O banco deve guardar trades `OPEN` e depois atualiza-los para `SETTLED`.
- Sempre exista um binario de reparo para liquidar trades pendentes se o processo cair.

## Estrutura Sugerida

```text
src/
  main.rs
  lib.rs
  backtest.rs
  types.rs
  config/
    mod.rs
    env.rs
    markets.rs
  strategy/
    mod.rs
    base.rs
    my_strategy.rs
  feed/
    mod.rs
    underlying.rs
    orderbook.rs
  execution/
    mod.rs
    exchange.rs
    claim.rs
  storage/
    mod.rs
    paper_trades.rs
  bin/
    backtest.rs
    settle_open_trades.rs
    sweep.rs
```

## Contratos Principais

### 1. StrategyInput

A estrategia deve receber um input pequeno e puro:

```rust
pub struct StrategyInput {
    pub quote: QuoteSnapshot,
    pub underlying_price_usd: f64,
    pub strike_price_usd: f64,
    pub now_ts: i64,
    pub close_ts: i64,
    pub seconds_to_close: f64,
}
```

### 2. SignalDecision

A estrategia deve devolver uma decisao serializavel e auditavel:

```rust
pub struct SignalDecision {
    pub action: Action,
    pub side: Option<Side>,
    pub size: u32,
    pub reason: &'static str,
    pub order_type: Option<OrderTimeInForce>,
    pub limit_price_cents: Option<f64>,
    pub confidence: Option<f64>,
    pub diagnostics: Option<StrategyDiagnostics>,
}
```

### 3. Trait de estrategia

```rust
pub trait Strategy: Send {
    fn name(&self) -> &'static str;
    fn decide(&mut self, input: &StrategyInput) -> SignalDecision;

    fn on_order_submitted(&mut self, _update: &ExecutionUpdate, _quote: &QuoteSnapshot) {}
}
```

## Fluxo Recomendado Do Runtime

O `main.rs` deve fazer:

1. carregar `.env`
2. decidir `dry_run` e `allow_live_trading`
3. conectar no Postgres ou Supabase se `DATABASE_URL` existir
4. iniciar feed do underlying
5. descobrir mercado ativo e token ids
6. iniciar feed de orderbook
7. instanciar estrategia
8. a cada update:
   - montar `StrategyInput`
   - chamar `strategy.decide(...)`
   - se `Hold`, nao faz nada
   - se `Buy` ou `Sell`, enviar para executor ou paper flow
9. no fechamento:
   - calcular resultado
   - liquidar no banco
   - gravar resultado local

## Dois Dry Runs Diferentes

Mantenha duas ideias separadas:

### A. Paper runtime

Usado para simular o ciclo completo:

- gerar sinal
- registrar trade `OPEN`
- aguardar fill simulado
- fazer settle ao final

Esse modo e o mais util para validacao operacional.

### B. Executor dry run

Usado apenas para nao mandar ordem real, mas retornando sucesso fake no executor.

Esse modo e util para testes isolados do executor, mas nao substitui um fluxo completo de paper trading.

Recomendacao:

- use `DRY_RUN=true` para o fluxo paper completo
- deixe o `executor.dry_run` como detalhe interno do modulo de execucao

## Estado Minimo Do Runtime

Cada mercado deve manter algo parecido com isso:

```rust
struct MarketRuntimeState {
    last_underlying_price_usd: Option<f64>,
    pending_entry: Option<PendingEntry>,
    open_trade: Option<TrackedTrade>,
}
```

### PendingEntry

Representa uma ordem emitida no paper trading que ainda nao foi preenchida.

Campos recomendados:

- `side`
- `size`
- `limit_price_cents`
- `submitted_at_ms`
- informacoes de diagnostico da estrategia

### TrackedTrade

Representa uma posicao aberta, pronta para ser liquidada no fechamento.

Campos recomendados:

- `side`
- `size`
- `entry_price_cents`
- `submitted_at_ms`

## Fluxo Recomendado De Paper Trading

Quando a estrategia retornar `Buy`:

1. salvar `pending_entry`
2. inserir trade `OPEN` no banco
3. esperar o orderbook tocar o preco limite
4. converter `pending_entry` em `open_trade`
5. no fechamento, calcular `winner_side`, `final_price_usd` e `pnl`
6. atualizar trade para `SETTLED`

Pseudo-fluxo:

```rust
if dry_run {
    state.pending_entry = Some(...);
    store.insert_open_trade(...).await?;
    return;
}
```

Depois, em cada update:

```rust
if let Some(pending) = state.pending_entry {
    if best_buy <= pending.limit_price_cents {
        state.open_trade = Some(...);
        state.pending_entry = None;
    }
}
```

No fechamento:

```rust
store.settle_open_trades(...).await?;
```

## Persistencia No Banco

Crie uma tabela unica de trades com status:

- `OPEN`
- `SETTLED`

Schema base sugerido:

```sql
CREATE TABLE IF NOT EXISTS paper_trades (
  id BIGSERIAL PRIMARY KEY,
  strategy_name TEXT NOT NULL,
  strategy_version TEXT NOT NULL,
  market_key TEXT NOT NULL,
  market_ticker TEXT NOT NULL,
  strike_price_usd DOUBLE PRECISION NOT NULL,
  close_ts BIGINT NOT NULL,
  submitted_at BIGINT NOT NULL,
  side TEXT NOT NULL,
  size INTEGER NOT NULL DEFAULT 1,
  entry_price_cents INTEGER NOT NULL,
  underlying_price_usd DOUBLE PRECISION NOT NULL,
  seconds_to_close DOUBLE PRECISION NOT NULL,
  decision_reason TEXT NOT NULL,
  diagnostics JSONB,
  status TEXT NOT NULL DEFAULT 'OPEN',
  winner_side TEXT,
  final_price_usd DOUBLE PRECISION,
  pnl_cents INTEGER,
  settled_at BIGINT,
  created_at BIGINT NOT NULL
);
```

Indices recomendados:

```sql
CREATE INDEX IF NOT EXISTS idx_paper_trades_market
  ON paper_trades(market_key, market_ticker, strategy_name, strategy_version);

CREATE INDEX IF NOT EXISTS idx_paper_trades_status
  ON paper_trades(status, close_ts);
```

## API Minima Da Camada Storage

Sua camada `storage/paper_trades.rs` deve expor pelo menos:

```rust
pub async fn connect(database_url: &str) -> Result<Self>;

pub async fn insert_open_trade(
    &self,
    market_key: &str,
    market_ticker: &str,
    strike_price_usd: f64,
    close_ts: i64,
    submitted_at: i64,
    side: Side,
    entry_price_cents: u32,
    underlying_price_usd: f64,
    seconds_to_close: f64,
    decision: &SignalDecision,
    dry_run: bool,
) -> Result<()>;

pub async fn fetch_open_groups(&self) -> Result<Vec<OpenTradeGroup>>;

pub async fn settle_open_trades(
    &self,
    market_key: &str,
    market_ticker: &str,
    strike_price_usd: f64,
    final_price_usd: f64,
    settled_at: i64,
) -> Result<u64>;
```

## Diagnostics JSONB

Sempre salve `diagnostics` em JSONB.

Guarde:

- `executionMode`
- `size`
- `confidence`
- `orderType`
- dados especificos da estrategia

Isso permite:

- auditoria
- dashboards
- comparacao entre versoes de estrategia
- filtros no Supabase

## Supabase / Postgres

Se usar Supabase via pooler, trate `DATABASE_URL` para garantir:

- `sslmode=require`
- `channel_binding=disable`

Checklist:

- aceitar `postgres://` e `postgresql://`
- ignorar query params nao suportados por `tokio-postgres`
- logar os params descartados

## Settle Automatico No Runtime

No fechamento do mercado:

1. obter o ultimo preco observado do underlying
2. calcular resultado:
   - `yes` se `final_price_usd > strike_price_usd`
   - `no` se `final_price_usd < strike_price_usd`
   - `draw` caso igual
3. atualizar todos os trades `OPEN` daquele mercado
4. gravar um log local de resultado

Formula recomendada para binario:

```text
win  => (100 - entry_price_cents) * size
draw => (50 - entry_price_cents) * size
loss => -entry_price_cents * size
```

## Binario De Reconciliacao

Sempre tenha um binario como `settle_open_trades`.

Funcao dele:

1. buscar grupos `OPEN` no banco
2. ignorar mercados que ainda nao fecharam
3. consultar uma fonte historica do underlying no timestamp de fechamento
4. chamar `settle_open_trades(...)`

Casos de uso:

- processo caiu antes do settle
- restart do VPS
- falha temporaria no feed
- conciliacao manual

## Backtest Reutilizavel

Separe em dois niveis:

### Engine de backtest

Fica em `src/backtest.rs` e recebe candles normalizados.

Responsabilidades:

- carregar candles
- agrupar por janela da estrategia
- reproduzir regra de entrada
- calcular win rate e PnL
- devolver `trades_detail`

### CLI do backtest

Fica em `src/bin/backtest.rs` e so:

- parseia argumentos
- chama a engine
- imprime resumo

## Regra Para Evitar Divergencia Entre Live E Backtest

O ideal e extrair um nucleo puro da estrategia para compartilhar:

- mesma resolucao de banda
- mesma definicao de side
- mesma regra de preco teto
- mesma logica de sizing

Se nao fizer isso, o risco e:

- backtest mostrar edge
- runtime operar com regra diferente

## Sweep De Parametros

Tenha um binario `sweep.rs` para testar:

- bandas
- thresholds de entry
- sizing
- filtros por tempo

Isso deve reutilizar a mesma engine do backtest, nunca codigo duplicado.

## Logs Locais Recomendados

Mesmo com banco, mantenha CSVs locais para inspeção rapida:

- `logs/paper_trades.csv`
- `logs/paper_trade_results.csv`

Use-os como apoio, nao como fonte oficial de verdade.

## Variaveis De Ambiente Minimas

```env
PRIVATE_KEY=
PROXY_WALLET=
CLOB_URL=https://clob.polymarket.com
CHAIN_ID=137
POLYGON_RPC_URL=

DRY_RUN=true
ALLOW_LIVE_TRADING=false
DATABASE_URL=
```

Se quiser paper trading persistido:

```env
DRY_RUN=true
DATABASE_URL=postgresql://...
```

Se quiser live trading com persistencia:

```env
DRY_RUN=false
ALLOW_LIVE_TRADING=true
DATABASE_URL=postgresql://...
```

## Checklist Para Criar Um Novo Bot

1. Criar `StrategyInput`, `SignalDecision` e `StrategyDiagnostics`.
2. Implementar a estrategia em modulo proprio.
3. Criar config tipada em `config/env.rs`.
4. Subir feed do underlying e feed de orderbook.
5. Implementar `MarketRuntimeState`.
6. Integrar `PaperTradeStore`.
7. Inserir trades `OPEN` quando houver sinal.
8. Fazer fill simulado em `dry_run`.
9. Fazer settle automatico no fechamento.
10. Criar binario `settle_open_trades`.
11. Criar `src/backtest.rs`.
12. Criar `src/bin/backtest.rs`.
13. Criar `src/bin/sweep.rs` se a estrategia precisar de tuning.

## O Que Deve Ser Generico

- tipos base
- trait da estrategia
- camada storage
- reconciliador de settle
- estrutura do backtest
- logs locais

## O Que Deve Ser Especifico De Cada Bot

- regra da estrategia
- fonte do underlying
- regra de entrada
- regra de sizing
- criterios de fill simulado
- logica de saida, se nao for hold-to-expiry

## Recomendacao Final

Se voce for reaproveitar isso em varios bots, vale extrair um crate interno comum, por exemplo:

```text
crates/
  bot-core/
    strategy/
    storage/
    types/
    runtime_helpers/
```

Assim cada bot implementa apenas:

- sua estrategia
- seus mercados
- seus thresholds
- seu backtest especifico

e reaproveita todo o resto.
