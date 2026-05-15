# 📊 Polymarket Tracker Bot

Um bot Discord que rastreia em tempo real as atividades de carteiras na **Polymarket**, enviando notificações instantâneas quando detecta mudanças no portfolio de usuários monitorados.

## 🎯 O que faz

O bot monitora carteiras da Polymarket e notifica sobre:

- 🆕 **Novas posições abertas** - Quando um trader abre uma nova posição em um mercado
- 📈 **Aumentos de posições** - Quando há incremento na quantidade de shares
- 📉 **Reduções/vendas parciais** - Quando há diminuição em posições existentes
- 🔴 **Fechamento de posições** - Quando uma posição é completamente vendida

As verificações ocorrem automaticamente a cada **10 segundos** para detectar mudanças rapidamente.

## 🚀 Configuração

### Pré-requisitos

- [Bun](https://bun.sh/) ou Node.js
- Um bot Discord criado no [Discord Developer Portal](https://discord.com/developers/applications)
- Um servidor Discord para testar
- MongoDB (local ou cloud como MongoDB Atlas)

### Instalação

1. **Clone o repositório**

   ```bash
   git clone https://github.com/ricardocisco/polymarket-tracker
   cd polymarket-tracker
   ```

2. **Instale as dependências**

   ```bash
   bun install
   ```

3. **Configure as variáveis de ambiente**

   Crie um arquivo `.env` na raiz do projeto:

   ```env
   DISCORD_TOKEN=seu_token_aqui
   CLIENT_ID=seu_client_id_aqui
   MONGO_URI=mongodb://localhost:27017/polybot
   PORT=3000
   ```

   - `DISCORD_TOKEN`: Token do seu bot (em [Discord Developer Portal](https://discord.com/developers/applications) → Bot → Reset Token)
   - `CLIENT_ID`: Client ID do seu bot (em [Discord Developer Portal](https://discord.com/developers/applications) → OAuth2 → Client ID)
   - `MONGO_URI`: URI de conexão com MongoDB
   - `PORT`: Porta para a API Elysia (padrão: 3000)

4. **Registre os comandos no Discord**

   ```bash
   bun run src/deploy-command.ts
   ```

5. **Inicie o bot**
   ```bash
   bun run src/index.ts
   ```

## 📋 Comandos Disponíveis

### `/track <endereço_ou_username>`

Ativa o rastreamento de uma carteira Polymarket neste canal.

**Exemplos:**

```
/track 0x742d35Cc6634C0532925a3b844Bc6e7481842A8
/track @ethereum_trader
```

**Aceita:**

- ✅ Endereço Ethereum completo (0x...)
- ✅ Username Polymarket (@username)

**Resposta:**

```
✅ Rastreamento Ativado!

📡 Carteira: 0x742d...842A8
⏰ Você receberá alertas de mudanças no portfolio (novas posições, aumentos, vendas).

💡 Como funciona: O bot compara o portfolio a cada 10s e detecta:
  • 🆕 Novas posições abertas
  • 📈 Aumentos em posições existentes
  • 📉 Reduções/vendas parciais
  • 🔴 Fechamento de posições
```

---

### `/untrack <endereço_ou_username>`

Desativa o rastreamento de uma carteira neste canal.

**Exemplos:**

```
/untrack 0x742d35Cc6634C0532925a3b844Bc6e7481842A8
/untrack @ethereum_trader
```

**Resposta:**

```
✅ Rastreamento Removido!

A carteira 0x742d...842A8 não será mais monitorada neste canal.
```

---

### `/list`

Lista todas as carteiras sendo rastreadas no canal atual.

**Resposta:**

```
📊 Carteiras Rastreadas Neste Canal:

1. 0x742d35Cc6634C0532925a3b844Bc6e7481842A8
2. 0xabc123def456789...
3. @polymarket_pro

Total: 3 carteiras ativas
```

---

### `/debug <endereço_ou_username>`

Testa a conectividade com as APIs da Polymarket e valida se um endereço/username existe.

**Exemplos:**

```
/debug 0x742d35Cc6634C0532925a3b844Bc6e7481842A8
/debug @ethereum_trader
```

**Resposta (Sucesso):**

```
✅ Teste de Conectividade

📡 API Status: ✅ Online
👤 Endereço Validado: 0x742d35Cc6634C0532925a3b844Bc6e7481842A8
📊 Portfolio: 5 posições ativas
```

**Resposta (Erro):**

```
❌ Endereço não encontrado

Certifique-se de que:
• O username está correto (ex: @nickname)
• Ou use o endereço 0x completo da carteira
```

---

### `/help`

Mostra uma mensagem de ajuda com informações sobre todos os comandos.

## 📊 Formato das Notificações

Quando o bot detecta mudanças, envia mensagens formatadas assim:

```
🚨 NOVA POSIÇÃO DETECTADA

📊 Trader: @username (0x742d...842A8)
🏛️ Mercado: Will Bitcoin reach $100k by EOY 2024?
🎯 Ação: COMPROU
💰 Valor: $500.00
📈 Shares: 10 YES

ID da Transação: 0xabc123...
⏰ Detectado: 13 de janeiro de 2026 às 15:30:45
```

## 🗄️ Estrutura do Banco de Dados

### Modelo: Wallet

```typescript
{
  _id: ObjectId,
  address: string,           // Endereço 0x da carteira
  lastTimestamp: number      // Último timestamp verificado
}
```

### Modelo: Subscription

```typescript
{
  _id: ObjectId,
  channelId: string,         // ID do canal Discord
  walletAddress: string,     // Endereço 0x monitorado
  userId: string             // ID do usuário que criou o rastreamento
}
```

## 🏗️ Arquitetura do Projeto

```
src/
├── index.ts              # Entrada principal, configura Elysia + Discord.js
├── bot.ts               # Lógica dos comandos slash do Discord
├── deploy-command.ts    # Script para registrar comandos no Discord
├── tracker.ts           # Loop de verificação de mudanças (10s)
├── polymarket.ts        # API do Polymarket (resolve endereços, fetch dados)
└── models.ts            # Schemas do MongoDB (Wallet, Subscription)
```

### Fluxo de Execução

1. **Inicialização**: O bot conecta ao Discord e MongoDB
2. **Registro de Comandos**: Comandos slash são registrados via `/deploy-command.ts`
3. **Interação do Usuário**: Usuário usa `/track` → Cria registro no MongoDB
4. **Loop de Verificação**: A cada 10s, `tracker.ts` verifica todas as carteiras
5. **Detecção de Mudanças**: Se detectada mudança → Envia mensagem no Discord
6. **API Elysia**: API simples para status e health checks

## ⚙️ Configuração Avançada

### Alterar Intervalo de Verificação

Em `src/tracker.ts`, procure por:

```typescript
const CHECK_INTERVAL = 10000; // 10 segundos
```

Mude o valor (em milissegundos) para o desejado.

### Cache de Mensagens

O bot mantém um cache de 2 minutos para evitar duplicação:

```typescript
const MESSAGE_CACHE_TTL = 120000; // 2 minutos
```

## 🐛 Troubleshooting

### "❌ Não consegui encontrar o endereço"

- Verifique se o username está escrito corretamente
- Tente usar o endereço 0x completo em vez de username
- Certifique-se de que a carteira existe na Polymarket

### "❌ Erro interno ao salvar no banco de dados"

- Verifique se MongoDB está rodando
- Confirme a variável `MONGO_URI` está correta no `.env`

### "🔄 Registrando comandos slash..." (travado)

- Verifique se `DISCORD_TOKEN` e `CLIENT_ID` estão corretos
- Confirme que o bot tem permissões `applications.commands` no servidor

## 📝 Licença

MIT

## 🤝 Contribuições

Sinta-se livre para abrir issues e pull requests!

---

**Desenvolvido com ❤️ para a comunidade Polymarket**

Developed by Chard
