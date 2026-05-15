# 🔑 Como Funcionam as Credenciais da Polymarket (Trading API)

## ⚠️ IMPORTANTE: Não Existe Interface Web!

A página **`https://polymarket.com/settings/api` NÃO EXISTE** e você verá um erro 404.

**Isso é normal!** As credenciais de Trading API da Polymarket **não são criadas manualmente**.

---

## 📚 Entendendo as Duas APIs Diferentes

A Polymarket tem **DUAS APIs COMPLETAMENTE SEPARADAS**:

### 1️⃣ Trading API (CLOB - Central Limit Order Book)

- **Propósito**: Executar trades, comprar/vender posições
- **Biblioteca**: `py-clob-client`
- **Documentação**: https://docs.polymarket.com/trading/quickstart
- **Variáveis .env**: `POLYMARKET_API_KEY`, `POLYMARKET_API_SECRET`, `POLYMARKET_API_PASSPHRASE`
- **Como obter**: Geradas PROGRAMATICAMENTE da sua PRIVATE_KEY ✅ (É O QUE VOCÊ PRECISA)

### 2️⃣ Builder API

- **Propósito**: Criar novos mercados customizados
- **Biblioteca**: `py_builder_signing_sdk`
- **Documentação**: https://docs.polymarket.com/builders/api-keys
- **Portal**: https://console.polymarket.com
- **Variáveis .env**: `POLY_BUILDER_API_KEY` etc
- **Como obter**: Requer aprovação da Polymarket
- ❌ **NÃO É O QUE VOCÊ QUER PARA TRADING**

---

## 🔐 Como Obter Suas Credenciais (Método Correto)

### Método Oficial (Documentado pela Polymarket)

As credenciais são **DERIVADAS** da sua `PRIVATE_KEY` usando o método `create_or_derive_api_creds()`:

```python
from py_clob_client.client import ClobClient

# Apenas com private key
client = ClobClient(
    host="https://clob.polymarket.com",
    chain_id=137,  # Polygon
    key="0xsua_private_key_aqui"
)

# Deriva/cria as credenciais (determinístico)
creds = client.create_or_derive_api_creds()

print(creds.api_key)        # POLYMARKET_API_KEY
print(creds.api_secret)     # POLYMARKET_API_SECRET
print(creds.api_passphrase) # POLYMARKET_API_PASSPHRASE
```

**Características importantes:**

✅ **Determinístico**: A mesma private key SEMPRE gera as mesmas credenciais  
✅ **Sem registro manual**: Não precisa criar conta em nenhum portal  
✅ **Sem aprovação**: Instantâneo, funciona imediatamente  
✅ **Único conjunto**: Cada wallet tem apenas 1 conjunto de credenciais válido

---

## 🚀 Passo a Passo Prático

### 1. Execute o script gerador

```bash
python generate_api_keys.py
```

Este script:

- Lê sua `PRIVATE_KEY` do arquivo `.env`
- Deriva suas credenciais usando o método oficial
- Exibe as 3 variáveis para copiar

### 2. Copie as credenciais para o .env

```env
POLYMARKET_API_KEY=019c929c-0ab7-7910-a148-ade4a1753930
POLYMARKET_API_SECRET=miLSVQHBprwGXteN6w1cGwf1vbt8xIY4vK-2NQNZvk0=
POLYMARKET_API_PASSPHRASE=ca4446db05f77dddda8d45b34b77ee6eae86bda8e1005245802c55db5e669e01
```

⚠️ **Importante**: Não use aspas! Cole os valores diretamente.

### 3. Valide a autenticação

```bash
python test_auth.py
```

### 4. Execute o bot

```bash
python main.py
# OU
python hybrid_runner.py --mode websocket
```

---

## 🔍 Por Que Funciona Assim?

Da [documentação oficial](https://docs.polymarket.com/trading/clients/l1):

> **L1 Methods** require the client to initialize with a **signer (private key)** but do not require user API credentials. Use these for **initial setup**.
>
> ### createOrDeriveApiKey
>
> Convenience method that **attempts to derive an API key** with the default nonce, or creates a new one if it doesn't exist. **Recommended for initial setup**.

### Fluxo de Autenticação da Polymarket

```
┌─────────────────┐
│  PRIVATE_KEY    │
└────────┬────────┘
         │
         │ createOrDeriveApiKey()
         ▼
┌─────────────────────────────────────┐
│  API Credentials (L2 Auth Headers)  │
│  • api_key                          │
│  • api_secret                       │
│  • api_passphrase                   │
└────────┬────────────────────────────┘
         │
         │ Usado em L2 Methods
         ▼
┌─────────────────────────────────────┐
│  Trade Execution                    │
│  • postOrder()                      │
│  • cancelOrder()                    │
│  • getOpenOrders()                  │
└─────────────────────────────────────┘
```

### Níveis de Métodos

1. **Public Methods** (sem autenticação)
   - `getMarkets()`, `getOrderBook()`, `getPrice()`
   - Usado para ler dados públicos

2. **L1 Methods** (apenas PRIVATE_KEY)
   - `createOrDeriveApiKey()` ← **GERA AS CREDENCIAIS**
   - `createOrder()`, `createMarketOrder()`
   - Usado para setup inicial e assinatura local

3. **L2 Methods** (API Credentials obrigatórias)
   - `postOrder()`, `cancelOrder()`, `getTrades()`
   - Usado para executar ações no orderbook

---

## ❓ Perguntas Frequentes

### Por que não tem interface web?

A Polymarket usa autenticação baseada em **assinatura criptográfica**. Suas credenciais são derivadas matematicamente da sua private key usando EIP-712. Não há "cadastro" - sua wallet **É** sua identidade.

### Posso ter múltiplas credenciais?

Não. Cada private key deriva exatamente **1 conjunto de credenciais**. Se você chamar `createOrDeriveApiKey()` novamente, receberá as mesmas credenciais.

### E se eu perder as credenciais?

Basta executar `generate_api_keys.py` novamente. Como o processo é determinístico, você receberá as **mesmas credenciais**.

### WebSocket precisa de credenciais?

**NÃO!** O WebSocket da Polymarket para dados de mercado é público:

- `wss://ws-subscriptions-clob.polymarket.com/ws/market`

Você só precisa de credenciais para **executar trades** (via L2 methods).

### Onde fica minha conta na Polymarket?

Se você está usando `SIGNATURE_TYPE=2` (Gnosis Safe/Proxy Wallet), sua conta fica em:

- https://polymarket.com/profile (depois de fazer login com a wallet)

Mas você NÃO precisa fazer login lá para usar a API. Sua private key é suficiente.

---

## 📖 Referências Oficiais

- [Trading Quickstart](https://docs.polymarket.com/trading/quickstart)
- [L1 Methods (API Key Derivation)](https://docs.polymarket.com/trading/clients/l1#createorderiveapikey)
- [L2 Methods (Trade Execution)](https://docs.polymarket.com/trading/clients/l2)
- [Authentication Deep Dive](https://docs.polymarket.com/api-reference/authentication)

---

## ✅ Resumo Final

1. ❌ **NÃO existe** https://polymarket.com/settings/api
2. ❌ **NÃO use** Builder API (é para criar mercados)
3. ✅ **USE** `generate_api_keys.py` para derivar suas credenciais
4. ✅ As credenciais são **geradas da sua PRIVATE_KEY**
5. ✅ O processo é **determinístico e oficial**

Execute agora:

```bash
python generate_api_keys.py
```
