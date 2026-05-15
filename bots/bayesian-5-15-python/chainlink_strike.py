"""
chainlink_strike.py — Busca o Strike Price (Price to Beat) na Binance

O strike price é o preço do ativo no momento do eventStartTime.
Buscamos via API Binance histórica (klines).
"""
import logging
import requests
from datetime import datetime, timezone
from typing import Optional

logger = logging.getLogger(__name__)


def get_strike_price(asset: str, start_time: datetime, binance_symbol: str) -> float:
    """
    Busca o preço do ativo na Binance no momento do eventStartTime.
    
    Args:
        asset: Nome do ativo (BTC, ETH, SOL, XRP)
        start_time: Timestamp do eventStartTime (momento do "Price to Beat")
        binance_symbol: Símbolo na Binance (BTCUSDT, ETHUSDT, etc)
    
    Returns:
        Strike price ou 0.0 se falhar
    """
    if not start_time:
        logger.warning(f"⚠️  {asset}: start_time não definido, não é possível buscar strike")
        return 0.0
    
    try:
        # Converte para timestamp em milissegundos
        start_ts_ms = int(start_time.timestamp() * 1000)
        
        logger.debug(f"🔍 Buscando strike para {asset} em {start_time.isoformat()}")
        
        # Busca vela de 1 minuto mais próxima do start_time
        # Nota: Para mercados de 5min, o strike é definido no início do período
        # Para mercados de 15min, o strike é definido no início do período
        url = "https://api.binance.com/api/v3/klines"
        params = {
            "symbol": binance_symbol,
            "interval": "1m",
            "startTime": start_ts_ms - 60000,  # 1 min antes
            "endTime": start_ts_ms + 60000,    # 1 min depois
            "limit": 3
        }
        
        response = requests.get(url, params=params, timeout=10)
        
        if response.status_code != 200:
            logger.warning(f"❌ Binance API error {response.status_code} para {binance_symbol}")
            return 0.0
        
        klines = response.json()
        
        if not klines:
            logger.warning(f"⚠️  Nenhuma vela encontrada para {binance_symbol} em {start_time}")
            return 0.0
        
        # Encontra a vela mais próxima do start_time
        best_kline = min(klines, key=lambda k: abs(k[0] - start_ts_ms))
        
        # Formato da vela:
        # [0] = Open time, [1] = Open, [2] = High, [3] = Low, [4] = Close, [5] = Volume, ...
        open_price = float(best_kline[1])
        close_price = float(best_kline[4])
        
        # Para o strike, usamos o preço de ABERTURA da vela
        # (momento exato do início do mercado)
        strike_price = open_price
        
        kline_time = datetime.fromtimestamp(best_kline[0] / 1000, tz=timezone.utc)
        time_diff = abs((kline_time - start_time).total_seconds())
        
        logger.info(
            f"✅ Strike {asset}: ${strike_price:,.4f} "
            f"(diff: {time_diff:.0f}s do eventStartTime)"
        )
        
        return strike_price
        
    except requests.RequestException as e:
        logger.warning(f"❌ Erro de rede ao buscar strike para {asset}: {e}")
        return 0.0
    
    except (KeyError, ValueError, IndexError) as e:
        logger.warning(f"❌ Erro ao parsear resposta da Binance para {asset}: {e}")
        return 0.0
    
    except Exception as e:
        logger.error(f"❌ Erro inesperado ao buscar strike para {asset}: {e}", exc_info=True)
        return 0.0


def update_market_strike(market, binance_symbol: str) -> bool:
    """
    Atualiza o strike price de um mercado em tempo real.
    
    Args:
        market: Objeto ActiveMarket
        binance_symbol: Símbolo na Binance
    
    Returns:
        True se conseguiu atualizar, False caso contrário
    """
    if market.strike_price > 0:
        return True  # Já tem strike
    
    if not market.start_time:
        # Tenta buscar eventStartTime via API
        logger.info(f"🔍 Buscando eventStartTime para {market.slug}...")
        
        try:
            import requests
            resp = requests.get(
                "https://gamma-api.polymarket.com/markets",
                params={"slug": market.slug},
                timeout=5
            )
            
            if resp.ok:
                data = resp.json()
                if data:
                    market_data = data[0] if isinstance(data, list) else data
                    
                    # Tenta extrair eventStartTime
                    start_raw = market_data.get("eventStartTime") or market_data.get("startDate") or ""
                    
                    if start_raw:
                        try:
                            dt = datetime.fromisoformat(start_raw.replace("Z", "+00:00"))
                            if dt.tzinfo is None:
                                dt = dt.replace(tzinfo=timezone.utc)
                            market.start_time = dt
                            logger.info(f"✅ eventStartTime encontrado: {dt.isoformat()}")
                        except Exception as e:
                            logger.warning(f"⚠️  Erro ao parsear eventStartTime: {e}")
        except Exception as e:
            logger.warning(f"⚠️  Erro ao buscar eventStartTime via API: {e}")
        
        if not market.start_time:
            logger.warning(f"⚠️  Market {market.slug} sem start_time, verificando API...")
            return False
    
    # Só busca strike se já passou do eventStartTime
    now = datetime.now(timezone.utc)
    if now < market.start_time:
        seconds_until = (market.start_time - now).total_seconds()
        if seconds_until > 60:  # Só loga se faltar mais de 1 minuto
            logger.debug(f"⏳ Strike será definido em {seconds_until:.0f}s para {market.slug}")
        return False
    
    strike = get_strike_price(market.crypto, market.start_time, binance_symbol)
    
    if strike > 0:
        market.strike_price = strike
        logger.info(f"🎯 Strike atualizado: {market.crypto} {market.interval}m → ${strike:,.4f}")
        return True
    
    return False
