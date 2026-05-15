"""
hybrid_runner.py — Bot Polymarket com OPÇÃO de WebSocket ou Polling

MODOS DISPONÍVEIS:
  1. POLLING (padrão): Busca preços a cada X segundos (mais simples, sem dependências)
  2. WEBSOCKET: Tempo real via WebSocket Polymarket + Binance (reação instantânea)

COMO USAR:
  python hybrid_runner.py                      → Modo POLLING (padrão)
  python hybrid_runner.py --mode websocket     → Modo WEBSOCKET (tempo real)
  python hybrid_runner.py --dry-run            → Simulação sem trades reais
  
CONFIGURAÇÃO (.env):
  # Para modo WebSocket, adicione:
  USE_WEBSOCKET=true  # ou false para polling

VANTAGENS de cada modo:
  
  POLLING:
  ✓ Mais simples, sem dependências extras
  ✓ Funciona offline (sem conexão persistente)
  ✓ Controle total do intervalo de verificação
  ✗ Delay de até 30s para detectar oportunidades
  ✗ Pode perder oportunidades rápidas
  
  WEBSOCKET:
  ✓ Reação INSTANTÂNEA a mudanças de preço
  ✓ Entra exatamente quando preço bate $0.95
  ✓ Sem perder oportunidades por delay
  ✗ Requer conexão persistente
  ✗ Dependência: pip install websocket-client
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

PRIVATE_KEY    = os.getenv("PRIVATE_KEY", "").strip()
FUNDER_ADDRESS = os.getenv("FUNDER_ADDRESS", "").strip()
SIGNATURE_TYPE = int(os.getenv("SIGNATURE_TYPE", "2"))
BANKROLL       = float(os.getenv("BANKROLL", "20.0"))

API_KEY        = os.getenv("POLYMARKET_API_KEY", "").strip()
API_SECRET     = os.getenv("POLYMARKET_API_SECRET", "").strip()
API_PASSPHRASE = os.getenv("POLYMARKET_API_PASSPHRASE", "").strip()

# Binance API Keys (opcional - para WebSocket Binance)
BINANCE_API_KEY    = os.getenv("BINANCE_API_KEY", "").strip()
BINANCE_API_SECRET = os.getenv("BINANCE_API_SECRET", "").strip()

# Config
USE_WEBSOCKET  = os.getenv("USE_WEBSOCKET", "false").lower() == "true"
POLL_INTERVAL  = int(os.getenv("POLL_INTERVAL", "5"))  # Polling rápido (5s padrão)
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

from market import find_active_market, refresh_prices, MarketConfig, Market
from analysis import analyze, get_price
from executor import execute, should_trade, print_stats, history

BANNER = """
╔══════════════════════════════════════════════╗
║  POLYMARKET SNIPER BOT (HYBRID MODE)        ║
║  BTC, ETH, XRP | 5m & 15m                   ║
║  Escolha: Polling OU WebSocket             ║
╚══════════════════════════════════════════════╝
"""

MARKETS_TO_TRADE = [
    MarketConfig("BTC", 15, "BTCUSDT"),
    MarketConfig("ETH", 15, "ETHUSDT"),
    MarketConfig("XRP", 15, "XRPUSDT"),
    MarketConfig("SOL", 15, "SOLUSDT"),
    MarketConfig("BTC", 5,  "BTCUSDT"),
    MarketConfig("ETH", 5, "ETHUSDT"),
    MarketConfig("XRP", 5, "XRPUSDT"),
    MarketConfig("SOL", 5, "SOLUSDT"),
]

# Mapeia símbolo Binance -> lista de configs
SYMBOL_TO_CONFIGS: Dict[str, list[MarketConfig]] = defaultdict(list)
for cfg in MARKETS_TO_TRADE:
    SYMBOL_TO_CONFIGS[cfg.binance_symbol].append(cfg)


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


class MarketMonitor:
    """Monitora mercados em ambos os modos (polling ou websocket)."""
    
    def __init__(self, client: ClobClient, dry_run: bool = False):
        self.client = client
        self.dry_run = dry_run
        self.traded_slugs = set()
        self.active_markets: Dict[str, Optional[Market]] = {}
        self.last_process_time: Dict[str, float] = {}
        
        # Cache de preços em tempo real (para modo WebSocket)
        self.realtime_prices: Dict[str, Dict] = {}
        
    def update_price_cache(self, token_id: str, side: str, price: float):
        """Atualiza cache de preços (usado em modo WebSocket)."""
        if token_id not in self.realtime_prices:
            self.realtime_prices[token_id] = {}
        self.realtime_prices[token_id][side] = price
        self.realtime_prices[token_id]["timestamp"] = time.time()
    
    def process_config(self, config: MarketConfig, trigger_reason: str = "event"):
        """Processa um único config - verifica e executa trade se necessário."""
        try:
            config_key = f"{config.asset}_{config.duration_minutes}m"
            now = time.time()
            
            # Rate limiting: não processa o mesmo config mais de 1x por segundo
            if config_key in self.last_process_time:
                if now - self.last_process_time[config_key] < 1.0:
                    return
            self.last_process_time[config_key] = now
            
            # 1. Encontrar mercado
            market = find_active_market(self.client, config)
            if not market:
                return
            
            # 2. Atualizar preços
            # Em modo WebSocket, usa cache se disponível
            if trigger_reason == "websocket" and market.up_token_id in self.realtime_prices:
                up_cache = self.realtime_prices.get(market.up_token_id, {})
                down_cache = self.realtime_prices.get(market.down_token_id, {})
                
                # Usa preço do cache se recente (< 5s)
                if now - up_cache.get("timestamp", 0) < 5:
                    market.up_price = up_cache.get("ask", market.up_price)
                if now - down_cache.get("timestamp", 0) < 5:
                    market.down_price = down_cache.get("ask", market.down_price)
            else:
                market = refresh_prices(self.client, market)
            
            # 3. Verificar se já operamos
            if market.slug in self.traded_slugs:
                return
            
            # 4. Janela de tempo
            mins_left = market.minutes_left
            if mins_left < 0.5 or mins_left > 3.0:
                return
            
            # 5. Verificar se preço está interessante (>= $0.95)
            if market.up_price < 0.95 and market.down_price < 0.95:
                return
            
            # 6. Buscar preço real e analisar
            current_price = get_price(config.binance_symbol)
            if not current_price:
                return
            
            signal = analyze(
                symbol=config.binance_symbol,
                strike_price=market.strike_price if market.strike_price > 0 else current_price,
                minutes_left=mins_left,
            )
            
            if not signal:
                return
            
            # 7. Decidir trade
            ok, reason = should_trade(signal, market, BANKROLL, min_edge=MIN_EDGE)
            
            if not ok:
                return
            
            # 8. Log e execução
            print(f"\n🎯 OPORTUNIDADE DETECTADA ({trigger_reason.upper()})!")
            print(f"📌 {market}")
            print(f"📊 {signal}")
            print(f"✅ {reason}")
            
            success = execute(self.client, market, signal, BANKROLL, dry_run=self.dry_run)
            
            if success:
                self.traded_slugs.add(market.slug)
                # Limita cache de slugs
                if len(self.traded_slugs) > 100:
                    self.traded_slugs = set(list(self.traded_slugs)[-100:])
                    
        except Exception as e:
            print(f"⚠️  Erro ao processar {config.asset} {config.duration_minutes}m: {e}")
    
    def check_all_markets(self, trigger_reason: str = "poll"):
        """Verifica todos os mercados (usado em modo polling)."""
        for config in MARKETS_TO_TRADE:
            self.process_config(config, trigger_reason)


# ════════════════════════════════════════════════════════════════════════════
# MODO POLLING (Padrão)
# ════════════════════════════════════════════════════════════════════════════

def run_polling_mode(dry_run: bool = False):
    """Modo POLLING: Verifica mercados a cada X segundos."""
    print(f"\n🔄 MODO: POLLING")
    print(f"   ➤ Intervalo: {POLL_INTERVAL}s")
    print(f"   ➤ Simples e confiável\n")
    
    client = create_client(dry_run=dry_run)
    monitor = MarketMonitor(client, dry_run=dry_run)
    
    cycle = 0
    while _running:
        cycle += 1
        now = datetime.now().strftime("%H:%M:%S")
        print(f"\n─── Ciclo #{cycle} | {now} " + "─" * 30)
        
        monitor.check_all_markets(trigger_reason="poll")
        
        print_stats()
        time.sleep(POLL_INTERVAL)
    
    print("\n✅ Bot encerrado (modo polling)")
    print_stats()


# ════════════════════════════════════════════════════════════════════════════
# MODO WEBSOCKET (Tempo Real)
# ════════════════════════════════════════════════════════════════════════════

def run_websocket_mode(dry_run: bool = False):
    """Modo WEBSOCKET: Reage instantaneamente a mudanças de preço."""
    print(f"\n⚡ MODO: WEBSOCKET (Tempo Real)")
    print(f"   ➤ Reação instantânea")
    print(f"   ➤ WebSocket Polymarket + Binance\n")
    
    # Verifica dependências
    try:
        import websocket
        from websocket_client import PolymarketWebSocketClient
    except ImportError:
        print("❌ Dependência faltando para modo WebSocket!")
        print("   Instale com: pip install websocket-client")
        print("\n   Ou use modo polling: python hybrid_runner.py --mode polling")
        sys.exit(1)
    
    try:
        from binance import ThreadedWebsocketManager
        BINANCE_WS_AVAILABLE = True
    except ImportError:
        BINANCE_WS_AVAILABLE = False
        print("⚠️  WebSocket Binance desabilitado (python-binance não instalado)")
    
    client = create_client(dry_run=dry_run)
    monitor = MarketMonitor(client, dry_run=dry_run)
    
    # ──────────────────────────────────────────────────────────────────────
    # WebSocket Polymarket
    # ──────────────────────────────────────────────────────────────────────
    
    def on_polymarket_price_update(token_id: str, event_type: str, data: dict):
        """Callback quando há atualização de preço na Polymarket."""
        # Atualiza cache
        if "ask" in data:
            monitor.update_price_cache(token_id, "ask", data["ask"])
        if "bid" in data:
            monitor.update_price_cache(token_id, "bid", data["bid"])
        
        # Dispara verificação dos mercados relacionados a este token
        # (Para otimização, você poderia mapear token_id -> config)
        for config in MARKETS_TO_TRADE:
            monitor.process_config(config, trigger_reason="websocket")
    
    poly_ws = PolymarketWebSocketClient(on_price_update=on_polymarket_price_update)
    
    # Coleta todos os token IDs que queremos monitorar
    # Para isso, precisamos buscar os mercados ativos primeiro
    print("🔍 Buscando mercados ativos para monitorar...")
    tokens_to_monitor = set()
    
    for config in MARKETS_TO_TRADE:
        market = find_active_market(client, config)
        if market:
            tokens_to_monitor.add(market.up_token_id)
            tokens_to_monitor.add(market.down_token_id)
            print(f"   ✓ {config.asset} {config.duration_minutes}m → {len(tokens_to_monitor)} tokens")
    
    if tokens_to_monitor:
        poly_ws.subscribe(list(tokens_to_monitor))
        poly_ws.start()
    else:
        print("⚠️  Nenhum mercado ativo encontrado para monitorar")
    
    # ──────────────────────────────────────────────────────────────────────
    # WebSocket Binance (Opcional)
    # ──────────────────────────────────────────────────────────────────────
    
    binance_ws = None
    if BINANCE_WS_AVAILABLE and BINANCE_API_KEY and BINANCE_API_SECRET:
        print("\n🔌 Conectando WebSocket Binance...")
        
        try:
            twm = ThreadedWebsocketManager(
                api_key=BINANCE_API_KEY,
                api_secret=BINANCE_API_SECRET
            )
            twm.start()
            
            def handle_binance_ticker(msg):
                """Callback para ticker da Binance."""
                if msg.get("e") != "24hrTicker":
                    return
                
                symbol = msg.get("s")
                # Dispara verificação dos configs relacionados a este símbolo
                for config in SYMBOL_TO_CONFIGS.get(symbol, []):
                    monitor.process_config(config, trigger_reason="binance_ws")
            
            # Subscribe para todos os símbolos
            for symbol in SYMBOL_TO_CONFIGS.keys():
                twm.start_symbol_ticker_socket(
                    callback=handle_binance_ticker,
                    symbol=symbol
                )
                print(f"   ✓ Binance WS: {symbol}")
            
            binance_ws = twm
            print("✅ WebSocket Binance conectado\n")
            
        except Exception as e:
            print(f"⚠️  Erro no WebSocket Binance: {e}\n")
    
    # ──────────────────────────────────────────────────────────────────────
    # Polling de backup (verifica novos mercados a cada 30s)
    # ──────────────────────────────────────────────────────────────────────
    
    print("🚀 Bot iniciado em modo WebSocket!")
    print("   - Polymarket WS: Preços em tempo real")
    if binance_ws:
        print("   - Binance WS: Ativo")
    print("   - Polling backup: A cada 30s para novos mercados\n")
    print("Pressione Ctrl+C para encerrar\n")
    
    last_market_check = time.time()
    last_stats_time = time.time()
    
    try:
        while _running:
            # Verifica novos mercados a cada 30s
            if time.time() - last_market_check > 30:
                print("\n🔍 Buscando novos mercados...")
                
                new_tokens = set()
                for config in MARKETS_TO_TRADE:
                    market = find_active_market(client, config)
                    if market:
                        new_tokens.add(market.up_token_id)
                        new_tokens.add(market.down_token_id)
                
                # Adiciona novos tokens ao WebSocket
                current_tokens = poly_ws.subscribed_tokens
                to_add = new_tokens - current_tokens
                
                if to_add:
                    poly_ws.subscribe(list(to_add))
                    print(f"   ➕ {len(to_add)} novos tokens adicionados")
                
                last_market_check = time.time()
            
            # Mostra stats a cada 60s
            if time.time() - last_stats_time > 60:
                print_stats()
                last_stats_time = time.time()
            
            time.sleep(1)
            
    except KeyboardInterrupt:
        pass
    finally:
        # Cleanup
        poly_ws.stop()
        if binance_ws:
            binance_ws.stop()
    
    print("\n✅ Bot encerrado (modo websocket)")
    print_stats()


# ════════════════════════════════════════════════════════════════════════════
# MAIN
# ════════════════════════════════════════════════════════════════════════════

def run(mode: str = "polling", dry_run: bool = False):
    """
    Executa o bot no modo escolhido.
    
    Args:
        mode: "polling" ou "websocket"
        dry_run: Se True, simula sem trades reais
    """
    print(BANNER)
    
    if dry_run:
        print("=" * 48)
        print("  MODO DRY-RUN — nenhum trade real será feito")
        print("=" * 48)
    
    print(f"\nBankroll: ${BANKROLL:.2f} USDC")
    
    if mode.lower() == "websocket":
        run_websocket_mode(dry_run=dry_run)
    else:
        run_polling_mode(dry_run=dry_run)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="Bot Polymarket Híbrido - Escolha entre Polling ou WebSocket",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Exemplos:
  python hybrid_runner.py                      # Polling (padrão)
  python hybrid_runner.py --mode websocket     # WebSocket (tempo real)
  python hybrid_runner.py --dry-run            # Simulação
  python hybrid_runner.py --mode websocket --dry-run
        """
    )
    
    parser.add_argument(
        "--mode",
        choices=["polling", "websocket"],
        default="polling",
        help="Modo de operação (padrão: polling)"
    )
    
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Simula sem trades reais"
    )
    
    args = parser.parse_args()
    
    # Sobrescreve com variável de ambiente se definido
    if USE_WEBSOCKET:
        args.mode = "websocket"
    
    run(mode=args.mode, dry_run=args.dry_run)
