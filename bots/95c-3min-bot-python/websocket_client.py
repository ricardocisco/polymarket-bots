"""
websocket_client.py — Cliente WebSocket para Polymarket

Monitora preços e orderbook em tempo real usando o WebSocket oficial da Polymarket.
Documentação: https://docs.polymarket.com/trading/orderbook#real-time-updates
"""
from __future__ import annotations

import json
import time
import threading
from typing import Callable, Dict, Optional
from collections import defaultdict
import websocket

POLYMARKET_WS_URL = "wss://ws-subscriptions-clob.polymarket.com/ws/market"


class PolymarketWebSocketClient:
    """
    Cliente WebSocket para monitorar preços da Polymarket em tempo real.
    
    Eventos disponíveis:
    - book: Snapshot completo do orderbook
    - price_change: Mudança de preço individual
    - last_trade_price: Novo trade executado
    - best_bid_ask: Mudança no topo do livro (melhor bid/ask)
    - tick_size_change: Mudança no tick size
    - new_market: Novo mercado criado
    - market_resolved: Mercado resolvido
    """
    
    def __init__(self, on_price_update: Optional[Callable] = None):
        """
        Args:
            on_price_update: Callback chamado quando há atualização de preço.
                             Recebe (token_id: str, event_type: str, data: dict)
        """
        self.on_price_update = on_price_update
        self.ws: Optional[websocket.WebSocketApp] = None
        self.running = False
        self.thread: Optional[threading.Thread] = None
        
        # Tokens sendo monitorados
        self.subscribed_tokens: set[str] = set()
        
        # Cache dos melhores preços (bid/ask) por token
        self.best_prices: Dict[str, Dict[str, float]] = defaultdict(dict)
        
        # Lock para thread safety
        self.lock = threading.Lock()
    
    def subscribe(self, token_ids: list[str]):
        """Inscreve-se para receber atualizações de preço de tokens específicos."""
        if not token_ids:
            return
        
        with self.lock:
            for token_id in token_ids:
                self.subscribed_tokens.add(token_id)
        
        # Se já conectado, envia subscribe dinâmico
        if self.ws and self.running:
            self._send_subscribe(token_ids)
    
    def unsubscribe(self, token_ids: list[str]):
        """Remove inscrição de tokens."""
        if not token_ids:
            return
        
        with self.lock:
            for token_id in token_ids:
                self.subscribed_tokens.discard(token_id)
        
        if self.ws and self.running:
            self._send_unsubscribe(token_ids)
    
    def _send_subscribe(self, token_ids: list[str]):
        """Envia mensagem de subscribe para o WebSocket."""
        if not self.ws:
            return
        
        try:
            msg = {
                "assets_ids": token_ids,
                "operation": "subscribe"
            }
            self.ws.send(json.dumps(msg))
            print(f"📥 WebSocket: Subscribed to {len(token_ids)} tokens")
        except Exception as e:
            print(f"❌ Erro ao enviar subscribe: {e}")
    
    def _send_unsubscribe(self, token_ids: list[str]):
        """Envia mensagem de unsubscribe para o WebSocket."""
        if not self.ws:
            return
        
        try:
            msg = {
                "assets_ids": token_ids,
                "operation": "unsubscribe"
            }
            self.ws.send(json.dumps(msg))
            print(f"📤 WebSocket: Unsubscribed from {len(token_ids)} tokens")
        except Exception as e:
            print(f"❌ Erro ao enviar unsubscribe: {e}")
    
    def _on_open(self, ws):
        """Callback quando conexão WebSocket é estabelecida."""
        print("✅ WebSocket Polymarket conectado!")
        
        # Envia mensagem de subscribe inicial se houver tokens
        with self.lock:
            tokens = list(self.subscribed_tokens)
        
        if tokens:
            msg = {
                "type": "market",
                "assets_ids": tokens,
                "custom_feature_enabled": True  # Habilita best_bid_ask, new_market, market_resolved
            }
            ws.send(json.dumps(msg))
            print(f"📥 Subscribed to {len(tokens)} tokens")
    
    def _on_message(self, ws, message):
        """Callback quando mensagem é recebida."""
        try:
            data = json.loads(message)
            event_type = data.get("event_type")
            
            if not event_type:
                return
            
            # Processa diferentes tipos de eventos
            if event_type == "best_bid_ask":
                self._handle_best_bid_ask(data)
            elif event_type == "price_change":
                self._handle_price_change(data)
            elif event_type == "last_trade_price":
                self._handle_last_trade(data)
            elif event_type == "book":
                self._handle_book_snapshot(data)
            elif event_type == "tick_size_change":
                self._handle_tick_size_change(data)
            
        except json.JSONDecodeError:
            pass
        except Exception as e:
            print(f"⚠️  Erro ao processar mensagem WS: {e}")
    
    def _handle_best_bid_ask(self, data: dict):
        """Processa evento best_bid_ask (melhor bid/ask)."""
        asset_id = data.get("asset_id")
        if not asset_id:
            return
        
        best_bid = float(data.get("best_bid", 0))
        best_ask = float(data.get("best_ask", 0))
        
        # Atualiza cache
        with self.lock:
            self.best_prices[asset_id] = {
                "bid": best_bid,
                "ask": best_ask,
                "spread": best_ask - best_bid,
                "mid": (best_bid + best_ask) / 2,
                "timestamp": time.time()
            }
        
        # Chama callback se configurado
        if self.on_price_update:
            try:
                self.on_price_update(asset_id, "best_bid_ask", {
                    "bid": best_bid,
                    "ask": best_ask,
                    "spread": best_ask - best_bid
                })
            except Exception as e:
                print(f"⚠️  Erro no callback on_price_update: {e}")
    
    def _handle_price_change(self, data: dict):
        """Processa evento price_change."""
        asset_id = data.get("asset_id")
        if not asset_id:
            return
        
        price_changes = data.get("price_changes", [])
        
        # Atualiza best bid/ask se disponível
        best_bid = data.get("best_bid")
        best_ask = data.get("best_ask")
        
        if best_bid and best_ask:
            with self.lock:
                self.best_prices[asset_id] = {
                    "bid": float(best_bid),
                    "ask": float(best_ask),
                    "spread": float(best_ask) - float(best_bid),
                    "mid": (float(best_bid) + float(best_ask)) / 2,
                    "timestamp": time.time()
                }
            
            if self.on_price_update:
                try:
                    self.on_price_update(asset_id, "price_change", {
                        "bid": float(best_bid),
                        "ask": float(best_ask)
                    })
                except Exception as e:
                    print(f"⚠️  Erro no callback: {e}")
    
    def _handle_last_trade(self, data: dict):
        """Processa evento last_trade_price."""
        asset_id = data.get("asset_id")
        if not asset_id or not self.on_price_update:
            return
        
        try:
            self.on_price_update(asset_id, "last_trade_price", {
                "price": float(data.get("price", 0)),
                "side": data.get("side"),
                "size": float(data.get("size", 0))
            })
        except Exception as e:
            print(f"⚠️  Erro no callback: {e}")
    
    def _handle_book_snapshot(self, data: dict):
        """Processa evento book (snapshot completo)."""
        asset_id = data.get("asset_id")
        if not asset_id:
            return
        
        bids = data.get("bids", [])
        asks = data.get("asks", [])
        
        if bids and asks:
            best_bid = float(bids[0]["price"]) if bids else 0
            best_ask = float(asks[0]["price"]) if asks else 0
            
            with self.lock:
                self.best_prices[asset_id] = {
                    "bid": best_bid,
                    "ask": best_ask,
                    "spread": best_ask - best_bid,
                    "mid": (best_bid + best_ask) / 2,
                    "timestamp": time.time()
                }
            
            if self.on_price_update:
                try:
                    self.on_price_update(asset_id, "book", {
                        "bid": best_bid,
                        "ask": best_ask,
                        "bids": bids,
                        "asks": asks
                    })
                except Exception as e:
                    print(f"⚠️  Erro no callback: {e}")
    
    def _handle_tick_size_change(self, data: dict):
        """Processa evento tick_size_change - CRÍTICO para bots de trading."""
        print(f"⚠️  TICK SIZE MUDOU: {data}")
        # O tick size muda quando preço > 0.96 ou < 0.04
        # Orders com tick size antigo serão rejeitadas
    
    def _on_error(self, ws, error):
        """Callback quando ocorre erro."""
        print(f"❌ WebSocket Polymarket erro: {error}")
    
    def _on_close(self, ws, close_status_code, close_msg):
        """Callback quando conexão é fechada."""
        print(f"🔌 WebSocket Polymarket desconectado (code={close_status_code})")
        
        # Tenta reconectar se ainda estiver running
        if self.running:
            print("🔄 Tentando reconectar em 5 segundos...")
            time.sleep(5)
            if self.running:
                self._connect()
    
    def _connect(self):
        """Estabelece conexão WebSocket."""
        try:
            self.ws = websocket.WebSocketApp(
                POLYMARKET_WS_URL,
                on_open=self._on_open,
                on_message=self._on_message,
                on_error=self._on_error,
                on_close=self._on_close
            )
            
            # Run forever em thread separada
            self.ws.run_forever()
            
        except Exception as e:
            print(f"❌ Erro ao conectar WebSocket: {e}")
    
    def start(self):
        """Inicia o cliente WebSocket em thread separada."""
        if self.running:
            print("⚠️  WebSocket já está rodando")
            return
        
        self.running = True
        self.thread = threading.Thread(target=self._connect, daemon=True)
        self.thread.start()
        print("🔌 WebSocket Polymarket iniciando...")
    
    def stop(self):
        """Para o cliente WebSocket."""
        self.running = False
        
        if self.ws:
            try:
                self.ws.close()
            except:
                pass
        
        if self.thread:
            self.thread.join(timeout=2)
        
        print("🔌 WebSocket Polymarket parado")
    
    def get_best_price(self, token_id: str, side: str = "BUY") -> Optional[float]:
        """
        Retorna o melhor preço para um token a partir do cache.
        
        Args:
            token_id: ID do token
            side: "BUY" (retorna ask) ou "SELL" (retorna bid)
        
        Returns:
            Preço ou None se não disponível
        """
        with self.lock:
            prices = self.best_prices.get(token_id)
            
            if not prices:
                return None
            
            # Verifica se o cache não está muito antigo (> 60s)
            if time.time() - prices.get("timestamp", 0) > 60:
                return None
            
            if side == "BUY":
                return prices.get("ask")
            else:
                return prices.get("bid")
    
    def get_midpoint(self, token_id: str) -> Optional[float]:
        """Retorna o midpoint (média bid/ask) para um token."""
        with self.lock:
            prices = self.best_prices.get(token_id)
            
            if not prices:
                return None
            
            if time.time() - prices.get("timestamp", 0) > 60:
                return None
            
            return prices.get("mid")
