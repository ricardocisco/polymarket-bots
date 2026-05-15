"""
event_runner.py — Bot Multi-Asset Polymarket (Event-Driven Mode)

Estratégia REATIVA:
  1. Monitora preços da Binance via WebSocket em tempo real
  2. Monitora preços da Polymarket com polling frequente quando próximo da janela
  3. Reage INSTANTANEAMENTE quando:
     - Preço Polymarket >= $0.95
     - Tempo restante entre 0.5-3 min
     - Análise técnica confirma entrada
  4. Elimina perda de oportunidades por polling lento

Uso:
  python event_runner.py            → modo live
  python event_runner.py --dry-run  → simulação
"""
from __future__ import annotations

import sys
import time
import signal
import argparse
import threading
from datetime import datetime
from typing import Optional, Dict
from collections import defaultdict

from py_clob_client.client import ClobClient
from py_clob_client.clob_types import ApiCreds

import os
from dotenv import load_dotenv
load_dotenv()

try:
    from binance import ThreadedWebsocketManager
    BINANCE_WS_AVAILABLE = True
except ImportError:
    BINANCE_WS_AVAILABLE = False
    print("⚠️  python-binance não instalado. Instale com: pip install python-binance")

PRIVATE_KEY    = os.getenv("PRIVATE_KEY", "").strip()
FUNDER_ADDRESS = os.getenv("FUNDER_ADDRESS", "").strip()
SIGNATURE_TYPE = int(os.getenv("SIGNATURE_TYPE", "2"))
BANKROLL       = float(os.getenv("BANKROLL", "20.0"))

API_KEY        = os.getenv("POLYMARKET_API_KEY", "").strip()
API_SECRET     = os.getenv("POLYMARKET_API_SECRET", "").strip()
API_PASSPHRASE = os.getenv("POLYMARKET_API_PASSPHRASE", "").strip()

# Binance API Keys (OPCIONAL - apenas para WebSocket e rate limits maiores)
# Se não configurar, o bot funciona com polling rápido da Polymarket
# Veja BINANCE_API_SETUP.md para instruções de como obter
BINANCE_API_KEY    = os.getenv("BINANCE_API_KEY", "").strip()
BINANCE_API_SECRET = os.getenv("BINANCE_API_SECRET", "").strip()

MIN_EDGE = float(os.getenv("MIN_EDGE", "0.04"))

CLOB_HOST = "https://clob.polymarket.com"
CHAIN_ID  = 137

_running = True

def _handle_signal(sig, frame):
    global _running
    print("\n\nEncerrando bot com segurança...")
    _running = False

signal.signal(signal.SIGINT, _handle_signal)
signal.signal(signal.SIGTERM, _handle_signal)

from market import find_active_market, refresh_prices, MarketConfig, Market
from analysis import analyze, get_price
from executor import execute, should_trade, print_stats, history

BANNER = """
╔══════════════════════════════════════════════╗
║  POLYMARKET SNIPER BOT (EVENT-DRIVEN MODE)  ║
║  BTC, ETH, XRP | 5m & 15m | WebSocket       ║
╚══════════════════════════════════════════════╝
"""

# Configuração dos Mercados
MARKETS_TO_TRADE = [
    MarketConfig("BTC", 15, "BTCUSDT"),
    MarketConfig("ETH", 15, "ETHUSDT"),
    MarketConfig("XRP", 15, "XRPUSDT"),
    MarketConfig("BTC", 5,  "BTCUSDT"),
]

# Mapeia símbolo Binance -> lista de configs
SYMBOL_TO_CONFIGS: Dict[str, list[MarketConfig]] = defaultdict(list)
for cfg in MARKETS_TO_TRADE:
    SYMBOL_TO_CONFIGS[cfg.binance_symbol].append(cfg)


def create_client() -> ClobClient:
    if not PRIVATE_KEY or not FUNDER_ADDRESS:
        print("❌ Configure PRIVATE_KEY e FUNDER_ADDRESS no .env")
        sys.exit(1)
    
    client = ClobClient(
        host=CLOB_HOST,
        key=PRIVATE_KEY,
        chain_id=CHAIN_ID,
        signature_type=SIGNATURE_TYPE,
        funder=FUNDER_ADDRESS,
    )

    if API_KEY and API_SECRET and API_PASSPHRASE:
        print("✅ Usando credenciais de API do .env")
        creds = ApiCreds(
            api_key=API_KEY,
            api_secret=API_SECRET,
            api_passphrase=API_PASSPHRASE,
        )
        client.set_api_creds(creds)
    else:
        print("⚠️  Tentando derivar credenciais de API automaticamente...")
        try:
            creds = client.create_or_derive_api_creds()
            client.set_api_creds(creds)
        except Exception as e:
            print(f"❌ Falha ao derivar credenciais: {e}")
            print("💡 DICA: Crie uma API Key na Polymarket e adicione no .env")
    
    return client


class MarketMonitor:
    """Monitora mercados e dispara análise quando condições são atendidas."""
    
    def __init__(self, client: ClobClient, dry_run: bool = False):
        self.client = client
        self.dry_run = dry_run
        self.traded_slugs = set()
        self.last_check_time: Dict[str, float] = {}
        self.active_markets: Dict[str, Optional[Market]] = {}
        self.last_process_time: Dict[str, float] = {}  # Rate limiting por config
        
    def process_config(self, config: MarketConfig, trigger_reason: str = "event"):
        """Processa um único config - mesma lógica do main.py mas reutilizável."""
        try:
            # Rate limiting: não processa o mesmo config mais de uma vez a cada 1 segundo
            config_key = f"{config.asset}_{config.duration_minutes}m"
            now = time.time()
            if config_key in self.last_process_time:
                if now - self.last_process_time[config_key] < 1.0:
                    return False
            self.last_process_time[config_key] = now
            
            # 1. Encontrar mercado
            market = find_active_market(self.client, config)
            if not market:
                return False
            
            # 2. Atualizar preços
            market = refresh_prices(self.client, market)
            
            # 3. Verificar se já operamos neste mercado
            if market.slug in self.traded_slugs:
                return False
            
            # 4. Verificar janela de tempo ANTES de fazer análise pesada
            mins_left = market.minutes_left
            if mins_left < 0.5 or mins_left > 3.0:
                return False
            
            # 5. Verificar preço mínimo ANTES de análise técnica
            if market.up_price < 0.95 and market.down_price < 0.95:
                return False
            
            # 6. Buscar preço Binance
            current_price = get_price(config.binance_symbol)
            if not current_price:
                return False
            
            print(f"\n🚨 {trigger_reason.upper()} | {config.asset} {config.duration_minutes}m")
            print(f"📌 {market}")
            
            # 7. Analisar
            signal = analyze(
                symbol=config.binance_symbol,
                strike_price=market.strike_price if market.strike_price > 0 else current_price,
                minutes_left=market.minutes_left,
            )
            
            if not signal:
                return False
            
            print(f"📊 {signal}")
            
            # 8. Verificar entrada
            ok, reason = should_trade(
                signal, market, BANKROLL,
                min_edge=MIN_EDGE,
            )
            
            if not ok:
                print(f"⏭️  Pulando: {reason}")
                return False
            
            # 9. Executar
            success = execute(self.client, market, signal, BANKROLL, dry_run=self.dry_run)
            
            if success:
                self.traded_slugs.add(market.slug)
                if len(self.traded_slugs) > 50:
                    self.traded_slugs = set(list(self.traded_slugs)[-50:])
                print_stats()
                return True
            
            return False
            
        except Exception as e:
            print(f"❌ Erro processando {config.asset} {config.duration_minutes}m: {e}")
            return False
    
    def check_all_markets(self, trigger_reason: str = "poll"):
        """Verifica todos os mercados - usado quando não há eventos específicos."""
        for config in MARKETS_TO_TRADE:
            if not _running:
                break
            self.process_config(config, trigger_reason)
    
    def check_markets_for_symbol(self, symbol: str, trigger_reason: str = "price_update"):
        """Verifica apenas mercados relacionados a um símbolo Binance."""
        configs = SYMBOL_TO_CONFIGS.get(symbol, [])
        for config in configs:
            if not _running:
                break
            self.process_config(config, trigger_reason)


def binance_price_monitor(monitor: MarketMonitor):
    """Thread que monitora preços da Binance via WebSocket."""
    if not BINANCE_WS_AVAILABLE:
        print("⚠️  WebSocket Binance não disponível - usando polling")
        return
    
    if not BINANCE_API_KEY or not BINANCE_API_SECRET:
        print("⚠️  BINANCE_API_KEY e BINANCE_API_SECRET não configurados")
        print("   → Bot funcionará com polling rápido da Polymarket (ainda muito rápido!)")
        print("   → Para WebSocket em tempo real, veja BINANCE_API_SETUP.md\n")
        return
    
    print("🔌 Conectando WebSocket Binance...")
    
    try:
        twm = ThreadedWebsocketManager(
            api_key=BINANCE_API_KEY,
            api_secret=BINANCE_API_SECRET
        )
        twm.start()
        
        def handle_ticker(msg):
            """Callback quando preço muda."""
            if not _running:
                return
            
            try:
                symbol = msg.get("s")
                price = float(msg.get("c", 0))
                
                if symbol and price > 0:
                    # Dispara verificação para todos os configs deste símbolo
                    monitor.check_markets_for_symbol(symbol, f"binance_ticker_{symbol}")
            except Exception as e:
                print(f"⚠️  Erro no callback Binance: {e}")
        
        # Subscribe para ticker de todos os símbolos únicos
        symbols = list(SYMBOL_TO_CONFIGS.keys())
        for symbol in symbols:
            twm.start_symbol_ticker_socket(
                callback=handle_ticker,
                symbol=symbol
            )
            print(f"   ✅ Subscribed: {symbol}")
        
        print("✅ WebSocket Binance conectado e monitorando...\n")
        
        # Mantém vivo
        while _running:
            time.sleep(1)
        
        twm.stop()
        print("🔌 WebSocket Binance desconectado")
        
    except Exception as e:
        print(f"❌ Erro no WebSocket Binance: {e}")
        print("   Continuando com polling...")


def polymarket_polling_monitor(monitor: MarketMonitor):
    """Thread que faz polling frequente da Polymarket quando próximo da janela."""
    print("🔄 Iniciando monitoramento Polymarket (polling inteligente)...\n")
    
    # Polling mais frequente quando próximo da janela
    FAST_POLL_INTERVAL = 2.0  # 2 segundos quando próximo
    SLOW_POLL_INTERVAL = 10.0  # 10 segundos quando longe
    
    while _running:
        try:
            # Verifica todos os mercados
            monitor.check_all_markets("polymarket_poll")
            
            # Ajusta intervalo baseado em quão próximo estamos da janela
            # (simplificado - sempre rápido para garantir que não perdemos oportunidades)
            time.sleep(FAST_POLL_INTERVAL)
            
        except Exception as e:
            print(f"❌ Erro no polling Polymarket: {e}")
            time.sleep(5)


def run(dry_run: bool = False):
    """Executa o bot em modo orientado a eventos."""
    print(BANNER)
    if dry_run:
        print("=" * 48)
        print("  MODO DRY-RUN — nenhum trade real será feito")
        print("=" * 48 + "\n")
    
    print(f"Bankroll: ${BANKROLL:.2f} USDC")
    print(f"Modo: Event-Driven (WebSocket + Polling Inteligente)\n")
    
    client = create_client()
    print("✅ Cliente Polymarket inicializado\n")
    
    monitor = MarketMonitor(client, dry_run=dry_run)
    
    # Thread 1: WebSocket Binance (preços em tempo real)
    binance_thread = None
    if BINANCE_WS_AVAILABLE and BINANCE_API_KEY and BINANCE_API_SECRET:
        binance_thread = threading.Thread(
            target=binance_price_monitor,
            args=(monitor,),
            daemon=True
        )
        binance_thread.start()
    else:
        print("⚠️  WebSocket Binance desabilitado")
        print("   → Bot funcionará com polling rápido da Polymarket (a cada 2s)")
        print("   → Para habilitar WebSocket, configure BINANCE_API_KEY e BINANCE_API_SECRET no .env")
        print("   → Veja BINANCE_API_SETUP.md para instruções\n")
    
    # Thread 2: Polling Polymarket (verifica preços e janela de tempo)
    polling_thread = threading.Thread(
        target=polymarket_polling_monitor,
        args=(monitor,),
        daemon=True
    )
    polling_thread.start()
    
    print("🚀 Bot iniciado! Monitorando eventos...")
    print("   - WebSocket Binance: Preços em tempo real")
    print("   - Polling Polymarket: Verificação a cada 2s\n")
    print("Pressione Ctrl+C para encerrar\n")
    
    try:
        # Loop principal apenas para manter vivo e mostrar stats periodicamente
        last_stats_time = time.time()
        while _running:
            time.sleep(5)
            
            # Mostra stats a cada 30 segundos
            if time.time() - last_stats_time > 30:
                print_stats()
                last_stats_time = time.time()
                # Limpa slugs antigos periodicamente
                if len(monitor.traded_slugs) > 100:
                    monitor.traded_slugs.clear()
    
    except KeyboardInterrupt:
        pass
    
    print("\n\nBot encerrado.")
    print_stats()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Bot Multi-Asset Polymarket (Event-Driven)")
    parser.add_argument("--dry-run", action="store_true", help="Simula sem trades reais")
    args = parser.parse_args()
    run(dry_run=args.dry_run)
