"""
analysis.py — Preço em tempo real + análise técnica

Busca candles da Binance e calcula indicadores para decidir
se o ativo vai subir ou cair no próximo intervalo.
"""
from __future__ import annotations

import time
from dataclasses import dataclass
from typing import Optional

import requests

BINANCE_API = "https://api.binance.com/api/v3"

@dataclass
class MarketSignal:
    symbol: str
    current_price: float
    strike_price: float
    distance_pct: float      # % de distância do preço atual ao strike
    rsi: float               # 0–100
    macd_signal: float       # positivo = bullish, negativo = bearish
    price_momentum: float    # variação % nos últimos 5 min
    up_probability: float    # probabilidade estimada de fechar acima do strike
    confidence: str          # "high" | "medium" | "low"
    recommended_side: str    # "UP" | "DOWN" | "SKIP"

    def __str__(self):
        return (
            f"{self.symbol}=${self.current_price:,.4f} | strike=${self.strike_price:,.4f} | "
            f"dist={self.distance_pct:+.2f}% | RSI={self.rsi:.1f} | "
            f"mom={self.price_momentum:+.2f}% | "
            f"→ {self.recommended_side} ({self.up_probability:.0%}) [{self.confidence}]"
        )


def get_price(symbol: str = "BTCUSDT") -> Optional[float]:
    """Preço atual via Binance."""
    try:
        resp = requests.get(
            f"{BINANCE_API}/ticker/price",
            params={"symbol": symbol},
            timeout=5
        )
        return float(resp.json()["price"])
    except Exception:
        return None


def get_candles(symbol: str = "BTCUSDT", interval: str = "1m", limit: int = 50) -> list[dict]:
    """
    Busca candles da Binance.
    interval: "1m", "5m", "15m"
    """
    try:
        resp = requests.get(
            f"{BINANCE_API}/klines",
            params={"symbol": symbol, "interval": interval, "limit": limit},
            timeout=8
        )
        raw = resp.json()
        candles = []
        for c in raw:
            candles.append({
                "open":   float(c[1]),
                "high":   float(c[2]),
                "low":    float(c[3]),
                "close":  float(c[4]),
                "volume": float(c[5]),
            })
        return candles
    except Exception:
        return []


def get_historical_price(symbol: str, timestamp: int) -> float:
    """
    Busca o preço de abertura na Binance para um timestamp específico.
    Usado quando o strike não está explícito na descrição do mercado.
    """
    try:
        resp = requests.get(
            f"{BINANCE_API}/klines",
            params={
                "symbol": symbol,
                "interval": "1m",
                "startTime": timestamp * 1000,
                "limit": 1
            },
            timeout=5
        )
        data = resp.json()
        if data and len(data) > 0:
            return float(data[0][1])  # Open price
        else:
            print(f"DEBUG: Nenhum dado retornado para {symbol} TS {timestamp}. Resp: {data}")
    except Exception as e:
        print(f"Erro buscando preço histórico: {e}")
    return 0.0


def _calculate_rsi(closes: list[float], period: int = 14) -> float:
    """RSI simples sem dependências externas."""
    if len(closes) < period + 1:
        return 50.0
    
    gains, losses = [], []
    for i in range(1, period + 1):
        delta = closes[-(period + 1) + i] - closes[-(period + 1) + i - 1]
        if delta > 0:
            gains.append(delta)
            losses.append(0)
        else:
            gains.append(0)
            losses.append(abs(delta))
    
    avg_gain = sum(gains) / period
    avg_loss = sum(losses) / period
    
    if avg_loss == 0:
        return 100.0
    
    rs = avg_gain / avg_loss
    return 100 - (100 / (1 + rs))


def _calculate_macd(closes: list[float]) -> float:
    """
    MACD simplificado: EMA12 - EMA26.
    """
    def ema(data, period):
        if len(data) < period:
            return data[-1] if data else 0
        k = 2 / (period + 1)
        ema_val = sum(data[:period]) / period
        for price in data[period:]:
            ema_val = price * k + ema_val * (1 - k)
        return ema_val
    
    if len(closes) < 26:
        return 0.0
    
    ema12 = ema(closes, 12)
    ema26 = ema(closes, 26)
    return ema12 - ema26


def _price_momentum(closes: list[float], periods: int = 5) -> float:
    """Variação % dos últimos N candles."""
    if len(closes) < periods + 1:
        return 0.0
    old = closes[-(periods + 1)]
    new = closes[-1]
    if old == 0:
        return 0.0
    return ((new - old) / old) * 100


def analyze(symbol: str, strike_price: float, minutes_left: float) -> Optional[MarketSignal]:
    """
    Análise completa: busca candles, calcula indicadores e
    estima probabilidade de UP vs DOWN.
    """
    # Busca candles de 1 min (últimos 50)
    candles = get_candles(symbol, "1m", 50)
    if not candles:
        return None

    closes = [c["close"] for c in candles]
    current_price = closes[-1]

    # Indicadores
    rsi        = _calculate_rsi(closes)
    macd       = _calculate_macd(closes)
    momentum   = _price_momentum(closes, 5)

    # Distância do preço atual ao strike (positivo = estamos acima)
    if strike_price > 0:
        distance_pct = ((current_price - strike_price) / strike_price) * 100
    else:
        distance_pct = 0.0

    # ── Modelo de probabilidade ────────────────────────────────────────────────
    up_prob = 0.50

    # 1. RSI
    if rsi > 80:
        up_prob -= 0.12
    elif rsi > 70:
        up_prob -= 0.06
    elif rsi > 60:
        up_prob += 0.02
    elif rsi < 30:
        up_prob += 0.12
    elif rsi < 40:
        up_prob += 0.06

    # 2. MACD
    if macd > 0.5:
        up_prob += min(0.12, abs(macd) / current_price * 6000)
    elif macd > 0:
        up_prob += 0.04
    elif macd < -0.5:
        up_prob -= min(0.12, abs(macd) / current_price * 6000)
    else:
        up_prob -= 0.04

    # 3. Momentum
    if momentum > 0.1:
        up_prob += 0.06
    elif momentum > 0.02:
        up_prob += 0.03
    elif momentum < -0.1:
        up_prob -= 0.06
    elif momentum < -0.02:
        up_prob -= 0.03

    # 4. Posição relativa ao strike
    speed_needed = abs(distance_pct) / max(minutes_left, 0.5)

    if distance_pct > 0:  # ACIMA do strike
        if speed_needed < 0.02:
            up_prob += 0.15
        elif speed_needed < 0.05:
            up_prob += 0.08
        else:
            up_prob += 0.03
    else:  # ABAIXO do strike
        if speed_needed < 0.02:
            up_prob -= 0.15
        elif speed_needed < 0.05:
            up_prob -= 0.08
        else:
            up_prob -= 0.03

    # 5. Pouco tempo
    if minutes_left < 3 and distance_pct > 0:
        up_prob += 0.05
    elif minutes_left < 3 and distance_pct < 0:
        up_prob -= 0.05

    # Clampar
    up_prob = max(0.05, min(0.95, up_prob))
    down_prob = 1 - up_prob

    # Decisão
    edge = abs(up_prob - 0.5)

    if edge >= 0.15:
        confidence = "high"
    elif edge >= 0.08:
        confidence = "medium"
    else:
        confidence = "low"

    if up_prob >= 0.54:
        recommended_side = "UP"
    elif down_prob >= 0.54:
        recommended_side = "DOWN"
    else:
        recommended_side = "SKIP"

    return MarketSignal(
        symbol=symbol,
        current_price=current_price,
        strike_price=strike_price,
        distance_pct=distance_pct,
        rsi=rsi,
        macd_signal=macd,
        price_momentum=momentum,
        up_probability=up_prob,
        confidence=confidence,
        recommended_side=recommended_side,
    )
