# 🚀 Guia Rápido - Escolhendo o Modo Ideal

## TL;DR - Qual modo usar?

```bash
# Iniciante? Use isso:
python main.py --dry-run

# Quer performance máxima? Use isso:
pip install websocket-client
python hybrid_runner.py --mode websocket --dry-run

# Equilíbrio? Use isso:
python hybrid_runner.py --mode polling --dry-run
```

---

## 📊 Comparação Detalhada

### ⏱️ Cenário: Preço sobe para $0.95

| Modo | Detecção | Execução | Total | Entrada Real |
|------|----------|----------|-------|--------------|
| **main.py (30s polling)** | 0-30s | 2s | 2-32s | $0.95-$0.97 |
| **hybrid polling (5s)** | 0-5s | 2s | 2-7s | $0.95-$0.96 |
| **hybrid websocket** | < 0.5s | 2s | < 2.5s | $0.95 ✓ |

**Resultado:** WebSocket entra no preço ideal, polling pode perder $0.01-$0.02 por share.

---

## 💰 Impacto Financeiro (exemplo real)

### Cenário: BTC bate $0.95 às 14:58:30, fecha às 15:00:00

**Trade:** 2 shares (~$2.00 investimento)

#### main.py (polling 30s)
- ⏰ Detecta: 14:58:45 (delay 15s)
- 💵 Preço: $0.968 (subiu enquanto esperava)
- 💸 Custo: $1.94
- ✅ Lucro: $2.00 - $1.94 = **$0.06**

#### hybrid_runner.py --mode websocket
- ⏰ Detecta: 14:58:30 (< 1s)
- 💵 Preço: $0.952
- 💸 Custo: $1.90
- ✅ Lucro: $2.00 - $1.90 = **$0.10** (+67% vs polling)

**Em 20 trades/dia:** Diferença de **$0.80/dia** = **$24/mês**

---

## 🎯 Recomendações

### Use `main.py` se:
- ✅ Primeira vez usando o bot
- ✅ Quer algo simples e confiável
- ✅ OK com perder alguns centavos por trade
- ✅ Não quer instalar dependências extras

### Use `hybrid_runner.py --mode polling` se:
- ✅ Quer melhor performance que main.py
- ✅ Ainda prefere simplicidade
- ✅ 5s de delay é aceitável
- ✅ Planeja migrar para WebSocket depois

### Use `hybrid_runner.py --mode websocket` se:
- ✅ Quer máxima performance
- ✅ Cada centavo importa (trading sério)
- ✅ Tem experiência com bots
- ✅ OK com configuração um pouco mais complexa

---

## 🔧 Setup por Modo

### main.py (Mais Simples)

```bash
# 1. Instalar
pip install -r requirements.txt

# 2. Configurar .env
cp .env.example .env
nano .env  # Preencher credenciais

# 3. Testar
python main.py --dry-run

# 4. Rodar
python main.py
```

**Total:** 5 minutos

---

### hybrid_runner.py --mode polling

```bash
# 1. Instalar (mesmo que main.py)
pip install -r requirements.txt

# 2. Configurar .env
cp .env.example .env
nano .env

# 3. Testar
python hybrid_runner.py --mode polling --dry-run

# 4. Rodar
python hybrid_runner.py --mode polling
```

**Total:** 5 minutos

---

### hybrid_runner.py --mode websocket (Máxima Performance)

```bash
# 1. Instalar (inclui websocket-client)
pip install -r requirements.txt

# 2. Configurar .env
cp .env.example .env
nano .env  # Preencher credenciais

# 3. (OPCIONAL) Adicionar Binance API Keys para WebSocket
# Edite .env:
# BINANCE_API_KEY="sua_key"
# BINANCE_API_SECRET="seu_secret"

# 4. Testar
python hybrid_runner.py --mode websocket --dry-run

# 5. Rodar
python hybrid_runner.py --mode websocket
```

**Total:** 7 minutos (+ 2 min se configurar Binance)

---

## ⚡ Performance Real (Logs)

### Polling 30s (main.py)

```
─── Ciclo #12 | 14:58:45 ────────────────────────
📌 BTC 15M | strike=$67,029.48 | UP=0.968 DOWN=0.032 | 1.3min
📊 BTCUSDT=$67,045.12 | RSI=52.1 | → UP (96%) [high]
🎯 Executando trade:
   Preço:   $0.968/share  ← perdeu oportunidade de $0.95
   ✅ Order executada
```

### WebSocket (hybrid_runner.py)

```
🎯 OPORTUNIDADE DETECTADA (WEBSOCKET)!
📌 BTC 15M | strike=$67,029.48 | UP=0.952 DOWN=0.048 | 1.5min
📊 BTCUSDT=$67,042.00 | RSI=51.8 | → UP (95%) [high]
🎯 Executando trade:
   Preço:   $0.952/share  ← entrou no preço ideal ✓
   ✅ Order executada
```

**Diferença:** $0.016/share × 2.1 shares = **$0.034 a mais de lucro**

---

## 🎓 Curva de Aprendizado

```
Tempo para dominar:

main.py             ████░░░░░░ (1 hora)
hybrid polling      █████░░░░░ (2 horas)
hybrid websocket    ███████░░░ (3-4 horas)
```

## 🔥 Recomendação Final

**Semana 1:** Use `main.py --dry-run` para aprender
**Semana 2:** Migre para `hybrid_runner.py --mode polling`
**Semana 3+:** Ative WebSocket com `--mode websocket`

Essa progressão minimiza erros e maximiza aprendizado!

---

## 💡 Dicas Pro

1. **Sempre teste com --dry-run primeiro**
2. **Monitore logs/** para entender o comportamento
3. **Comece com BANKROLL baixo** ($10-20)
4. **WebSocket não é mágico** - a estratégia é a mesma, apenas mais rápida
5. **Binance API Keys** são opcionais - WebSocket Polymarket já é suficiente

---

Boa sorte! 🎯💰
