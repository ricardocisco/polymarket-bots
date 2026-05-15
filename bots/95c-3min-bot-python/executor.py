"""
executor.py — Executa trades nos mercados

Decide quanto apostar e em qual lado com base no sinal de análise
e nos preços atuais do mercado Polymarket.
"""
from __future__ import annotations

import json
import os
from datetime import datetime, timezone
from dataclasses import dataclass, asdict

from py_clob_client.client import ClobClient
from py_clob_client.clob_types import MarketOrderArgs, OrderType
from py_clob_client.order_builder.constants import BUY

from market import Market
from analysis import MarketSignal

HISTORY_FILE = "logs/trades.json"

@dataclass
class TradeResult:
    timestamp: str
    market_slug: str
    asset: str              # BTC, ETH, etc.
    side: str               # "UP" ou "DOWN"
    entry_price: float      # preço pago por share
    amount_usdc: float      # total investido
    shares: float
    up_probability: float   # probabilidade estimada na hora da entrada
    rsi: float
    momentum: float
    distance_pct: float
    order_id: str
    status: str             # "open" | "won" | "lost"
    pnl: float = 0.0


class TradeHistory:
    def __init__(self):
        self.trades: list[TradeResult] = []
        os.makedirs("logs", exist_ok=True)
        self._load()

    def _load(self):
        if os.path.exists(HISTORY_FILE):
            try:
                with open(HISTORY_FILE) as f:
                    data = json.load(f)
                    self.trades = [TradeResult(**t) for t in data]
            except:
                pass

    def save(self):
        with open(HISTORY_FILE, "w") as f:
            json.dump([asdict(t) for t in self.trades], f, indent=2)

    def add(self, trade: TradeResult):
        self.trades.append(trade)
        self.save()

    def stats(self) -> dict:
        closed = [t for t in self.trades if t.status in ("won", "lost")]
        wins   = [t for t in closed if t.status == "won"]
        total_pnl = sum(t.pnl for t in closed)
        today = datetime.now().strftime("%Y-%m-%d")
        daily = [t for t in closed if t.timestamp.startswith(today)]
        daily_pnl = sum(t.pnl for t in daily)
        return {
            "total": len(closed),
            "wins": len(wins),
            "losses": len(closed) - len(wins),
            "win_rate": len(wins) / len(closed) if closed else 0,
            "total_pnl": total_pnl,
            "daily_pnl": daily_pnl,
            "open": len([t for t in self.trades if t.status == "open"]),
        }


history = TradeHistory()


def should_trade(
    signal: MarketSignal,
    market: Market,
    bankroll: float,
    min_edge: float = 0.0,
    min_minutes: float = 0.5,
    max_minutes: float = 3.0,
) -> tuple[bool, str]:
    """
    Estratégia SNIPER:
    - Entra faltando pouco tempo (0.5 a 3 min)
    - Só entra se o preço já estiver > $0.95 (vitória quase certa)
    - Valida com análise técnica para evitar reversões de último segundo
    """
    mins = market.minutes_left
    if mins < min_minutes:
        return False, f"Muito perto do fim ({mins:.1f}m < {min_minutes}m)"
    if mins > max_minutes:
        return False, f"Ainda muito cedo ({mins:.1f}m > {max_minutes}m) - Aguardando < 3m"

    if signal.recommended_side == "UP":
        price = market.up_price
        side = "UP"
    elif signal.recommended_side == "DOWN":
        price = market.down_price
        side = "DOWN"
    else:
        # Se o sinal for SKIP, mas o preço estiver > 0.98, confia no preço
        if market.up_price > 0.98:
            price = market.up_price
            side = "UP"
        elif market.down_price > 0.98:
            price = market.down_price
            side = "DOWN"
        else:
            return False, "Sinal neutro e sem dominância clara de preço"

    # Preço Mínimo (Segurança Sniper)
    if price < 0.95:
        return False, f"Preço baixo (${price:.3f} < $0.95) - Risco alto para Sniper"

    if price > 0.995:
        return False, f"Preço teto (${price:.3f}) - Sem lucro possível"

    # Validação Técnica
    if side == "UP" and signal.distance_pct < 0:
        return False, f"PERIGO: Preço UP alto mas estamos ABAIXO do strike ({signal.distance_pct:.2f}%)"
    if side == "DOWN" and signal.distance_pct > 0:
        return False, f"PERIGO: Preço DOWN alto mas estamos ACIMA do strike ({signal.distance_pct:.2f}%)"

    return True, "OK (Sniper Setup)"


def calculate_bet_size(
    bankroll: float,
    probability: float,
    price: float,
    max_pct: float = 0.10,
) -> float:
    """
    Retorna valor FIXO de $2.00 para estratégia Sniper.
    (Ajustável se quiser voltar para Kelly depois)
    """
    # Aposta fixa de $2.00, desde que não exceda a banca
    target_bet = 2.0
    return min(target_bet, bankroll)

    # Lógica antiga (Kelly) comentada para referência:
    # if price <= 0 or price >= 1: return min(bankroll * max_pct, 2.0)
    # b = (1 / price) - 1
    # p = probability
    # q = 1 - p
    # kelly = (p * b - q) / b
    # quarter_kelly = kelly * 0.25
    # amount = quarter_kelly * bankroll
    # return max(1.0, min(amount, bankroll * max_pct))


def execute(
    client: ClobClient,
    market: Market,
    signal: MarketSignal,
    bankroll: float,
    dry_run: bool = False,
) -> bool:
    """Executa o trade."""
    side = signal.recommended_side
    # Fallback side determination logic matches should_trade if signal is SKIP
    if side == "SKIP":
        if market.up_price > 0.95: side = "UP"
        elif market.down_price > 0.95: side = "DOWN"

    if side == "UP":
        token_id = market.up_token_id
        price = market.up_price
        prob = signal.up_probability
    elif side == "DOWN":
        token_id = market.down_token_id
        price = market.down_price
        prob = 1 - signal.up_probability
    else:
        return False

    if not token_id or token_id in ["0", "", None]:
        print(f"   ❌ Trade abortado: token_id inválido ({token_id})")
        return False

    print(f"   ℹ️  Token ID: {token_id}")

    amount = calculate_bet_size(bankroll, prob, price)
    shares = round(amount / price, 4) if price > 0 else 0

    print(f"\n{'[DRY-RUN] ' if dry_run else ''}🎯 Executando trade ({market.config.asset}):")
    print(f"   Lado:    {side}")
    print(f"   Preço:   ${price:.3f}/share")
    print(f"   Valor:   ${amount:.2f} USDC ({shares:.1f} shares)")
    print(f"   RSI:     {signal.rsi:.1f}")
    print(f"   Dist:    {signal.distance_pct:+.2f}% do strike")

    trade_record = TradeResult(
        timestamp=datetime.now(timezone.utc).isoformat(),
        market_slug=market.slug,
        asset=market.config.asset,
        side=side,
        entry_price=price,
        amount_usdc=amount,
        shares=shares,
        up_probability=prob,
        rsi=signal.rsi,
        momentum=signal.price_momentum,
        distance_pct=signal.distance_pct,
        order_id="DRY-RUN" if dry_run else "",
        status="open",
    )

    if dry_run:
        history.add(trade_record)
        print("   ✓ [DRY-RUN] Trade simulado registrado")
        return True

    try:
        order_args = MarketOrderArgs(token_id=token_id, amount=amount, side=BUY)
        signed = client.create_market_order(order_args)
        resp = client.post_order(signed, OrderType.FOK)

        if not resp or resp.get("errorMsg"):
            print(f"   ❌ Order rejeitada: {resp}")
            return False

        order_id = resp.get("orderID", resp.get("id", "unknown"))
        trade_record.order_id = order_id
        history.add(trade_record)
        print(f"   ✅ Order executada! ID: {order_id[:12]}...")
        return True

    except Exception as e:
        print(f"   ❌ Erro ao executar: {e}")
        return False


def print_stats():
    s = history.stats()
    
    # Calcular exposição atual
    open_trades = [t for t in history.trades if t.status == "open"]
    exposure = sum(t.amount_usdc for t in open_trades)
    potential_profit = sum((t.shares * 1.0) - t.amount_usdc for t in open_trades)
    
    print(f"\n📊 STATUS FINANCEIRO")
    print(f"   ➤ Trades Abertos: {len(open_trades)}")
    print(f"   ➤ Exposição Total: ${exposure:.2f}")
    print(f"   ➤ Lucro Potencial (Abertos): ${potential_profit:.2f}")
    print(f"   ➤ Histórico Fechado: {s['wins']} Wins / {s['losses']} Losses")
    print(f"   ➤ PnL Realizado (Histórico): ${s['total_pnl']:+.2f}")
