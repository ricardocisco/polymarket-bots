"""
Cliente WebSocket para Polymarket CLOB API
"""
import json
import asyncio
import logging
from typing import Callable, Dict, List, Optional, Any
from dataclasses import dataclass
from datetime import datetime
import websockets
from websockets.client import WebSocketClientProtocol

from config import POLYMARKET_WSS, WS_SETTINGS

logger = logging.getLogger(__name__)


@dataclass
class MarketData:
    """Estrutura para dados de mercado"""
    asset_id: str
    timestamp: datetime
    price: Optional[float] = None
    bid: Optional[float] = None
    ask: Optional[float] = None
    spread: Optional[float] = None
    last_trade_price: Optional[float] = None
    volume: Optional[float] = None
    event_type: Optional[str] = None
    raw_data: Optional[Dict] = None


class PolymarketWebSocket:
    """
    Cliente WebSocket para Polymarket
    Recebe atualizações em tempo real de mercados
    """
    
    def __init__(self, asset_ids: List[str]):
        self.asset_ids = asset_ids
        self.ws: Optional[WebSocketClientProtocol] = None
        self.running = False
        self.callbacks: Dict[str, List[Callable]] = {
            'book': [],
            'price_change': [],
            'last_trade_price': [],
            'tick_size_change': [],
            'best_bid_ask': [],   # evento de top-of-book (requer custom_feature_enabled)
            'error': []
        }
        self.reconnect_attempts = 0
        self.message_queue = asyncio.Queue(maxsize=WS_SETTINGS['message_queue_size'])

    def on_best_bid_ask(self, callback: Callable[[List['MarketData']], None]):
        """Registra callback para top-of-book (best_bid_ask event)"""
        self.callbacks.setdefault('best_bid_ask', []).append(callback)
        return self
        
    def on_book(self, callback: Callable[[List[MarketData]], None]):
        """Registra callback para updates de orderbook"""
        self.callbacks['book'].append(callback)
        return self
    
    def on_price_change(self, callback: Callable[[List[MarketData]], None]):
        """Registra callback para mudanças de preço"""
        self.callbacks['price_change'].append(callback)
        return self
    
    def on_last_trade(self, callback: Callable[[List[MarketData]], None]):
        """Registra callback para último trade"""
        self.callbacks['last_trade_price'].append(callback)
        return self
    
    def on_error(self, callback: Callable[[Exception], None]):
        """Registra callback para erros"""
        self.callbacks['error'].append(callback)
        return self
    
    async def connect(self):
        """Conecta ao WebSocket da Polymarket"""
        try:
            logger.info(f"🔌 Conectando ao WebSocket Polymarket...")
            self.ws = await websockets.connect(
                POLYMARKET_WSS,
                ping_interval=WS_SETTINGS['ping_interval'],
                ping_timeout=WS_SETTINGS['ping_timeout']
            )
            
            # Subscreve aos mercados
            await self._subscribe()
            
            self.running = True
            self.reconnect_attempts = 0
            logger.info(f"✅ Conectado! Monitorando {len(self.asset_ids)} mercados")
            
        except Exception as e:
            logger.error(f"❌ Erro ao conectar: {e}")
            await self._handle_reconnect()
    
    async def _subscribe(self):
        """Subscreve aos asset IDs especificados"""
        if not self.ws:
            return
        
        # Formato de subscrição conforme doc oficial:
        # https://docs.polymarket.com/trading/orderbook#connecting
        # custom_feature_enabled=true habilita eventos extras:
        #   best_bid_ask   — top-of-book changes (best_bid, best_ask, spread)
        #   new_market     — novo mercado criado
        #   market_resolved — mercado resolvido
        subscribe_message = {
            "type": "market",
            "assets_ids": self.asset_ids,
            "custom_feature_enabled": True,
        }

        await self.ws.send(json.dumps(subscribe_message))
        logger.info(f"📡 Subscrito a {len(self.asset_ids)} mercados (custom_feature_enabled=True)")
    
    async def _handle_reconnect(self):
        """Gerencia reconexão automática"""
        if self.reconnect_attempts >= WS_SETTINGS['max_reconnect_attempts']:
            logger.error("❌ Máximo de tentativas de reconexão atingido")
            self.running = False
            return
        
        self.reconnect_attempts += 1
        delay = WS_SETTINGS['reconnect_delay'] * self.reconnect_attempts
        
        logger.warning(f"🔄 Reconectando em {delay}s (tentativa {self.reconnect_attempts})...")
        await asyncio.sleep(delay)
        
        await self.connect()
    
    async def listen(self):
        """Loop principal de escuta de mensagens"""
        if not self.ws:
            await self.connect()
        
        try:
            async for message in self.ws:
                await self._process_message(message)
                
        except websockets.exceptions.ConnectionClosed:
            logger.warning("⚠️  Conexão fechada")
            if self.running:
                await self._handle_reconnect()
                
        except Exception as e:
            logger.error(f"❌ Erro no listener: {e}")
            await self._trigger_callbacks('error', e)
            if self.running:
                await self._handle_reconnect()
    
    async def _process_message(self, message: str):
        """Processa mensagem recebida do WebSocket"""
        try:
            data = json.loads(message)
            
            # Verifica tipo de evento
            if isinstance(data, dict):
                event_type = data.get('event_type', data.get('type'))
                
                if event_type == 'book':
                    await self._handle_book_update(data)
                elif event_type == 'price_change':
                    await self._handle_price_change(data)
                elif event_type == 'last_trade_price':
                    await self._handle_last_trade(data)
                elif event_type == 'tick_size_change':
                    # Evento normal a cada novo ciclo de mercado (0.01 → 0.001)
                    # Só é relevante em live trading com ordens abertas
                    old = data.get('old_tick_size', '?')
                    new = data.get('new_tick_size', '?')
                    logger.debug(f"tick_size_change: {old} → {new} | {str(data.get('asset_id',''))[:20]}...")
                elif event_type == 'best_bid_ask':
                    await self._handle_best_bid_ask(data)
                elif event_type in ('new_market', 'market_resolved'):
                    logger.info(f"📣 Evento de mercado ({event_type}): {data.get('asset_id', data.get('winning_asset_id', '?'))}")
                else:
                    logger.debug(f"Evento desconhecido: {event_type}")
                    
        except json.JSONDecodeError as e:
            logger.error(f"Erro ao parsear JSON: {e}")
        except Exception as e:
            logger.error(f"Erro ao processar mensagem: {e}")
            await self._trigger_callbacks('error', e)
    
    async def _handle_book_update(self, data: Dict):
        """Processa update de orderbook"""
        try:
            market_updates = []
            
            if 'asset_id' in data:
                # Single update
                market_updates.append(self._parse_book_data(data))
            elif 'assets' in data:
                # Multiple updates
                for asset_data in data['assets']:
                    market_updates.append(self._parse_book_data(asset_data))
            
            if market_updates:
                await self._trigger_callbacks('book', market_updates)
                
        except Exception as e:
            logger.error(f"Erro ao processar book update: {e}")
    
    async def _handle_price_change(self, data: Dict):
        """Processa mudança de preço"""
        try:
            market_data = MarketData(
                asset_id=data.get('asset_id', ''),
                timestamp=datetime.utcnow(),
                price=float(data.get('price', 0)),
                event_type='price_change',
                raw_data=data
            )
            
            await self._trigger_callbacks('price_change', [market_data])
            
        except Exception as e:
            logger.error(f"Erro ao processar price change: {e}")
    
    async def _handle_last_trade(self, data: Dict):
        """Processa último trade"""
        try:
            market_data = MarketData(
                asset_id=data.get('asset_id', ''),
                timestamp=datetime.utcnow(),
                last_trade_price=float(data.get('price', 0)),
                event_type='last_trade_price',
                raw_data=data
            )

            await self._trigger_callbacks('last_trade_price', [market_data])

        except Exception as e:
            logger.error(f"Erro ao processar last trade: {e}")

    async def _handle_best_bid_ask(self, data: Dict):
        """
        Processa evento best_bid_ask (top-of-book).
        Requer custom_feature_enabled=true na subscription.
        doc: https://docs.polymarket.com/trading/orderbook#event-types
        Payload: { asset_id, best_bid, best_ask, spread }
        """
        try:
            best_bid_raw = data.get('best_bid')
            best_ask_raw = data.get('best_ask')
            if best_bid_raw is None or best_ask_raw is None:
                return

            market_data = MarketData(
                asset_id=data.get('asset_id', ''),
                timestamp=datetime.utcnow(),
                bid=float(best_bid_raw),
                ask=float(best_ask_raw),
                spread=float(data.get('spread', 0)),
                price=(float(best_bid_raw) + float(best_ask_raw)) / 2,
                event_type='best_bid_ask',
                raw_data=data,
            )

            # Dispara callbacks de book (reutiliza o mesmo canal)
            await self._trigger_callbacks('best_bid_ask', [market_data])
            await self._trigger_callbacks('book', [market_data])

        except Exception as e:
            logger.error(f"Erro ao processar best_bid_ask: {e}")

    def _parse_book_data(self, data: Dict) -> MarketData:
        """Parse dados do orderbook (evento 'book' com snapshot completo)"""
        asset_id = data.get('asset_id', '')

        # Extrai melhor bid e ask
        # O WS retorna bids/asks como lista de dicts {"price": ..., "size": ...}
        # conforme o mesmo formato da REST API (doc oficial)
        bids = data.get('bids', [])
        asks = data.get('asks', [])

        def _price(entry) -> Optional[float]:
            if not entry:
                return None
            if isinstance(entry, dict):
                return float(entry.get('price', 0)) or None
            # fallback: lista [price, size]
            return float(entry[0]) if entry else None

        best_bid = _price(bids[0]) if bids else None
        best_ask = _price(asks[0]) if asks else None

        # REST e WS da Polymarket retornam:
        #   asks: ordem DECRESCENTE (0.99, 0.98, 0.97...) → melhor ask (mais barato) = asks[-1]
        #   bids: ordem CRESCENTE  (0.01, 0.02, 0.03...) → melhor bid (mais alto)   = bids[-1]
        if asks:
            best_ask = _price(asks[-1])   # menor ask = mais barato para comprar
        if bids:
            best_bid = _price(bids[-1])   # maior bid = melhor para vender

        spread = (best_ask - best_bid) if (best_bid is not None and best_ask is not None) else None
        
        # Calcula preço mid
        price = None
        if best_bid and best_ask:
            price = (best_bid + best_ask) / 2
        
        return MarketData(
            asset_id=asset_id,
            timestamp=datetime.utcnow(),
            price=price,
            bid=best_bid,
            ask=best_ask,
            spread=spread,
            event_type='book',
            raw_data=data
        )
    
    async def _trigger_callbacks(self, event_type: str, data: Any):
        """Dispara callbacks registrados"""
        callbacks = self.callbacks.get(event_type, [])
        
        for callback in callbacks:
            try:
                if asyncio.iscoroutinefunction(callback):
                    await callback(data)
                else:
                    callback(data)
            except Exception as e:
                logger.error(f"Erro no callback {event_type}: {e}")
    
    async def close(self):
        """Fecha conexão WebSocket"""
        self.running = False
        if self.ws:
            await self.ws.close()
            logger.info("🔌 WebSocket fechado")
    
    def add_asset_ids(self, asset_ids: List[str]):
        """Adiciona novos asset IDs para monitorar"""
        new_ids = [aid for aid in asset_ids if aid not in self.asset_ids]
        self.asset_ids.extend(new_ids)
        logger.info(f"➕ Adicionados {len(new_ids)} novos mercados")
    
    async def start(self):
        """Inicia o WebSocket client"""
        logger.info("🚀 Iniciando Polymarket WebSocket client...")
        await self.connect()
        await self.listen()
