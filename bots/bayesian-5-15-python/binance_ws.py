"""
Cliente WebSocket para Binance
Monitora preços em tempo real para análise técnica
"""
import json
import asyncio
import logging
from typing import Callable, Dict, List, Optional
from dataclasses import dataclass
from datetime import datetime
from collections import deque
import websockets

from config import BINANCE_WSS_BASE, WS_SETTINGS

logger = logging.getLogger(__name__)


@dataclass
class BinanceKline:
    """Estrutura para dados de candle Binance"""
    symbol: str
    timestamp: datetime
    open: float
    high: float
    low: float
    close: float
    volume: float
    close_time: datetime
    is_closed: bool
    
    def to_dict(self) -> Dict:
        return {
            'symbol': self.symbol,
            'timestamp': self.timestamp,
            'open': self.open,
            'high': self.high,
            'low': self.low,
            'close': self.close,
            'volume': self.volume,
            'close_time': self.close_time,
            'is_closed': self.is_closed
        }


class BinanceWebSocket:
    """
    Cliente WebSocket para Binance
    Monitora preços de criptomoedas em tempo real
    """
    
    def __init__(self, streams: List[str]):
        """
        Args:
            streams: Lista de streams Binance (ex: ["btcusdt@kline_1m"])
        """
        self.streams = streams
        self.ws: Optional[websockets.WebSocketClientProtocol] = None
        self.running = False
        self.callbacks: Dict[str, List[Callable]] = {
            'kline': [],
            'error': []
        }
        self.reconnect_attempts = 0
        
        # Cache de preços recentes por símbolo
        self.price_history: Dict[str, deque] = {}
        self.max_history_size = 500  # Mantém últimos 500 candles
        
        for stream in streams:
            symbol = stream.split('@')[0].upper()
            self.price_history[symbol] = deque(maxlen=self.max_history_size)
    
    def on_kline(self, callback: Callable[[BinanceKline], None]):
        """Registra callback para updates de kline (candle)"""
        self.callbacks['kline'].append(callback)
        return self
    
    def on_error(self, callback: Callable[[Exception], None]):
        """Registra callback para erros"""
        self.callbacks['error'].append(callback)
        return self
    
    async def connect(self):
        """Conecta ao WebSocket da Binance"""
        try:
            # Combina múltiplos streams em uma conexão
            combined_streams = '/'.join(self.streams)
            url = f"{BINANCE_WSS_BASE}/stream?streams={combined_streams}"
            
            logger.info(f"🔌 Conectando ao WebSocket Binance...")
            logger.debug(f"URL: {url}")
            
            self.ws = await websockets.connect(
                url,
                ping_interval=WS_SETTINGS['ping_interval'],
                ping_timeout=WS_SETTINGS['ping_timeout']
            )
            
            self.running = True
            self.reconnect_attempts = 0
            logger.info(f"✅ Conectado! Monitorando {len(self.streams)} streams")
            
        except Exception as e:
            logger.error(f"❌ Erro ao conectar Binance: {e}")
            await self._handle_reconnect()
    
    async def _handle_reconnect(self):
        """Gerencia reconexão automática"""
        if self.reconnect_attempts >= WS_SETTINGS['max_reconnect_attempts']:
            logger.error("❌ Máximo de tentativas de reconexão atingido (Binance)")
            self.running = False
            return
        
        self.reconnect_attempts += 1
        delay = WS_SETTINGS['reconnect_delay'] * self.reconnect_attempts
        
        logger.warning(f"🔄 Reconectando Binance em {delay}s (tentativa {self.reconnect_attempts})...")
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
            logger.warning("⚠️  Conexão Binance fechada")
            if self.running:
                await self._handle_reconnect()
                
        except Exception as e:
            logger.error(f"❌ Erro no listener Binance: {e}")
            await self._trigger_callbacks('error', e)
            if self.running:
                await self._handle_reconnect()
    
    async def _process_message(self, message: str):
        """Processa mensagem recebida do WebSocket"""
        try:
            data = json.loads(message)
            
            # Binance envia dados em formato {"stream": "...", "data": {...}}
            if 'data' in data:
                stream_data = data['data']
                event_type = stream_data.get('e')
                
                if event_type == 'kline':
                    await self._handle_kline(stream_data)
                    
        except json.JSONDecodeError as e:
            logger.error(f"Erro ao parsear JSON Binance: {e}")
        except Exception as e:
            logger.error(f"Erro ao processar mensagem Binance: {e}")
            await self._trigger_callbacks('error', e)
    
    async def _handle_kline(self, data: Dict):
        """Processa dados de kline (candle)"""
        try:
            kline_data = data['k']
            
            symbol = data['s']
            
            kline = BinanceKline(
                symbol=symbol,
                timestamp=datetime.fromtimestamp(kline_data['t'] / 1000),
                open=float(kline_data['o']),
                high=float(kline_data['h']),
                low=float(kline_data['l']),
                close=float(kline_data['c']),
                volume=float(kline_data['v']),
                close_time=datetime.fromtimestamp(kline_data['T'] / 1000),
                is_closed=kline_data['x']
            )
            
            # Adiciona ao histórico
            self.price_history[symbol].append(kline)
            
            # Dispara callbacks
            await self._trigger_callbacks('kline', kline)
            
        except Exception as e:
            logger.error(f"Erro ao processar kline: {e}")
    
    async def _trigger_callbacks(self, event_type: str, data):
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
    
    def get_recent_prices(self, symbol: str, count: int = 100) -> List[BinanceKline]:
        """
        Retorna preços recentes de um símbolo
        
        Args:
            symbol: Símbolo da cripto (ex: "BTCUSDT")
            count: Número de candles a retornar
        """
        symbol = symbol.upper()
        history = list(self.price_history.get(symbol, []))
        return history[-count:] if len(history) > count else history
    
    def get_current_price(self, symbol: str) -> Optional[float]:
        """Retorna último preço conhecido de um símbolo"""
        symbol = symbol.upper()
        history = self.price_history.get(symbol)
        
        if history and len(history) > 0:
            return history[-1].close
        
        return None
    
    def get_price_change(self, symbol: str, minutes: int) -> Optional[float]:
        """
        Calcula mudança de preço em % nos últimos N minutos
        
        Args:
            symbol: Símbolo da cripto
            minutes: Janela de tempo em minutos
        
        Returns:
            Mudança percentual ou None se dados insuficientes
        """
        symbol = symbol.upper()
        history = list(self.price_history.get(symbol, []))
        
        if len(history) < minutes:
            return None
        
        old_price = history[-minutes].close
        current_price = history[-1].close
        
        return ((current_price - old_price) / old_price) * 100
    
    async def close(self):
        """Fecha conexão WebSocket"""
        self.running = False
        if self.ws:
            await self.ws.close()
            logger.info("🔌 WebSocket Binance fechado")
    
    async def start(self):
        """Inicia o WebSocket client"""
        logger.info("🚀 Iniciando Binance WebSocket client...")
        await self.connect()
        await self.listen()
