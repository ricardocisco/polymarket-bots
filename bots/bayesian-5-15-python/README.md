## Backtest mode

This bot now has a historical backtest CLI that reuses the same `OptimizedBayesianModel` and `KellyCriterion` used by live mode. It reads public Gamma/CLOB/Binance data only and never sends orders.

```bash
python backtest.py --days 3
python backtest.py --days 7 --mode AGGRESSIVE_OPTIMIZED --asset BTC --interval 5 --trades
python backtest.py --days 14 --bankroll 20 --stake 1
```

The backtest filters closed BTC/ETH/SOL/XRP 5m/15m markets, replays Binance 1m candles, checks historical CLOB entry prices against `ORDER_BOOK_PARAMS`, and prints win rate, PnL, ROI, and skip reasons.

---

# Polymarket Bayes+Kelly Trading Bot

Bot de trading automatizado para mercados de previsão cripto na [Polymarket](https://polymarket.com).  
Opera nos mercados **BTC/ETH/SOL/XRP Up-or-Down** (5min e 15min) usando modelo Bayesiano + Kelly Criterion para dimensionamento de posição.

---

## Índice

1. [Pré-requisitos](#pré-requisitos)
2. [Instalação](#instalação)
3. [Configuração do .env](#configuração-do-env)
4. [Como executar](#como-executar)
5. [Modos de operação](#modos-de-operação)
6. [Como funciona a análise](#como-funciona-a-análise)
7. [Modelo Bayesiano](#modelo-bayesiano)
8. [Kelly Criterion](#kelly-criterion)
9. [Gerenciamento de risco](#gerenciamento-de-risco)
10. [Arquivos de log](#arquivos-de-log)

---

## Pré-requisitos

- Python 3.11+
- Conta na Polymarket com saldo em USDC (rede Polygon)
- Chave privada da carteira Polygon que possui o saldo

---

## Instalação

```bash
# Clone o repositório
git clone <repo-url>
cd polymarket_trading

# Crie e ative o ambiente virtual
python -m venv venv

# Windows
venv\Scripts\activate

# Linux/macOS
source venv/bin/activate

# Instale as dependências
pip install -r requirements.txt
```

---

## Configuração do .env

Copie o arquivo de exemplo e preencha com suas credenciais:

```bash
cp .env.example .env
```

Conteúdo do `.env.example`:

```env
POLYMARKET_PRIVATE_KEY=
POLYMARKET_FUNDER=
POLYMARKET_SIGNATURE_TYPE=

# OPCIONAL - Credenciais de API (se você tiver)
POLYMARKET_API_KEY=
POLYMARKET_API_SECRET=
POLYMARKET_API_PASSPHRASE=
```

### Explicação de cada variável

| Variável                    | Obrigatória | Descrição                                                                                                                                                                                 |
| --------------------------- | ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POLYMARKET_PRIVATE_KEY`    | ✅ Sim      | Chave privada da carteira Polygon (começa com `0x`). Usada para assinar ordens (auth L1).                                                                                                 |
| `POLYMARKET_FUNDER`         | ✅ Sim      | Endereço público da carteira que aparece no seu perfil Polymarket (ex: `0xd604...`).                                                                                                      |
| `POLYMARKET_SIGNATURE_TYPE` | ✅ Sim      | Tipo de assinatura: `0` = EOA (MetaMask sem conta no site), `1` = POLY_PROXY (conta via Google/Magic Link), `2` = GNOSIS_SAFE (proxy de contas novas). Se criou conta pelo site, use `1`. |
| `POLYMARKET_API_KEY`        | ⚠️ Opcional | Chave de API L2 para autenticação HMAC. Se não tiver, o bot deriva credenciais via L1.                                                                                                    |
| `POLYMARKET_API_SECRET`     | ⚠️ Opcional | Secret da API L2.                                                                                                                                                                         |
| `POLYMARKET_API_PASSPHRASE` | ⚠️ Opcional | Passphrase da API L2.                                                                                                                                                                     |

### Como obter suas credenciais

1. Acesse [polymarket.com](https://polymarket.com) e faça login
2. Em **Profile → Settings → Export Private Key** — este é o `POLYMARKET_PRIVATE_KEY`
3. O endereço exibido no seu perfil é o `POLYMARKET_FUNDER`
4. Se criou conta pelo site (Google/email), use `POLYMARKET_SIGNATURE_TYPE=1`

> ⚠️ **Nunca compartilhe sua chave privada.** O `.env` já está no `.gitignore`.

---

## Como executar

Sempre ative o venv antes:

```bash
# Windows
venv\Scripts\activate

python main.py
```

Ao iniciar, o bot vai:

1. Pedir para selecionar o **modo de operação**
2. Pedir o valor da sua **banca em USD** — sem valor padrão, você define
3. Buscar automaticamente os mercados ativos de BTC/ETH/SOL/XRP
4. Conectar nos WebSockets da Binance e Polymarket
5. Monitorar e executar trades quando todas as condições forem atendidas

```
============================================================
  🎯 SELECIONE O MODO DE OPERAÇÃO
============================================================

  [1] Conservador 🛡️
      Baixo risco, alta seletividade

  [2] Agressivo ⚔️
      Risco moderado, retorno balanceado

  [3] Agressivo Otimizado 🎯
      Baseado em análise de 575 trades - Filtros inteligentes

  [4] Degen 🚀💎
      Alto risco, máximo retorno (YOLO mode)

============================================================
  💰 QUAL É A SUA BANCA? (BANKROLL)
============================================================

  Bankroll em USD: $38
  ✅ Banca: $38.00
```

---

## Modos de operação

O bot tem 4 perfis pré-configurados. Ao selecionar um modo, todos os parâmetros do modelo Bayesiano, Kelly e gerenciamento de risco são substituídos automaticamente.

### 1. Conservador 🛡️

Ideal para quem está começando ou quer preservar capital.

| Parâmetro               | Valor                          |
| ----------------------- | ------------------------------ |
| Kelly Fraction          | 20%                            |
| Posição máxima          | $5,00                          |
| Posição mínima          | $2,50                          |
| Confiança mínima        | 60%                            |
| Edge mínimo             | 5%                             |
| Posições simultâneas    | 2                              |
| Stop por drawdown       | 15%                            |
| Cooldown após perda     | 60 min                         |
| Losses consecutivos máx | 2                              |
| Log                     | `trades_log_conservative.json` |

### 2. Agressivo ⚔️

Equilíbrio entre frequência de trades e controle de risco.

| Parâmetro               | Valor                        |
| ----------------------- | ---------------------------- |
| Kelly Fraction          | 35%                          |
| Posição máxima          | $5,00                        |
| Posição mínima          | $2,50                        |
| Confiança mínima        | 62%                          |
| Edge mínimo             | 5%                           |
| Posições simultâneas    | 2                            |
| Stop por drawdown       | 25%                          |
| Cooldown após perda     | 30 min                       |
| Losses consecutivos máx | 3                            |
| Log                     | `trades_log_aggressive.json` |

### 3. Agressivo Otimizado 🎯

Calibrado com análise de **575 trades reais**. Inclui filtros extras baseados em dados históricos.

| Parâmetro            | Valor                                                                   |
| -------------------- | ----------------------------------------------------------------------- |
| Kelly Fraction       | 35%                                                                     |
| Posição máxima       | $5,00                                                                   |
| Posição mínima       | $2,50                                                                   |
| Confiança mínima     | 54%                                                                     |
| Edge mínimo          | 5%                                                                      |
| Posições simultâneas | 2                                                                       |
| Stop por drawdown    | 20%                                                                     |
| Cooldown após perda  | 45 min                                                                  |
| Somente UP trades    | ✅ (DOWN tem 42,9% WR vs 53,3% UP no histórico)                         |
| Horários bloqueados  | 2h, 3h, 5h, 12h, 18h, 19h, 22h (WR < 45% nesses horários)               |
| Volume máximo        | 1,5× volume médio (volume muito alto correlaciona com pior performance) |
| Log                  | `trades_log_aggressive_optimized.json`                                  |

### 4. Degen 🚀💎

Alto risco, máximo retorno. Não recomendado como operação principal.

| Parâmetro               | Valor                   |
| ----------------------- | ----------------------- |
| Kelly Fraction          | 50%                     |
| Posição máxima          | $15,00 (~40% da banca)  |
| Posição mínima          | $2,50                   |
| Confiança mínima        | 51%                     |
| Edge mínimo             | 1%                      |
| Posições simultâneas    | 6                       |
| Stop por drawdown       | 40%                     |
| Cooldown após perda     | 15 min                  |
| Losses consecutivos máx | 5                       |
| Log                     | `trades_log_degen.json` |

---

## Como funciona a análise

A cada vela de 1 minuto recebida da Binance, o bot executa o seguinte pipeline:

```
Kline Binance (1min)
        │
        ▼
┌─────────────────────┐
│  Coleta 60 candles  │  RSI, EMA, Volume, ATR
│  de histórico       │
└────────┬────────────┘
         │
         ▼
┌──────────────────────────────┐
│   Modelo Bayesiano           │
│   → p_up, p_down             │
│   → confidence, edge         │
└────────┬─────────────────────┘
         │
         ▼
┌──────────────────────────────┐
│   Filtros de entrada         │
│   • Confiança mínima         │
│   • Edge mínimo              │
│   • Order book (50c–58c)     │
│   • Liquidez mínima ($5)     │
│   • Expira em > 2 min        │
│   • Posições abertas < máx   │
│   • Não está em cooldown     │
└────────┬─────────────────────┘
         │
         ▼
┌──────────────────────────────┐
│   Kelly Criterion            │
│   → tamanho da posição       │
│   → ajustes dinâmicos        │
└────────┬─────────────────────┘
         │
         ▼
┌──────────────────────────────┐
│   Execução via CLOB          │
│   Polymarket (ordem GTC)     │
└──────────────────────────────┘
```

**O bot só entra se TODOS os critérios passarem:**

1. Confiança do modelo ≥ limiar do modo selecionado
2. Edge real ≥ edge mínimo do modo selecionado
3. Best ask do token entre **$0,50** e **$0,58** no order book da Polymarket
4. Liquidez ≥ $5 no ask
5. Mercado expira em mais de 2 minutos
6. Posições abertas < máximo configurado
7. Não está em cooldown por perda recente

---

## Modelo Bayesiano

O modelo usa **atualização Bayesiana sequencial** para combinar 4 sinais técnicos independentes e estimar a probabilidade de o preço subir ou descer antes do vencimento do mercado.

### Sinais utilizados

| Sinal                  | Peso base | Como é calculado                                                                                       |
| ---------------------- | --------- | ------------------------------------------------------------------------------------------------------ |
| **Momentum (RSI)**     | 40%       | RSI 14 períodos. RSI < 32 → forte sinal UP. RSI > 68 → forte sinal DOWN. Zona neutra 42–58: sem sinal. |
| **Trend (EMA)**        | 40%       | EMA5 vs EMA15. Gap > 0,10% para cima → UP. Gap > 0,10% para baixo → DOWN.                              |
| **Volume**             | 10%       | Volume atual vs média. Volume > 1,5× médio amplifica o sinal da direção do preço.                      |
| **Volatilidade (ATR)** | 10%       | ATR relativo ao preço. Alta volatilidade reduz a confiança geral.                                      |

### Prior calibrado

O prior (probabilidade inicial antes de ver os dados) reflete o comportamento histórico real:

```python
# Modo Agressivo Otimizado — baseado em 575 trades reais
prior_up   = 0.62
prior_down = 0.38
```

Modos conservadores usam `prior_up = 0.45` para exigir mais evidência antes de entrar.

### Atualização sequencial

Para cada sinal, o modelo aplica o Teorema de Bayes:

```
P(UP | sinal) = P(sinal | UP) × P(UP)  /  P(sinal)
```

Os 4 sinais são aplicados em sequência, atualizando a distribuição a cada passo. O resultado final é:

- `p_up` — probabilidade estimada de o preço subir
- `p_down` — probabilidade estimada de o preço descer
- `confidence` — confiança geral (média ponderada das forças dos sinais)
- `edge` = `|p_up - p_down|` — vantagem real estimada sobre o mercado

### Penalidade do strike

O modelo penaliza entradas quando o preço atual está muito próximo do strike (o nível que determina UP/DOWN na resolução):

- Distância < 0,5% do strike → penalidade alta (WR histórico de 37,8% nessa faixa)
- Distância ≥ 0,5% → sem penalidade adicional

### Aprendizado contínuo

Ao iniciar, o modelo carrega o arquivo de log do modo selecionado e analisa os trades resolvidos para **recalibrar os pesos dos sinais automaticamente**:

- Sinal com acurácia < 48% → peso mínimo (0,05) — pior que aleatório
- Sinal com acurácia > 55% → peso × 1,5 — reforça o que funciona
- Combina 60% acurácia direcional + 40% taxa de lucratividade no score final

Se não houver histórico (primeira execução), usa os pesos padrão do modo selecionado.

---

## Kelly Criterion

O Kelly Criterion calcula o tamanho ótimo de posição para maximizar o crescimento do capital no longo prazo, dado o edge estimado pelo modelo Bayesiano.

### Fórmula base

```
f* = (p × b - q) / b
```

Onde:

- `f*` = fração do bankroll a apostar
- `p` = probabilidade estimada de ganho (modelo Bayesiano)
- `q = 1 - p` = probabilidade de perda
- `b` = odds de retorno = (1 / preço do token) - 1

### Kelly fracionário

O bot nunca usa o Kelly completo. Aplica um multiplicador conservador:

```
Posição = bankroll × f* × kelly_fraction
```

Com `kelly_fraction = 0.35` (modo Agressivo), usa apenas **35% do Kelly ótimo**.

### Ajustes dinâmicos em tempo real

O multiplicador do Kelly é ajustado conforme o desempenho recente:

| Condição               | Ajuste                                                        |
| ---------------------- | ------------------------------------------------------------- |
| 2+ losses consecutivos | Kelly × 0,5 — reduz à metade                                  |
| 3+ wins consecutivos   | Kelly × 1,1 — leve aumento, máximo de 30% do Kelly base       |
| Drawdown > 10%         | Kelly × fator proporcional ao drawdown (mínimo 30% do normal) |

### Hard caps — limites rígidos

Independente do resultado do cálculo Kelly, a posição nunca ultrapassa:

1. `max_position_size` — ex: $5,00
2. `bankroll × max_bankroll_per_trade` — ex: 13% de $38 = $4,94

O limite mais restritivo é sempre o **último** a ser aplicado — nenhum bônus pode aumentar a posição depois dele.

Se o Kelly calcular menos que `min_position_size` ($2,50), o bot usa o mínimo como piso (a Polymarket exige no mínimo 5 shares × $0,50 por ordem).

### Exemplo completo com $38 de banca

```
Bankroll: $38
Best ask: $0.50
Modelo: p_up = 0.576
Edge: 0.576 - 0.500 = 0.076

Odds:          (1 / 0.50) - 1 = 1.0
Kelly full:    (0.576 × 1.0 - 0.424) / 1.0 = 0.152
Kelly frac:    0.152 × 0.35 = 0.053

Posição bruta: $38 × 0.053 = $2.01
Abaixo do mínimo ($2.50) → usar piso: $2.50
Hard cap 13%:  $38 × 0.13 = $4.94 → não aplica

✅ Posição final: $2.50
```

---

## Gerenciamento de risco

### Proteções ativas antes de entrar

| Proteção                 | Como funciona                                                                                                                                               |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Cooldown**             | Após perda, aguarda X minutos antes do próximo trade                                                                                                        |
| **Losses consecutivos**  | Pausa o bot após N perdas seguidas                                                                                                                          |
| **Drawdown máximo**      | Para de operar se a banca cair mais de X%                                                                                                                   |
| **Posições simultâneas** | Limita exposição total para nunca colocar toda a banca em risco ao mesmo tempo                                                                              |
| **Race condition**       | `asyncio.Lock` garante que dois sinais simultâneos (ex: BTC e ETH chegando no mesmo milissegundo) não abram ambas as posições antes de registrar a primeira |

### Faixa de preço do order book

O bot só entra se o best ask do token estiver entre **$0,50** e **$0,58**:

- Abaixo de $0,50: mercado precifica o evento como muito improvável — sem edge real
- Acima de $0,58: pagar mais de $0,58 para ganhar $1,00 requer WR > 58%, nível que o modelo não garante

---

## Arquivos de log

Cada modo grava em um arquivo separado para análise independente:

| Modo                | Arquivo                                |
| ------------------- | -------------------------------------- |
| Conservador         | `trades_log_conservative.json`         |
| Agressivo           | `trades_log_aggressive.json`           |
| Agressivo Otimizado | `trades_log_aggressive_optimized.json` |
| Degen               | `trades_log_degen.json`                |

Cada entrada de trade contém: mercado, direção, preço de entrada, tamanho da posição, resultado da resolução, PnL, sinais Bayesianos completos e parâmetros Kelly usados naquele momento.

Para analisar o histórico:

```bash
python analyze_trades.py
python view_trades.py
```

---

> ⚠️ **DISCLAIMER**: Este bot é para fins educacionais. Trading envolve risco de perda de capital. Use por sua conta e risco.
