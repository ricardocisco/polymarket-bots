"""
market.py — Encontra mercados ativos na Polymarket

Suporta BTC, ETH, XRP em janelas de 5m e 15m.
"""
from __future__ import annotations

from datetime import datetime, timezone, timedelta
from dataclasses import dataclass
from typing import Optional
import json
import re

import requests
from py_clob_client.client import ClobClient

GAMMA_URL = "https://gamma-api.polymarket.com/markets"

@dataclass
class MarketConfig:
    asset: str            # "BTC", "ETH"
    duration_minutes: int # 15, 5
    binance_symbol: str   # "BTCUSDT"
    
    @property
    def slug_prefix(self) -> str:
        return f"{self.asset.lower()}-updown-{self.duration_minutes}m"

@dataclass
class Market:
    slug: str
    condition_id: str
    question: str
    up_token_id: str
    down_token_id: str
    strike_price: float
    end_time: datetime
    up_price: float = 0.0
    down_price: float = 0.0
    config: MarketConfig = None # Reference to config
    
    @property
    def minutes_left(self) -> float:
        now = datetime.now(timezone.utc)
        return max(0, (self.end_time - now).total_seconds() / 60)
    
    def __str__(self):
        return (
            f"{self.config.asset} {self.config.duration_minutes}M | "
            f"strike=${self.strike_price:,.4f} | "
            f"UP={self.up_price:.3f} DOWN={self.down_price:.3f} | "
            f"{self.minutes_left:.1f}min restantes"
        )


def _get_interval_timestamps(duration_minutes: int) -> list[int]:
    """
    Gera timestamps para o início dos intervalos.
    Arredonda para baixo para o múltiplo de 'duration_minutes'.
    """
    now = datetime.now(timezone.utc)
    # Arredonda minute
    minute_floored = (now.minute // duration_minutes) * duration_minutes
    current_start = now.replace(minute=minute_floored, second=0, microsecond=0)
    
    candidates = []
    # Tenta anterior, atual e próximos
    for offset in [-1, 0, 1, 2]:
        t = current_start + timedelta(minutes=offset * duration_minutes)
        candidates.append(int(t.timestamp()))
    
    return candidates


def _parse_strike(market_data: dict) -> float:
    """Extrai strike da descrição ou pergunta."""
    text = (market_data.get("description") or "") + " " + (market_data.get("question") or "")
    
    # Padrão: $67,029.48
    matches = re.findall(r'\$[\d,]+(?:\.\d+)?', text)
    if matches:
        try:
            return float(matches[-1].replace('$', '').replace(',', ''))
        except:
            pass
            
    # Padrão: Strike: 67029.48
    matches_clean = re.findall(r'(?:Strike|Price|Reference|Beat)[:\s]+([\d,]+(?:\.\d+)?)', text, re.IGNORECASE)
    if matches_clean:
         try:
            return float(matches_clean[-1].replace(',', ''))
         except:
            pass
    return 0.0


def _get_token_price(client: ClobClient, token_id: str) -> float:
    try:
        price = client.get_price(token_id, side="BUY")
        if price and price.get("price"):
            return float(price["price"])
        mid = client.get_midpoint(token_id)
        if mid and mid.get("mid"):
            return float(mid["mid"])
    except:
        pass
    return 0.5


def find_active_market(client: ClobClient, config: MarketConfig) -> Optional[Market]:
    """Busca mercado ativo para uma configuração específica."""
    timestamps = _get_interval_timestamps(config.duration_minutes)
    
    for ts in timestamps:
        slug = f"{config.slug_prefix}-{ts}"
        try:
            # print(f"  → Testando slug: {slug}")
            resp = requests.get(GAMMA_URL, params={"slug": slug}, timeout=5)
            if not resp.ok: continue
            
            data = resp.json()
            if not data: continue
            
            market_data = data[0] if isinstance(data, list) else data
            
            # Validações básicas
            if not market_data.get("active") and not market_data.get("closed") is False:
                continue

            # Tokens
            tokens = market_data.get("tokens") or market_data.get("clobTokenIds") or []
            if isinstance(tokens, str):
                try: tokens = json.loads(tokens)
                except: continue
                
            if len(tokens) < 2: continue
            
            up_token = tokens[0] if isinstance(tokens[0], str) else tokens[0].get("token_id", "")
            down_token = tokens[1] if isinstance(tokens[1], str) else tokens[1].get("token_id", "")
            
            if not up_token or not down_token: continue

            # Data fim
            end_date_str = market_data.get("endDate") or market_data.get("end_date_iso", "")
            if not end_date_str: continue
            end_time = datetime.fromisoformat(end_date_str.replace("Z", "+00:00"))
            
            minutes_left = (end_time - datetime.now(timezone.utc)).total_seconds() / 60
            if minutes_left < 2: # Janela mínima
                # print(f"    ❌ {slug} muito perto do fim ({minutes_left:.1f}m)")
                continue

            # Strike
            strike = _parse_strike(market_data)
            
            # Fallback histórico
            if strike <= 0:
                ts_int = int(slug.split("-")[-1])
                # print(f"    ⚠️ Strike histórico {config.asset} TS {ts_int}")
                from analysis import get_historical_price
                strike = get_historical_price(config.binance_symbol, ts_int)
            
            if strike <= 0: continue

            # Preços
            up_price = _get_token_price(client, up_token)
            down_price = _get_token_price(client, down_token)
            
            print(f"    ✅ Encontrado: {slug} | Strike: {strike}")
            
            return Market(
                slug=slug,
                condition_id=market_data.get("conditionId", ""),
                question=market_data.get("question", ""),
                up_token_id=up_token,
                down_token_id=down_token,
                strike_price=strike,
                end_time=end_time,
                up_price=up_price,
                down_price=down_price,
                config=config
            )
            
        except Exception:
            continue
            
    return None


def refresh_prices(client: ClobClient, market: Market) -> Market:
    market.up_price = _get_token_price(client, market.up_token_id)
    market.down_price = _get_token_price(client, market.down_token_id)
    return market
