"""
Historical backtest mode for the Bayesian 5/15m bot.

This reuses the same Bayesian model and Kelly validator used by live mode, then
replays closed Polymarket UP/DOWN markets with Binance 1m candles and CLOB
price-history data. It never sends orders and does not require trading keys.

Examples:
  python backtest.py --days 3
  python backtest.py --days 7 --mode AGGRESSIVE_OPTIMIZED --asset BTC --interval 5
  python backtest.py --days 14 --stake 1 --trades
"""
from __future__ import annotations

import argparse
import json
import math
import re
import time
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Iterable, Optional

import requests

from bayesian_model import OptimizedBayesianModel
from binance_ws import BinanceKline
from config import (
    ORDER_BOOK_PARAMS,
    POLYMARKET_CLOB_API,
    POLYMARKET_GAMMA_API,
    TARGET_MARKETS,
    TRADING_MODES,
    set_active_mode,
)
from kelly_criterion import KellyCriterion


@dataclass
class HistoricalMarket:
    slug: str
    symbol: str
    asset: str
    interval: int
    start_ts: int
    end_ts: int
    strike: float
    up_token: str
    down_token: str
    outcome_prices: list[float]


@dataclass
class BacktestTrade:
    slug: str
    asset: str
    interval: int
    entry_ts: int
    direction: str
    entry_price: float
    stake: float
    pnl: float
    won: bool
    confidence: float
    edge: float


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Backtest do Polymarket Bayesian 5/15m bot")
    parser.add_argument("--days", type=int, default=3, help="Janela historica em dias")
    parser.add_argument("--mode", choices=list(TRADING_MODES.keys()), default="AGGRESSIVE_OPTIMIZED")
    parser.add_argument("--bankroll", type=float, default=20.0)
    parser.add_argument("--stake", type=float, default=1.0, help="Stake simulado por entrada")
    parser.add_argument("--asset", choices=list(TARGET_MARKETS.keys()), help="Filtra ativo")
    parser.add_argument("--interval", type=int, choices=[5, 15], help="Filtra intervalo")
    parser.add_argument("--limit", type=int, default=0, help="Limita numero de mercados apos filtros")
    parser.add_argument("--trades", action="store_true", help="Mostra trades individuais")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    set_active_mode(args.mode)

    model = OptimizedBayesianModel()
    kelly = KellyCriterion(args.bankroll)

    markets = list(fetch_closed_markets(args.days, args.asset, args.interval))
    if args.limit > 0:
        markets = markets[: args.limit]

    print("=" * 88)
    print(
        f"Backtest Bayesian 5/15m | days={args.days} | mode={args.mode} | "
        f"asset={args.asset or 'ALL'} | interval={str(args.interval) + 'm' if args.interval else 'ALL'}"
    )
    print("=" * 88)
    print(f"Markets loaded: {len(markets)}")

    trades: list[BacktestTrade] = []
    skip_reasons: dict[str, int] = {}

    for idx, market in enumerate(markets, 1):
        print(f"[{idx}/{len(markets)}] {market.slug}")
        if market.strike <= 0:
            market.strike = fetch_binance_price_at(market.symbol, market.start_ts) or 0.0
        if market.strike <= 0:
            add_skip(skip_reasons, "sem strike")
            continue

        candles = fetch_binance_candles(market.symbol, market.start_ts - 3600, market.end_ts)
        if len(candles) < 35:
            add_skip(skip_reasons, "candles insuficientes")
            continue

        up_history = fetch_clob_price_history(market.up_token, market.start_ts, market.end_ts)
        down_history = fetch_clob_price_history(market.down_token, market.start_ts, market.end_ts)
        if not up_history or not down_history:
            add_skip(skip_reasons, "sem historico CLOB")
            continue

        trade = replay_market(market, candles, up_history, down_history, model, kelly, args.stake)
        if trade is None:
            add_skip(skip_reasons, "sem entrada")
            continue
        trades.append(trade)

    print_report(trades, skip_reasons, show_trades=args.trades)


def fetch_closed_markets(
    days: int,
    asset_filter: Optional[str],
    interval_filter: Optional[int],
) -> Iterable[HistoricalMarket]:
    session = requests.Session()
    session.headers.update({"User-Agent": "bayesian-backtest/1.0"})
    end = datetime.now(timezone.utc)
    start = end - timedelta(days=days)
    window_start = start

    while window_start < end:
        window_end = min(window_start + timedelta(days=1), end)
        cursor = None
        while True:
            params = {
                "closed": "true",
                "limit": 500,
                "start_time_min": window_start.isoformat().replace("+00:00", "Z"),
                "start_time_max": window_end.isoformat().replace("+00:00", "Z"),
            }
            if cursor:
                params["after_cursor"] = cursor
            resp = session.get(f"{POLYMARKET_GAMMA_API}/events/keyset", params=params, timeout=20)
            resp.raise_for_status()
            payload = resp.json()
            events = payload.get("events", [])

            for event in events:
                for raw_market in event.get("markets") or []:
                    parsed = parse_market(raw_market, asset_filter, interval_filter)
                    if parsed:
                        yield parsed

            cursor = payload.get("next_cursor")
            if len(events) < 500 or not cursor:
                break
        window_start = window_end


def parse_market(
    raw: dict,
    asset_filter: Optional[str],
    interval_filter: Optional[int],
) -> Optional[HistoricalMarket]:
    slug = raw.get("slug") or ""
    match = re.match(r"(?P<asset>btc|eth|sol|xrp)-updown-(?P<interval>5|15)m-", slug, re.I)
    if not match:
        return None

    asset = match.group("asset").upper()
    interval = int(match.group("interval"))
    if asset_filter and asset != asset_filter.upper():
        return None
    if interval_filter and interval != interval_filter:
        return None

    tokens = parse_token_ids(raw)
    if len(tokens) < 2:
        return None

    start_ts = parse_ts(raw.get("eventStartTime") or raw.get("startDate")) or parse_slug_ts(slug)
    end_ts = parse_ts(raw.get("endDate"))
    if not start_ts or not end_ts:
        return None

    outcome_prices = parse_float_list(raw.get("outcomePrices"))
    description = raw.get("description") or ""
    question = raw.get("question") or ""
    strike = parse_strike(f"{description} {question}")

    symbol = TARGET_MARKETS[asset]["symbol"]
    return HistoricalMarket(
        slug=slug,
        symbol=symbol,
        asset=asset,
        interval=interval,
        start_ts=start_ts,
        end_ts=end_ts,
        strike=strike,
        up_token=tokens[0],
        down_token=tokens[1],
        outcome_prices=outcome_prices,
    )


def replay_market(
    market: HistoricalMarket,
    candles: list[BinanceKline],
    up_history: list[tuple[int, float]],
    down_history: list[tuple[int, float]],
    model: OptimizedBayesianModel,
    kelly: KellyCriterion,
    stake: float,
) -> Optional[BacktestTrade]:
    min_price = ORDER_BOOK_PARAMS["min_buy_price"]
    max_price = ORDER_BOOK_PARAMS["max_buy_price"]
    latest_entry_ts = market.end_ts - 120

    for idx, candle in enumerate(candles):
        ts = int(candle.close_time.timestamp())
        if ts <= market.start_ts or ts >= latest_entry_ts:
            continue
        if idx < 30:
            continue

        recent = candles[max(0, idx - 99) : idx + 1]
        minutes_left = max((market.end_ts - ts) / 60.0, 0.0)
        prediction = model.predict(
            market.symbol,
            recent,
            strike_price=market.strike,
            current_price=candle.close,
            minutes_to_expiry=minutes_left,
        )
        if not prediction.should_trade():
            continue

        direction = prediction.get_direction()
        selected_history = up_history if direction == "UP" else down_history
        entry_price = nearest_price(selected_history, ts)
        if entry_price is None or not (min_price <= entry_price <= max_price):
            continue

        token_yes_price = entry_price if direction == "UP" else round(1.0 - entry_price, 4)
        token_no_price = entry_price if direction == "DOWN" else round(1.0 - entry_price, 4)
        kelly_result = kelly.calculate(prediction, entry_price, token_yes_price, token_no_price)
        ok, _reason = kelly.validate_bet(kelly_result, current_positions=0)
        if not ok:
            continue

        winner = winner_direction(market, candles)
        won = direction == winner
        pnl = (stake / entry_price - stake) if won else -stake
        return BacktestTrade(
            slug=market.slug,
            asset=market.asset,
            interval=market.interval,
            entry_ts=ts,
            direction=direction,
            entry_price=entry_price,
            stake=stake,
            pnl=pnl,
            won=won,
            confidence=prediction.confidence,
            edge=prediction.edge,
        )

    return None


def winner_direction(market: HistoricalMarket, candles: list[BinanceKline]) -> str:
    if len(market.outcome_prices) >= 2 and market.outcome_prices[0] != market.outcome_prices[1]:
        return "UP" if market.outcome_prices[0] > market.outcome_prices[1] else "DOWN"
    final_price = candles[-1].close if candles else market.strike
    return "UP" if final_price > market.strike else "DOWN"


def fetch_binance_candles(symbol: str, start_ts: int, end_ts: int) -> list[BinanceKline]:
    out: list[BinanceKline] = []
    cursor = start_ts
    while cursor < end_ts:
        params = {
            "symbol": symbol,
            "interval": "1m",
            "startTime": cursor * 1000,
            "endTime": end_ts * 1000,
            "limit": 1000,
        }
        resp = requests.get("https://api.binance.com/api/v3/klines", params=params, timeout=10)
        resp.raise_for_status()
        rows = resp.json()
        if not rows:
            break
        for row in rows:
            open_ts = int(row[0] / 1000)
            close_ts = int(row[6] / 1000)
            out.append(
                BinanceKline(
                    symbol=symbol,
                    timestamp=datetime.fromtimestamp(open_ts, timezone.utc),
                    open=float(row[1]),
                    high=float(row[2]),
                    low=float(row[3]),
                    close=float(row[4]),
                    volume=float(row[5]),
                    close_time=datetime.fromtimestamp(close_ts, timezone.utc),
                    is_closed=True,
                )
            )
        cursor = int(rows[-1][6] / 1000) + 1
        if len(rows) < 1000:
            break
    return out


def fetch_binance_price_at(symbol: str, ts: int) -> Optional[float]:
    params = {"symbol": symbol, "interval": "1m", "startTime": ts * 1000, "limit": 1}
    resp = requests.get("https://api.binance.com/api/v3/klines", params=params, timeout=10)
    if not resp.ok:
        return None
    rows = resp.json()
    return float(rows[0][1]) if rows else None


def fetch_clob_price_history(token_id: str, start_ts: int, end_ts: int) -> list[tuple[int, float]]:
    params = {"market": token_id, "startTs": start_ts, "endTs": end_ts, "fidelity": 1}
    for attempt in range(3):
        try:
            resp = requests.get(f"{POLYMARKET_CLOB_API}/prices-history", params=params, timeout=20)
            resp.raise_for_status()
            return [
                (int(point["t"]), float(point["p"]))
                for point in resp.json().get("history", [])
                if "t" in point and "p" in point
            ]
        except requests.RequestException:
            if attempt == 2:
                return []
            time.sleep(0.4 * (attempt + 1))
    return []


def nearest_price(points: list[tuple[int, float]], ts: int) -> Optional[float]:
    if not points:
        return None
    best = min(points, key=lambda point: abs(point[0] - ts))
    if abs(best[0] - ts) > 90:
        return None
    return best[1]


def parse_token_ids(raw: dict) -> list[str]:
    value = raw.get("clobTokenIds") or raw.get("tokens") or []
    if isinstance(value, str):
        try:
            value = json.loads(value)
        except json.JSONDecodeError:
            return []
    out = []
    for item in value:
        if isinstance(item, str):
            out.append(item)
        elif isinstance(item, dict):
            token = item.get("token_id") or item.get("tokenId")
            if token:
                out.append(str(token))
    return out


def parse_float_list(value) -> list[float]:
    if isinstance(value, str):
        try:
            value = json.loads(value)
        except json.JSONDecodeError:
            return []
    if not isinstance(value, list):
        return []
    out = []
    for item in value:
        try:
            out.append(float(item))
        except (TypeError, ValueError):
            pass
    return out


def parse_ts(value: Optional[str]) -> Optional[int]:
    if not value:
        return None
    try:
        return int(datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp())
    except ValueError:
        return None


def parse_slug_ts(slug: str) -> Optional[int]:
    try:
        return int(slug.rsplit("-", 1)[-1])
    except ValueError:
        return None


def parse_strike(text: str) -> float:
    matches = re.findall(r"\$[\d,]+(?:\.\d+)?", text)
    if matches:
        return float(matches[-1].replace("$", "").replace(",", ""))
    matches = re.findall(r"(?i)(?:Strike|Price|Reference|Beat)[:\s]+([\d,]+(?:\.\d+)?)", text)
    if matches:
        return float(matches[-1].replace(",", ""))
    return 0.0


def add_skip(skip_reasons: dict[str, int], reason: str) -> None:
    skip_reasons[reason] = skip_reasons.get(reason, 0) + 1


def print_report(
    trades: list[BacktestTrade],
    skip_reasons: dict[str, int],
    show_trades: bool,
) -> None:
    total = len(trades)
    wins = sum(1 for trade in trades if trade.won)
    stake = sum(trade.stake for trade in trades)
    pnl = sum(trade.pnl for trade in trades)
    win_rate = wins / total * 100 if total else 0.0
    roi = pnl / stake * 100 if stake else 0.0

    print("-" * 88)
    print(f"Trades     : {total}")
    print(f"Wins       : {wins}")
    print(f"Win rate   : {win_rate:.2f}%")
    print(f"Stake      : ${stake:.2f}")
    print(f"PnL        : {pnl:+.2f}")
    print(f"ROI        : {roi:+.2f}%")
    print("-" * 88)
    print("Skip reasons:")
    for reason, count in sorted(skip_reasons.items(), key=lambda item: item[1], reverse=True):
        print(f"  {count:>5}  {reason}")

    if show_trades:
        print("-" * 88)
        print(f"{'Asset':<7} {'Int':<5} {'Dir':<5} {'Entry':>7} {'Conf':>7} {'Edge':>7} {'PnL':>8}  Slug")
        for trade in trades:
            print(
                f"{trade.asset:<7} {str(trade.interval) + 'm':<5} {trade.direction:<5} "
                f"{trade.entry_price:>7.3f} {trade.confidence:>7.2%} {trade.edge:>7.2%} "
                f"{trade.pnl:>+8.2f}  {trade.slug}"
            )
    print("=" * 88)


if __name__ == "__main__":
    main()
