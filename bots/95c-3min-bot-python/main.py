"""
main.py — Bot Multi-Asset Polymarket (Sniper Mode)

Estratégia:
  1. Monitora BTC, ETH, SOL, XRP em 15m e BTC em 5m.
  2. Procura oportunidades "Sniper" (fim do intervalo, preço > $0.95).
  3. Valida com análise técnica e executa.

Uso:
  python main.py            → modo live
  python main.py --dry-run  → simulação
"""
from __future__ import annotations

import sys
import time
import signal
import argparse
from datetime import datetime

from py_clob_client.client import ClobClient
from py_clob_client.clob_types import ApiCreds

import os
from dotenv import load_dotenv
load_dotenv()

PRIVATE_KEY    = os.getenv("PRIVATE_KEY", "").strip()
FUNDER_ADDRESS = os.getenv("FUNDER_ADDRESS", "").strip()
SIGNATURE_TYPE = int(os.getenv("SIGNATURE_TYPE", "2"))
BANKROLL       = float(os.getenv("BANKROLL", "20.0")) # Atualizado para $20

API_KEY        = os.getenv("POLYMARKET_API_KEY", "").strip()
API_SECRET     = os.getenv("POLYMARKET_API_SECRET", "").strip()
API_PASSPHRASE = os.getenv("POLYMARKET_API_PASSPHRASE", "").strip()

LOOP_INTERVAL  = int(os.getenv("LOOP_INTERVAL", "20"))
MIN_EDGE       = float(os.getenv("MIN_EDGE", "0.04"))

CLOB_HOST = "https://clob.polymarket.com"
CHAIN_ID  = 137

_running = True

def _handle_signal(sig, frame):
    global _running
    print("\n\nEncerrando bot com segurança...")
    _running = False

signal.signal(signal.SIGINT, _handle_signal)
signal.signal(signal.SIGTERM, _handle_signal)

from market import find_active_market, refresh_prices, MarketConfig
from analysis import analyze, get_price
from executor import execute, should_trade, print_stats, history

BANNER = """
╔══════════════════════════════════════════════╗
║    POLYMARKET SNIPER BOT (MULTI-ASSET)       ║
║    BTC, ETH, XRP | 5m & 15m                  ║
╚══════════════════════════════════════════════╝
"""

# Configuração dos Mercados
MARKETS_TO_TRADE = [
    MarketConfig("BTC", 15, "BTCUSDT"),
    MarketConfig("ETH", 15, "ETHUSDT"),
    MarketConfig("XRP", 15, "XRPUSDT"),
    MarketConfig("BTC", 5,  "BTCUSDT"),
    MarketConfig("ETH", 5, "ETHUSDT"),
    MarketConfig("XRP", 5, "XRPUSDT"),
]

def create_client(dry_run: bool = False) -> ClobClient:
    if dry_run:
        print("[DRY-RUN] Usando cliente CLOB publico, sem validar credenciais de trading.")
        return ClobClient(host=CLOB_HOST, chain_id=CHAIN_ID)

    if not PRIVATE_KEY or not FUNDER_ADDRESS:
        print("❌ Configure PRIVATE_KEY e FUNDER_ADDRESS no .env")
        sys.exit(1)
    
    # Validação de API Keys ANTES de criar o cliente
    if not API_KEY or not API_SECRET or not API_PASSPHRASE:
        print("\n" + "="*60)
        print("  ❌ ERRO: API KEYS DA POLYMARKET NÃO CONFIGURADAS")
        print("="*60)
        print("\nPara executar trades, você PRECISA de API Keys válidas.")
        print("\n📋 SOLUÇÃO:")
        print("   1. Vá para: https://polymarket.com/settings/api-keys")
        print("   2. Crie uma nova API Key")
        print("   3. Cole as credenciais no arquivo .env:")
        print("      POLYMARKET_API_KEY=\"sua_key\"")
        print("      POLYMARKET_API_SECRET=\"seu_secret\"")
        print("      POLYMARKET_API_PASSPHRASE=\"seu_passphrase\"")
        print("\n📖 Guia completo: Veja COMO_OBTER_API_KEYS.md")
        print("🧪 Teste depois: python test_auth.py\n")
        sys.exit(1)
    
    client = ClobClient(
        host=CLOB_HOST,
        key=PRIVATE_KEY,
        chain_id=CHAIN_ID,
        signature_type=SIGNATURE_TYPE,
        funder=FUNDER_ADDRESS,
    )

    print("✅ Configurando API credentials...")
    creds = ApiCreds(
        api_key=API_KEY,
        api_secret=API_SECRET,
        api_passphrase=API_PASSPHRASE,
    )
    client.set_api_creds(creds)
    
    # Valida credenciais com um teste rápido
    try:
        print("🔐 Validando API Keys...")
        client.get_api_keys()
        print("✅ Autenticação OK\n")
    except Exception as e:
        error_msg = str(e)
        if "401" in error_msg or "Unauthorized" in error_msg or "Invalid api key" in error_msg:
            print("\n" + "="*60)
            print("  ❌ ERRO 401: API KEYS INVÁLIDAS")
            print("="*60)
            print("\nSuas API Keys estão incorretas ou expiradas.")
            print("\n📋 SOLUÇÃO:")
            print("   1. Vá para: https://polymarket.com/settings/api-keys")
            print("   2. DELETE a API Key antiga")
            print("   3. Crie uma NOVA API Key")
            print("   4. Atualize o arquivo .env com os novos valores")
            print("\n📖 Guia completo: Veja COMO_OBTER_API_KEYS.md")
            print("🧪 Teste depois: python test_auth.py\n")
            sys.exit(1)
        else:
            print(f"\n❌ Erro inesperado ao validar API Keys: {e}")
            print("🧪 Execute: python test_auth.py para diagnóstico completo\n")
            sys.exit(1)
    
    return client


def run(dry_run: bool = False):
    print(BANNER)
    if dry_run:
        print("=" * 48)
        print("  MODO DRY-RUN — nenhum trade real será feito")
        print("=" * 48 + "\n")

    print(f"Bankroll: ${BANKROLL:.2f} USDC")
    print(f"Loop a cada: {LOOP_INTERVAL}s\n")

    client = create_client(dry_run=dry_run)

    cycle        = 0
    traded_slugs = set()

    while _running:
        cycle += 1
        now = datetime.now().strftime("%H:%M:%S")
        print(f"\n─── Ciclo #{cycle} | {now} " + "─" * 30)

        active_found = False

        for config in MARKETS_TO_TRADE:
            # print(f"\n🔎 Buscando {config.asset} {config.duration_minutes}m...")
            
            # 1. Encontrar mercado
            market = find_active_market(client, config)
            if not market:
                continue
            
            active_found = True

            # 2. Atualizar preços
            market = refresh_prices(client, market)
            print(f"📌 {market}")

            # 3. Buscar preço real
            current_price = get_price(config.binance_symbol)
            if current_price:
                # print(f"💰 {config.asset} atual: ${current_price:,.4f}")
                pass

            # 4. Analisar
            signal = analyze(
                symbol=config.binance_symbol,
                strike_price=market.strike_price if market.strike_price > 0 else (current_price or 0),
                minutes_left=market.minutes_left,
            )

            if not signal:
                print(f"⚠️  Sem dados para {config.asset}")
                continue

            print(f"📊 {signal}")

            # 5. Verificar entrada
            if market.slug in traded_slugs:
                print(f"↩️  Já operado neste ciclo.")
                continue

            ok, reason = should_trade(
                signal, market, BANKROLL,
                min_edge=MIN_EDGE,
            )

            if not ok:
                print(f"⏭️  Pulando: {reason}")
                continue

            # 6. Executar
            success = execute(client, market, signal, BANKROLL, dry_run=dry_run)

            if success:
                traded_slugs.add(market.slug)
                if len(traded_slugs) > 50:
                    traded_slugs = set(list(traded_slugs)[-50:])

        if not active_found:
            print("⏳ Nenhum mercado ativo na janela de interesse.")

        print_stats()
        time.sleep(LOOP_INTERVAL)

    print("\nBot encerrado.")
    print_stats()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Bot Multi-Asset Polymarket")
    parser.add_argument("--dry-run", action="store_true", help="Simula sem trades reais")
    args = parser.parse_args()
    run(dry_run=args.dry_run)
