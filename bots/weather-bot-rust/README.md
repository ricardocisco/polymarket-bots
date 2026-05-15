# Polymarket Weather Bot

Bot em Rust que opera mercados de temperatura diária na Polymarket usando previsões da Open-Meteo API. Monitora automaticamente todas as cidades disponíveis, prevê a temperatura máxima de cada estação e decide BUY YES / BUY NO via critério Quarter-Kelly.

---

## Instalação

```bash
git clone <repositorio>
cd polymarket-weather-bot
cp .env.example .env
cargo build --release
```

---

## Configuração (`.env`)

| Variável                 | Padrão          | Descrição                              |
| ------------------------ | --------------- | -------------------------------------- |
| `POLYMARKET_PRIVATE_KEY` | —               | Chave privada hex (só para bot real)   |
| `MIN_CONFIDENCE`         | `0.98`          | Confiança mínima para entrar (0–1)     |
| `MAX_POSITION_SIZE_USDC` | `10.0`          | Tamanho máximo por posição em USDC     |
| `MIN_ORDER_SIZE_USDC`    | `1.0`           | Tamanho mínimo de ordem em USDC        |
| `RUN_INTERVAL_SECS`      | `3600`          | Segundos entre ciclos do bot           |
| `RUST_LOG`               | `bot=info,warn` | Nível de log (`info`, `debug`, `warn`) |

> Paper trading e backtest não precisam de `POLYMARKET_PRIVATE_KEY`.

---

## Fluxo recomendado

```
1. cargo build --release
2. cargo run --bin setup          # verifica conexão e credenciais
3. cargo run --bin backtest -- --days 60   # valida estratégia historicamente
4. cargo run --release --bin paper -- --watch  # paper trading em tempo real
5. cargo run --release --bin bot  # trading real (só após validar)
```

---

## Binários

### `setup` — Verificação inicial

```bash
cargo run --bin setup
```

Valida chave privada, conexão com o CLOB, credenciais L2 e mercados/previsões por cidade.

---

### `paper` — Paper Trading

Simula entradas com dinheiro fictício em tempo real. Registra decisões em `data/paper_entries.json` e resolve automaticamente quando o mercado fecha.

```bash
cargo run --release --bin paper                        # one-shot
cargo run --release --bin paper -- --watch             # loop (1h)
cargo run --release --bin paper -- --watch --interval 1800  # loop (30min)
cargo run --release --bin paper -- --resolve-only      # só resolve pendentes
cargo run --release --bin paper -- --report            # só exibe relatório
```

| Flag               | Padrão | Descrição                        |
| ------------------ | ------ | -------------------------------- |
| `--watch`          | off    | Loop contínuo                    |
| `--interval`       | `3600` | Segundos entre ciclos            |
| `--min-confidence` | `0.98` | Confiança mínima                 |
| `--max-position`   | `10.0` | Tamanho máximo de posição (USDC) |
| `--analyze-only`   | off    | Só analisa, sem resolver         |
| `--resolve-only`   | off    | Só resolve, sem analisar         |
| `--report`         | off    | Só exibe relatório               |

---

### `backtest` — Backtesting histórico

Testa a estratégia em dados históricos reais (Open-Meteo Archive).

```bash
cargo run --bin backtest                              # últimos 30 dias
cargo run --bin backtest -- --days 60                 # janela maior
cargo run --bin backtest -- --days 30 --min-confidence 0.95
cargo run --release --bin backtest -- --watch         # loop contínuo
```

| Flag               | Padrão  | Descrição                        |
| ------------------ | ------- | -------------------------------- |
| `--days`           | `30`    | Dias para analisar               |
| `--min-confidence` | `0.98`  | Confiança mínima                 |
| `--max-position`   | `10.0`  | Tamanho máximo (USDC)            |
| `--watch`          | off     | Loop contínuo                    |
| `--interval`       | `86400` | Segundos entre ciclos no --watch |

---

### `bot` — Trading real

> ⚠️ Coloca ordens com USDC real. Execute apenas após validar com paper trading e backtest.

```bash
cargo run --release --bin bot
RUST_LOG=info cargo run --release --bin bot   # com logs detalhados
nohup cargo run --release --bin bot >> bot.log 2>&1 &  # em background
```

---

## Como a estratégia funciona

**Descoberta:** Gamma API busca todos os mercados ativos de temperatura → extrai ICAO do link Wunderground → geocoding para coordenadas da estação.

**Previsão:** Open-Meteo retorna `temperature_2m_max` para 3 dias. A confiança é estimada pelo coeficiente de variação:

```
cv   = desvio_padrão / média
conf = clamp(0.99 - cv × 1.5, 0.50, 0.99)
```

**Decisão e sizing:**

```
eff_conf = confiança_base + margin_bonus   (margin_bonus ∈ [-0.10, +0.04])

Se eff_conf < MIN_CONFIDENCE → Skip
Se previsão dentro do range  → BUY YES
Se previsão fora do range    → BUY NO

Quarter-Kelly:
  b    = (1 / preço) - 1
  k    = (p × b - q) / b
  size = (k / 4) × MAX_POSITION
```

---

## Estrutura

```
src/
├── main.rs       # bot — loop de trading real
├── config.rs     # leitura do .env
├── markets.rs    # Gamma API + extração ICAO + geocoding
├── weather.rs    # Open-Meteo (previsão + histórico)
├── strategy.rs   # Quarter-Kelly + lógica de decisão
├── types.rs      # tipos compartilhados
├── cities.rs     # lista de cidades (fallback backtest)
└── bin/
    ├── setup.rs
    ├── backtest.rs
    └── paper.rs
```

---

## Segurança

- Nunca adicione `.env` ao git (já está no `.gitignore`)
- Use wallet dedicada com limite de USDC pré-definido
- A chave privada é usada apenas para assinar transações EIP-712 localmente

Developed by Chard
