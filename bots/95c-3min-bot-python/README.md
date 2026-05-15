# Polymarket Sniper Bot 🎯

Bot automatizado para operar em mercados da Polymarket em modo "sniper". Monitora BTC, ETH e XRP e busca oportunidades de fim de janela com alta probabilidade de sucesso (preço > $0.95).

---

## 📋 Índice

- [Características](#-características)
- [Modos de Operação](#-modos-de-operação)
- [Instalação](#-instalação)
- [Configuração](#-configuração)
- [Uso](#-uso)
- [Estratégia](#-estratégia)
- [FAQ](#-faq)

---

## ✨ Características

- ✅ **3 Modos de Operação**: Polling, WebSocket ou Híbrido
- ✅ **Múltiplos Ativos**: BTC, ETH, XRP
- ✅ **Múltiplas Janelas**: 5m e 15m
- ✅ **Análise Técnica**: RSI, MACD, Momentum
- ✅ **Gestão de Risco**: Kelly Criterion adaptado
- ✅ **Tempo Real**: WebSocket Polymarket + Binance (opcional)
- ✅ **Dry-run**: Teste sem arriscar capital

---

## 🚀 Modos de Operação

O bot oferece 3 modos diferentes - escolha o que melhor se adapta às suas necessidades:

### 1️⃣ **Modo POLLING** (`main.py`) - Recomendado para Iniciantes

**Quando usar:**

- Você quer algo simples e confiável
- Não precisa de reação instantânea (trades a cada 20-30s está OK)
- Primeira vez usando o bot

**Características:**

- ✓ Mais simples
- ✓ Sem dependências extras
- ✓ Funciona offline (sem conexão persistente)
- ✗ Delay de até 30s para detectar oportunidades

**Como usar:**

```bash
python main.py            # modo live
python main.py --dry-run  # simulação
```

---

### 2️⃣ **Modo WEBSOCKET** (`hybrid_runner.py --mode websocket`) - Recomendado para Máxima Performance

**Quando usar:**

- Você quer ZERO delay - reação instantânea
- Quer entrar EXATAMENTE quando preço bate $0.95
- Busca maximizar oportunidades

**Características:**

- ✓ Reação INSTANTÂNEA (< 1 segundo)
- ✓ WebSocket Polymarket (preços em tempo real)
- ✓ WebSocket Binance (opcional - preços cripto)
- ✓ Detecta oportunidades que o polling perde
- ✗ Requer conexão persistente
- ✗ Dependência extra: `websocket-client`

**Como usar:**

```bash
# Instale a dependência extra (apenas primeira vez)
pip install websocket-client

# Execute em modo WebSocket
python hybrid_runner.py --mode websocket
python hybrid_runner.py --mode websocket --dry-run  # simulação
```

---

### 3️⃣ **Modo HÍBRIDO** (`hybrid_runner.py --mode polling`) - Melhor Custo-Benefício

**Quando usar:**

- Quer equilíbrio entre simplicidade e performance
- Polling rápido (5s) é suficiente
- Quer opção de mudar para WebSocket depois

**Características:**

- ✓ Polling rápido (5s vs 30s do main.py)
- ✓ Código preparado para WebSocket
- ✓ Fácil migração entre modos
- ✗ Ainda tem delay (menor que main.py)

**Como usar:**

```bash
python hybrid_runner.py --mode polling  # padrão
python hybrid_runner.py --dry-run       # simulação
```

---

## 📦 Instalação

### 1. Clone o repositório

```bash
git clone <seu-repo>
cd polymarket-farm
```

### 2. Crie ambiente virtual

```bash
python -m venv venv
```

### 3. Ative o ambiente virtual

**Windows:**

```bash
venv\Scripts\activate
```

**Linux/Mac:**

```bash
source venv/bin/activate
```

### 4. Instale dependências

**Modo Polling (básico):**

```bash
pip install -r requirements.txt
```

**Modo WebSocket (completo):**

```bash
pip install -r requirements.txt
# websocket-client já incluído no requirements.txt
```

---

## ⚙️ Configuração

### 1. Crie o arquivo `.env`

Crie um arquivo `.env` na raiz do projeto com:

```bash
# ══════════════════════════════════════════════════════════════
# OBRIGATÓRIO - Credenciais Polymarket
# ══════════════════════════════════════════════════════════════

PRIVATE_KEY="sua_private_key_aqui"
FUNDER_ADDRESS="0xSeuEnderecoPolymarket"
SIGNATURE_TYPE=2  # 0=EOA, 1=Magic/Email, 2=Proxy/Browser Wallet

# API Keys da Polymarket (obtenha em https://polymarket.com/)
POLYMARKET_API_KEY="sua_api_key"
POLYMARKET_API_SECRET="seu_api_secret"
POLYMARKET_API_PASSPHRASE="seu_passphrase"

# ══════════════════════════════════════════════════════════════
# CONFIGURAÇÕES DO BOT
# ══════════════════════════════════════════════════════════════

BANKROLL=20.0        # Capital disponível (USD)
LOOP_INTERVAL=20     # Polling interval (segundos) - apenas main.py
POLL_INTERVAL=5      # Polling interval (segundos) - hybrid_runner.py
MIN_EDGE=0.04        # Edge mínima para trade (4%)

# ══════════════════════════════════════════════════════════════
# OPCIONAL - WebSocket Binance (para modo WebSocket)
# ══════════════════════════════════════════════════════════════

BINANCE_API_KEY=""      # Opcional - melhora performance do WebSocket
BINANCE_API_SECRET=""   # Obtenha em https://www.binance.com/

# ══════════════════════════════════════════════════════════════
# MODO DE OPERAÇÃO (hybrid_runner.py)
# ══════════════════════════════════════════════════════════════

USE_WEBSOCKET=false  # true para WebSocket, false para Polling
```

### 2. Obtenha suas credenciais

#### Polymarket:

1. Acesse [Polymarket](https://polymarket.com/)
2. Conecte sua carteira
3. Vá em Settings → API Keys
4. Crie uma nova API Key
5. Copie: API Key, Secret e Passphrase para o `.env`

#### Binance (Opcional - apenas para WebSocket):

1. Acesse [Binance](https://www.binance.com/)
2. Account → API Management
3. Create API
4. **NÃO precisa de permissões de trading** - apenas leitura
5. Copie API Key e Secret para o `.env`

---

## 🎮 Uso

### Modo Polling (Simples)

```bash
# Teste primeiro com dry-run
python main.py --dry-run

# Quando estiver confiante, execute live
python main.py
```

### Modo WebSocket (Avançado)

```bash
# Teste primeiro com dry-run
python hybrid_runner.py --mode websocket --dry-run

# Execute live
python hybrid_runner.py --mode websocket
```

### Modo Híbrido (Polling Rápido)

```bash
# Polling rápido (5s)
python hybrid_runner.py --mode polling --dry-run
python hybrid_runner.py --mode polling
```

### Dicas:

- ✅ **Sempre teste com --dry-run primeiro**
- ✅ Comece com capital baixo (BANKROLL=10 ou 20)
- ✅ Monitore logs em `logs/trades.json`
- ✅ Use Ctrl+C para parar o bot com segurança

---

## 🎯 Estratégia

### Estratégia "Sniper"

O bot opera em modo "sniper" - entra apenas quando:

1. **Tempo restante:** Entre 0.5 e 3 minutos para fechar
2. **Preço mínimo:** >= $0.95 (vitória quase certa)
3. **Validação técnica:** RSI, MACD e momentum confirmam
4. **Strike price:** Preço atual está do lado certo do strike

### Gestão de Risco

- **Aposta fixa:** $2.00 por trade (ajustável)
- **Exposição máxima:** Controlado pelo BANKROLL
- **Stop automático:** Não opera se condições não forem ideais

### Exemplo de Trade

```
📌 BTC 15M | strike=$67,029.48 | UP=0.973 DOWN=0.027 | 2.3min restantes
📊 BTCUSDT=$67,045.12 | strike=$67,029.48 | dist=+0.02% | RSI=52.1 | mom=+0.05%
   → UP (97%) [high]

🎯 Executando trade (BTC):
   Lado:    UP
   Preço:   $0.973/share
   Valor:   $2.00 USDC (2.1 shares)
   ✅ Order executada!
```

Se BTC fechar acima de $67,029.48:

- **Lucro:** (2.1 shares × $1.00) - $2.00 = **+$0.06**
- **ROI:** ~3% em 2 minutos

---

## ❓ FAQ

### Q: Qual modo devo usar?

**A:** Depende:

- **Iniciante?** → `main.py` (polling 30s)
- **Quer performance máxima?** → `hybrid_runner.py --mode websocket`
- **Equilíbrio?** → `hybrid_runner.py --mode polling` (5s)

### Q: Preciso das API Keys da Binance?

**A:** NÃO é obrigatório. O bot funciona sem elas usando a API pública da Binance. As keys apenas:

- Melhoram rate limits
- Habilitam WebSocket Binance (opcional)

### Q: O bot faz auto-claim das posições ganhas?

**A:** Não automaticamente. O "claim" é uma operação on-chain (Polygon) que requer:

- Interação direta com smart contracts via `web3.py`
- Pagamento de gas (MATIC)

**Alternativa:** Você pode configurar o bot para **vender** a posição vencedora a $0.99 antes do mercado fechar (evita claim manual).

### Q: Quanto posso ganhar?

**A:** Depende de vários fatores:

- **Número de oportunidades:** 5-20 por dia
- **Win rate:** ~85-95% (preço > $0.95)
- **Lucro médio por trade:** $0.05-$0.15
- **Estimativa:** $2-$5/dia com bankroll de $20

**⚠️ Lembre-se:** Trading envolve risco. Use apenas capital que pode perder.

### Q: O bot está perdendo oportunidades?

**A:** Se usar `main.py` (polling 30s), sim - pode perder trades rápidos. Solução:

```bash
# Mude para WebSocket (reação instantânea)
python hybrid_runner.py --mode websocket
```

### Q: Como sei se está funcionando?

**A:** Verifique:

1. **Console:** Mostra cada verificação e trade
2. **logs/trades.json:** Histórico completo
3. **Polymarket UI:** Confirme orders na interface web

---

## 📁 Estrutura do Projeto

```
polymarket-farm/
├── main.py                 # Modo Polling (básico)
├── hybrid_runner.py        # Modo Híbrido (Polling OU WebSocket)
├── event_runner.py         # Modo Event-Driven (legado)
├── websocket_client.py     # Cliente WebSocket Polymarket
├── market.py               # Busca mercados ativos
├── analysis.py             # Análise técnica (RSI, MACD)
├── executor.py             # Executa trades
├── check_creds.py          # Utilitário: testa credenciais
├── requirements.txt        # Dependências Python
├── .env                    # Configuração (VOCÊ CRIA)
└── logs/
    └── trades.json         # Histórico de trades
```

---

## 🛡️ Segurança

- ⚠️ **NUNCA** compartilhe seu `.env` ou private key
- ⚠️ Adicione `.env` no `.gitignore`
- ⚠️ Use carteira separada para trading automatizado
- ⚠️ Comece com capital baixo para testar

---

## 📊 Comparação de Modos

| Característica       | main.py    | hybrid (polling) | hybrid (websocket) |
| -------------------- | ---------- | ---------------- | ------------------ |
| **Delay**            | ~30s       | ~5s              | < 1s               |
| **Complexidade**     | Baixa      | Média            | Alta               |
| **Dependências**     | Básicas    | Básicas          | + websocket-client |
| **CPU/Rede**         | Baixo      | Médio            | Médio-Alto         |
| **Win Rate**         | Bom        | Muito Bom        | Excelente          |
| **Recomendado para** | Iniciantes | Intermediário    | Avançado           |

---

## 🔧 Troubleshooting (Resolução de Problemas)

### ❌ Erro 401: Unauthorized/Invalid api key

**Sintoma:**

```
❌ Erro ao executar: PolyApiException[status_code=401, error_message={'error': 'Unauthorized/Invalid api key'}]
```

**Causa:** API Keys da Polymarket estão ausentes, incorretas ou expiradas.

**Solução:**

1. **Execute o teste de diagnóstico:**

   ```bash
   python test_auth.py
   ```

2. **Se falhar, obtenha novas API Keys:**
   - Veja o guia completo: [COMO_OBTER_API_KEYS.md](COMO_OBTER_API_KEYS.md)
   - Resumo rápido:
     1. Acesse https://polymarket.com/settings/api-keys
     2. Delete API Key antiga (se existir)
     3. Crie nova API Key
     4. Copie os 3 valores (Key, Secret, Passphrase)
     5. Cole no `.env`

3. **Verifique o `.env`:**

   ```bash
   POLYMARKET_API_KEY="sua_key_completa"
   POLYMARKET_API_SECRET="seu_secret_completo"
   POLYMARKET_API_PASSPHRASE="seu_passphrase_completo"
   ```

   ⚠️ **Importante:**
   - SEM espaços antes/depois das aspas
   - Aspas DUPLAS ("), não simples (')
   - Valores COMPLETOS, sem truncar

4. **Teste novamente:**
   ```bash
   python test_auth.py
   ```

---

### ❌ ModuleNotFoundError: No module named 'websocket'

**Sintoma:**

```
ModuleNotFoundError: No module named 'websocket'
```

**Solução:**

```bash
pip install websocket-client
```

---

### ❌ WebSocket funciona mas trades falham

**Explicação:**
O **WebSocket da Polymarket NÃO precisa de autenticação** - é público. Mas **executar trades PRECISA de API Keys**.

Veja explicação completa: [WEBSOCKET_AUTENTICACAO.md](WEBSOCKET_AUTENTICACAO.md)

**Resumo:**

- ✅ WebSocket → Público (não precisa API Keys)
- ❌ Executar trades → Privado (PRECISA de API Keys válidas)

---

### ⚠️ Saldo insuficiente

**Sintoma:**

```
❌ Erro: Insufficient balance
```

**Solução:**

1. Verifique seu saldo:

   ```bash
   python test_auth.py
   ```

2. Deposite USDC na sua conta Polymarket
3. Aguarde confirmação na blockchain (Polygon)

---

### 🐌 Bot muito lento / Perdendo oportunidades

**Se usar `main.py` (polling 30s):**

Migre para modo WebSocket:

```bash
pip install websocket-client
python hybrid_runner.py --mode websocket --dry-run
```

**Ganho esperado:**

- Polling 30s → WebSocket: +67% de lucro por trade
- Entra a $0.95 em vez de $0.96-$0.97

---

### 📚 Guias Detalhados

| Problema                   | Guia                                                   |
| -------------------------- | ------------------------------------------------------ |
| Como obter API Keys        | [COMO_OBTER_API_KEYS.md](COMO_OBTER_API_KEYS.md)       |
| Erro 401 / Autenticação    | [COMO_OBTER_API_KEYS.md](COMO_OBTER_API_KEYS.md)       |
| WebSocket vs Polling       | [GUIA_RAPIDO.md](GUIA_RAPIDO.md)                       |
| WebSocket precisa de auth? | [WEBSOCKET_AUTENTICACAO.md](WEBSOCKET_AUTENTICACAO.md) |
| Qual modo escolher?        | [GUIA_RAPIDO.md](GUIA_RAPIDO.md)                       |

---

## 🤝 Suporte

- 📖 **Documentação Polymarket:** https://docs.polymarket.com/
- 💬 **Discord Polymarket:** https://discord.gg/polymarket
- 🐛 **Issues:** Abra uma issue neste repositório

---

Boa sorte nas operações! 🧠💰🎯

Developed by Chard
Rust engine (current)
=====================

The live engine was ported to Rust for lower startup overhead, typed config, and a reusable backtest flow. The old Python files are kept in this folder as legacy/reference code, but Docker now runs the Rust binary.

Polymarket integration follows the current CLOB model from the docs:

- public market data uses unauthenticated CLOB/Gamma/Binance HTTP reads
- live trading uses the official Rust CLOB SDK path for signing and posting orders
- default mode is paper trading: `DRY_RUN=true` and `ALLOW_LIVE_TRADING=false`

Run locally:

```bash
cargo run --bin bot
```

Backtest:

```bash
cargo run --bin backtest -- --days 3
cargo run --bin backtest -- --days 7 --asset BTC --interval 5 --trades
cargo run --bin sweep -- --days 3 --asset ETH --interval 15
```

Reconcile paper trades that stayed open after a restart:

```bash
cargo run --bin settle_open_trades
```

Important env vars:

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

# only for live trading
POLYMARKET_PRIVATE_KEY=
POLYMARKET_SIGNATURE_TYPE=proxy
```

---
