# Bot de aviso Polymarket -> Discord

Esse bot monitora o evento da Polymarket:

https://polymarket.com/event/brazil-presidential-election/will-renan-santos-win-the-2026-brazilian-presidential-election

Ele manda uma notificacao no Discord somente quando detectar que um candidato foi adicionado ao evento:

- mercado novo com candidato;
- placeholder tipo `Person X` virando candidato real.

Ele ignora nome especifico, `Other`, preco, volume, liquidez, mercado ficando ativo/inativo, mudanca em ordens, fechamento ou qualquer outro update que nao represente candidato novo.

## Como rodar

1. Copie o arquivo de exemplo:

```powershell
Copy-Item .env.example .env
```

2. Edite `.env` e coloque seu `DISCORD_WEBHOOK_URL`.

3. Rode uma checagem inicial:

```powershell
npm run check
```

Na primeira execucao ele salva o estado atual e, por padrao, nao notifica. Isso evita mandar todos os candidatos existentes como se fossem novidade.

4. Deixe o bot rodando:

```powershell
npm start
```

## Teste sem enviar mensagem

Use:

```powershell
$env:DRY_RUN="true"; npm run check
```

## Testar o webhook do Discord

Para enviar uma mensagem fake ao canal e validar que o webhook esta funcionando:

```powershell
npm run test:webhook
```

Esse comando nao altera o estado salvo do bot. Ele consulta o evento real, simula um candidato novo chamado `Teste Novo Candidato` e envia exatamente o tipo de alerta que seria disparado quando a Polymarket adicionasse um candidato.

Para ver o payload sem postar no Discord:

```powershell
$env:DRY_RUN="true"; npm run test:webhook
```

Para mudar o nome usado no teste:

```powershell
$env:TEST_CANDIDATE_NAME="Candidato Simulado"; npm run test:webhook
```

## Observacoes

- A fonte e a Gamma API publica da Polymarket: `https://gamma-api.polymarket.com/events/slug/{slug}`.
- O endpoint costuma vir com cache HTTP. Por isso `BYPASS_CACHE=true` adiciona um parametro unico na URL a cada polling.
- Mantenha o webhook privado. Qualquer pessoa com essa URL consegue postar no canal.
- O bot apenas alerta sobre mudancas. Ele nao executa compra e nao e recomendacao financeira.
