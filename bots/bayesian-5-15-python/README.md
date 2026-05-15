# Polymarket Bayesian 5/15 Bot

Bot Python para mercados cripto `Up or Down` de 5m e 15m na Polymarket. Ele combina sinais tecnicos, modelo Bayesiano e Kelly Criterion para decidir direcao e tamanho de posicao.

## Como funciona

O bot monitora BTC, ETH, SOL e XRP. Para cada mercado ativo, ele:

- le candles de 1 minuto da Binance;
- calcula sinais como momentum, tendencia, volume e volatilidade;
- estima probabilidade de `UP` ou `DOWN`;
- valida preco/liquidez no order book da Polymarket;
- calcula stake pelo Kelly fracionario;
- roda em dry-run ou envia ordem real, conforme configuracao.

## Techs

- Python 3.11+
- requests
- websockets
- numpy/pandas/scipy
- python-dotenv
- py-clob-client
- Binance API
- Gamma/CLOB APIs da Polymarket

## Instalar

```bash
cd bots/bayesian-5-15-python
python -m venv venv
venv\Scripts\activate
pip install -r requirements.txt
```

No Linux/macOS, use `source venv/bin/activate`.

## Configurar conta

```bash
copy .env.example .env
```

Preencha:

```env
POLYMARKET_PRIVATE_KEY=
POLYMARKET_FUNDER=
POLYMARKET_SIGNATURE_TYPE=1

# Opcional, se voce ja tiver credenciais L2
POLYMARKET_API_KEY=
POLYMARKET_API_SECRET=
POLYMARKET_API_PASSPHRASE=
```

Use `POLYMARKET_SIGNATURE_TYPE=1` para conta criada pelo site/Google/email. Use `0` para wallet direta e `2` para Safe/proxy quando aplicavel.

## Rodar

Primeiro em dry-run:

```bash
python main.py --mode AGGRESSIVE_OPTIMIZED --bankroll 20 --dry-run
```

Trading real, somente depois de validar:

```bash
python main.py --mode AGGRESSIVE_OPTIMIZED --bankroll 20 --live
```

Via Docker, a partir de `bots/`:

```bash
docker compose --profile bayesian up --build bayesian-bot
```

## Backtest

O backtest nao precisa de chave privada e nao envia ordens.

```bash
python backtest.py --days 3
python backtest.py --days 7 --mode AGGRESSIVE_OPTIMIZED --asset BTC --interval 5 --trades
python backtest.py --days 14 --bankroll 20 --stake 1
```

Logs e resultados de trades ficam em arquivos `trades_log*.json` e devem continuar fora do git.

