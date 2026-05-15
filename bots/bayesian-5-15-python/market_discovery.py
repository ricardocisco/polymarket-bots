"""
Market Discovery - Encontra mercados ativos na Polymarket
Lógica baseada no market.py original que funciona.
Strike price buscado APÓS encontrar o mercado (não bloqueia a busca).
"""
from __future__ import annotations

import re
import json
import logging
import requests
from dataclasses import dataclass, field
from datetime import datetime, timezone, timedelta
from typing import Dict, List, Optional

from config import TARGET_MARKETS

logger = logging.getLogger(__name__)

GAMMA_URL = "https://gamma-api.polymarket.com/markets"



# ─── Dataclasses ──────────────────────────────────────────────────────────────

@dataclass
class MarketConfig:
    asset: str
    duration_minutes: int
    binance_symbol: str

    @property
    def slug_prefix(self) -> str:
        return f"{self.asset.lower()}-updown-{self.duration_minutes}m"


@dataclass
class ActiveMarket:
    slug: str
    condition_id: str
    question: str
    token_id_yes: str
    token_id_no: str
    strike_price: float       # 0.0 até ser preenchido pelo Chainlink
    end_time: datetime
    start_time: Optional[datetime]  # eventStartTime = momento do "Price to Beat"
    crypto: str
    interval: int
    up_price: float = 0.0
    down_price: float = 0.0

    @property
    def minutes_left(self) -> float:
        return max(0.0, (self.end_time - datetime.now(timezone.utc)).total_seconds() / 60)

    def is_expired(self) -> bool:
        return datetime.now(timezone.utc) >= self.end_time

    def time_until_expiry(self) -> timedelta:
        return max(timedelta(0), self.end_time - datetime.now(timezone.utc))

    def __str__(self) -> str:
        strike = f"${self.strike_price:,.4f}" if self.strike_price > 0 else "strike pendente"
        return (
            f"{self.crypto} {self.interval}m | {strike} | "
            f"{self.minutes_left:.1f}min restantes | {self.slug}"
        )


# ─── Helpers (portados do market.py original) ─────────────────────────────────

def _get_interval_timestamps(duration_minutes: int) -> List[int]:
    now = datetime.now(timezone.utc)
    minute_floored = (now.minute // duration_minutes) * duration_minutes
    current_start = now.replace(minute=minute_floored, second=0, microsecond=0)
    return [
        int((current_start + timedelta(minutes=o * duration_minutes)).timestamp())
        for o in [-1, 0, 1, 2]
    ]


def _extract_tokens(market_data: dict):
    tokens = (
        market_data.get("tokens")
        or market_data.get("clobTokenIds")
        or market_data.get("clob_token_ids")
        or []
    )
    if isinstance(tokens, str):
        try:
            tokens = json.loads(tokens)
        except Exception:
            return None, None
    if len(tokens) < 2:
        return None, None
    t0, t1 = tokens[0], tokens[1]
    up   = t0 if isinstance(t0, str) else t0.get("token_id", "")
    down = t1 if isinstance(t1, str) else t1.get("token_id", "")
    return (up or None), (down or None)


def _fetch_market_details(slug: str, session: requests.Session) -> Optional[dict]:
    """Busca detalhes completos do mercado, incluindo eventStartTime"""
    try:
        # Tenta API Gamma com mais parâmetros
        resp = session.get(
            GAMMA_URL,
            params={"slug": slug, "archived": "false"},
            timeout=5
        )
        
        if resp.ok:
            data = resp.json()
            if data:
                return data[0] if isinstance(data, list) else data
    except Exception as e:
        logger.debug(f"Erro ao buscar detalhes de {slug}: {e}")
    
    return None


def _parse_dt(raw: str) -> Optional[datetime]:
    if not raw:
        return None
    try:
        dt = datetime.fromisoformat(raw.replace("Z", "+00:00"))
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        return dt
    except Exception:
        return None


def _parse_strike_from_text(market_data: dict) -> float:
    """Tenta extrair strike da descrição/questão — geralmente não vem."""
    text = (market_data.get("description") or "") + " " + (market_data.get("question") or "")
    matches = re.findall(r'\$[\d,]+(?:\.\d+)?', text)
    if matches:
        try:
            return float(matches[-1].replace('$', '').replace(',', ''))
        except Exception:
            pass
    return 0.0


def _fetch_strike(asset: str, start_time: Optional[datetime], binance_symbol: str) -> float:
    """Busca strike via Chainlink → fallback Binance. Nunca bloqueia."""
    if not start_time:
        return 0.0
    try:
        from chainlink_strike import get_strike_price
        return get_strike_price(asset, start_time, binance_symbol)
    except Exception as e:
        logger.debug(f"_fetch_strike {asset}: {e}")
        return 0.0


# ─── Busca de mercado individual ──────────────────────────────────────────────

def _find_single_market(
    config: MarketConfig,
    session: requests.Session,
    min_minutes_left: float = 2.0,
) -> Optional[ActiveMarket]:
    """
    1. Gera slugs candidatos por timestamp
    2. Valida: active, tokens, end_time — SEM depender do strike
    3. Retorna o mercado; strike é preenchido depois
    """
    for ts in _get_interval_timestamps(config.duration_minutes):
        slug = f"{config.slug_prefix}-{ts}"
        logger.debug(f"  → {slug}")

        try:
            resp = session.get(GAMMA_URL, params={"slug": slug}, timeout=5)
            if not resp.ok:
                continue

            data = resp.json()
            if not data:
                continue

            m = data[0] if isinstance(data, list) else data

            # ── Status (lógica exata do market.py original) ──────────────
            is_active = m.get("active")
            is_closed = m.get("closed")
            if is_active is False and is_closed is not False:
                continue

            # ── Tokens ──────────────────────────────────────────────────
            up_token, down_token = _extract_tokens(m)
            if not up_token or not down_token:
                continue

            # ── Data de fim ──────────────────────────────────────────────
            end_time = _parse_dt(m.get("endDate") or m.get("end_date_iso") or "")
            if not end_time:
                continue

            minutes_left = (end_time - datetime.now(timezone.utc)).total_seconds() / 60
            if minutes_left < min_minutes_left:
                logger.debug(f"    Pulado: {minutes_left:.1f}min restantes")
                continue

            # ── eventStartTime = "Price to Beat" timestamp ───────────────
            start_raw  = m.get("eventStartTime") or m.get("startDate") or ""
            start_time = _parse_dt(start_raw)
            
            if start_time:
                logger.debug(f"    eventStartTime: {start_time.isoformat()}")
            else:
                logger.warning(f"    ⚠️  Sem eventStartTime para {slug}")

            # ── Strike: tenta texto, depois Chainlink/Binance ─────────────
            strike = _parse_strike_from_text(m)
            if strike <= 0:
                strike = _fetch_strike(config.asset, start_time, config.binance_symbol)

            logger.info(
                f"  ✅ {slug} | strike={'$' + f'{strike:,.4f}' if strike > 0 else 'pendente'}"
                f" | {minutes_left:.1f}min"
            )

            return ActiveMarket(
                slug=slug,
                condition_id=m.get("conditionId", ""),
                question=m.get("question", ""),
                token_id_yes=up_token,
                token_id_no=down_token,
                strike_price=strike,
                end_time=end_time,
                start_time=start_time,
                crypto=config.asset,
                interval=config.duration_minutes,
            )

        except Exception as e:
            logger.debug(f"    Erro {slug}: {e}")
            continue

    return None


# ─── Classe principal ─────────────────────────────────────────────────────────

class MarketDiscovery:

    def __init__(self):
        self.active_markets: Dict[str, ActiveMarket] = {}
        self.last_refresh: Optional[datetime] = None
        self.refresh_interval: int = 55

        self._session = requests.Session()
        self._session.headers.update({
            "User-Agent": "PolymarketBot/1.0",
            "Accept": "application/json",
        })

        self._configs: List[MarketConfig] = [
            MarketConfig(
                asset=crypto,
                duration_minutes=interval,
                binance_symbol=cfg["symbol"],
            )
            for crypto, cfg in TARGET_MARKETS.items()
            for interval in cfg.get("intervals", [15])
        ]

    async def discover_all_markets(self) -> Dict[str, ActiveMarket]:
        logger.info("🔍 Descobrindo mercados UP/DOWN ativos...")
        found: Dict[str, ActiveMarket] = {}

        for cfg in self._configs:
            key    = f"{cfg.asset}_{cfg.duration_minutes}m"
            market = _find_single_market(cfg, self._session)
            if market:
                found[key] = market
            else:
                logger.warning(f"  ⚠️  {cfg.asset} {cfg.duration_minutes}min — não encontrado")

        self.active_markets = found
        self.last_refresh   = datetime.now(timezone.utc)
        logger.info(f"📊 {len(found)}/{len(self._configs)} mercados encontrados")
        return found

    def should_refresh(self) -> bool:
        if not self.last_refresh:
            return True
        return (
            datetime.now(timezone.utc) - self.last_refresh
        ).total_seconds() >= self.refresh_interval

    async def refresh_if_needed(self):
        if self.should_refresh():
            await self.discover_all_markets()

    def get_active_market(self, crypto: str, interval: int) -> Optional[ActiveMarket]:
        m = self.active_markets.get(f"{crypto}_{interval}m")
        if m and m.is_expired():
            return None
        return m

    def get_all_token_ids(self) -> List[str]:
        return [
            tid
            for m in self.active_markets.values()
            if not m.is_expired()
            for tid in [m.token_id_yes, m.token_id_no]
        ]

    def print_active_markets(self):
        if not self.active_markets:
            print("  ❌ Nenhum mercado ativo")
            return
        print("\n" + "═" * 80)
        print(f"  📊 MERCADOS ATIVOS ({len(self.active_markets)})")
        print("═" * 80)
        for key, m in sorted(self.active_markets.items()):
            mins = int(m.minutes_left)
            secs = int(m.time_until_expiry().total_seconds() % 60)
            strike_str = f"${m.strike_price:,.4f}" if m.strike_price > 0 else "pendente"
            print(f"\n  {m.crypto} {m.interval}min  [{mins}m{secs:02d}s restantes]")
            print(f"    Slug      : {m.slug}")
            print(f"    Strike    : {strike_str}")
            print(f"    eventStart: {m.start_time}")
            print(f"    UP token  : {m.token_id_yes[:24]}...")
            print(f"    DOWN token: {m.token_id_no[:24]}...")
        print("═" * 80 + "\n")