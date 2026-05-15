"""
trade_logger.py — Logging JSON de trades com PnL real da Polymarket

Fluxo:
  1. log_entry()    → salva entrada com odds do token no momento da compra
  2. resolve_trade() → após mercado fechar, calcula PnL real e atualiza o JSON

PnL real (Polymarket):
  Comprou UP (token YES) a $0.62 com $81.50
  → comprou 81.50 / 0.62 = 131.45 tokens
  → ganhou: recebe 131.45 × $1.00 = $131.45  → PnL = +$49.95 (+61.3%)
  → perdeu: recebe 131.45 × $0.00 = $0.00    → PnL = -$81.50 (-100%)

Suporte a modos de operação:
  - Cada modo (CONSERVATIVE, AGGRESSIVE, DEGEN) tem seu arquivo de log
  - Use set_log_file() para definir qual arquivo usar
"""
from __future__ import annotations

import json
import time
from datetime import datetime, timezone
from pathlib import Path
import logging

logger = logging.getLogger(__name__)

# Arquivo de log atual (pode ser alterado via set_log_file)
_LOG_FILE = Path("trades_log.json")

def set_log_file(filename: str) -> None:
    """
    Define o arquivo de log a ser usado
    
    Args:
        filename: Nome do arquivo (ex: "trades_log_conservative.json")
    """
    global _LOG_FILE
    _LOG_FILE = Path(filename)
    logger.info(f"📁 Arquivo de log definido: {_LOG_FILE}")

def get_log_file() -> Path:
    """Retorna o arquivo de log atual"""
    return _LOG_FILE


# ─────────────────────────────────────────────────────────────────────────────
# Helpers
# ─────────────────────────────────────────────────────────────────────────────

def _now_iso() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def _load() -> dict:
    if _LOG_FILE.exists():
        try:
            with open(_LOG_FILE, "r", encoding="utf-8") as f:
                return json.load(f)
        except (json.JSONDecodeError, OSError):
            pass
    return {"summary": _empty_summary(), "trades": []}


def _empty_summary() -> dict:
    return {
        "total_trades": 0,
        "resolved_count": 0,
        "wins": 0,
        "losses": 0,
        "win_rate_pct": 0.0,
        "bayes_accuracy_pct": 0.0,
        "total_pnl_usd": 0.0,
        "total_wagered_usd": 0.0,
        "roi_pct": 0.0,
        "avg_pnl_per_trade_usd": 0.0,
        "avg_pnl_pct_per_trade": 0.0,
        "best_trade_pnl_usd": 0.0,
        "worst_trade_pnl_usd": 0.0,
        "kelly_avg_fraction": 0.0,
        "kelly_avg_position_usd": 0.0,
        "pending_resolution": 0,
    }


def _save(data: dict) -> None:
    tmp = _LOG_FILE.with_suffix(".tmp")
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2, ensure_ascii=False)
    for attempt in range(5):
        try:
            tmp.replace(_LOG_FILE)
            return
        except PermissionError:
            if attempt < 4:
                time.sleep(0.2)
            else:
                logger.error("Failed to replace %s after 5 attempts", _LOG_FILE)
                raise


def _recalc_summary(data: dict) -> None:
    trades   = data["trades"]
    resolved = [t for t in trades if t.get("resolution") and t["resolution"].get("resolved")]

    wins   = sum(1 for t in resolved if t["resolution"].get("correct_prediction"))
    losses = len(resolved) - wins

    pnls     = [t["resolution"]["pnl_usd"]           for t in resolved if "pnl_usd"    in t["resolution"]]
    pnl_pcts = [t["resolution"]["pnl_pct"]           for t in resolved if "pnl_pct"    in t["resolution"]]
    wagered  = [t["entry"]["kelly"]["position_usd"]  for t in resolved if t.get("entry")]
    all_pos  = [t["entry"]["kelly"]["position_usd"]  for t in trades   if t.get("entry")]
    all_frac = [t["entry"]["kelly"]["fraction"]       for t in trades   if t.get("entry")]

    total_pnl     = sum(pnls)
    total_wagered = sum(wagered)
    roi           = (total_pnl / total_wagered * 100) if total_wagered > 0 else 0.0

    data["summary"] = {
        "total_trades":            len(trades),
        "resolved_count":          len(resolved),
        "wins":                    wins,
        "losses":                  losses,
        "win_rate_pct":            round(wins / len(resolved) * 100, 2) if resolved else 0.0,
        "bayes_accuracy_pct":      round(wins / len(resolved) * 100, 2) if resolved else 0.0,
        "total_pnl_usd":           round(total_pnl, 2),
        "total_wagered_usd":       round(total_wagered, 2),
        "roi_pct":                 round(roi, 2),
        "avg_pnl_per_trade_usd":   round(total_pnl / len(resolved), 2) if resolved else 0.0,
        "avg_pnl_pct_per_trade":   round(sum(pnl_pcts) / len(pnl_pcts), 2) if pnl_pcts else 0.0,
        "best_trade_pnl_usd":      round(max(pnls), 2) if pnls else 0.0,
        "worst_trade_pnl_usd":     round(min(pnls), 2) if pnls else 0.0,
        "kelly_avg_fraction":      round(sum(all_frac) / len(all_frac), 4) if all_frac else 0.0,
        "kelly_avg_position_usd":  round(sum(all_pos)  / len(all_pos),  2) if all_pos  else 0.0,
        "pending_resolution":      len(trades) - len(resolved),
    }


# ─────────────────────────────────────────────────────────────────────────────
# PnL Polymarket
# ─────────────────────────────────────────────────────────────────────────────

def calc_pnl(
    direction: str,
    token_yes_price: float,
    token_no_price: float,
    position_usd: float,
    correct: bool,
) -> tuple[float, float]:
    """
    Retorna (pnl_usd, pnl_pct) baseado nas odds reais do token no momento da entrada.

    Na Polymarket cada token resolve para $1.00 (ganhou) ou $0.00 (perdeu).
    O preço do token é a probabilidade implícita: token_yes=0.62 → mercado acha 62% chance de UP.
    """
    entry_price = token_yes_price if direction == "UP" else token_no_price

    # Rejeita preços fora do range válido do CLOB (0.99/0.01 = Gamma API corrompida)
    # Só deve receber preço real do orderbook — monitors bloqueiam 0.50 antes de chegar aqui
    if entry_price < 0.10 or entry_price > 0.90:
        logger.error(f"calc_pnl: preço inválido {entry_price:.4f} para {direction} — origem: Gamma API (não CLOB)")
        return 0.0, 0.0

    tokens_bought = position_usd / entry_price

    if correct:
        payout  = tokens_bought * 1.0
        pnl_usd = payout - position_usd
        pnl_pct = (pnl_usd / position_usd * 100) if position_usd > 0 else 0.0
    else:
        pnl_usd = -position_usd
        pnl_pct = -100.0

    return round(pnl_usd, 2), round(pnl_pct, 2)


# ─────────────────────────────────────────────────────────────────────────────
# API pública
# ─────────────────────────────────────────────────────────────────────────────

def log_entry(
    slug: str,
    interval_minutes: int,
    symbol: str,
    end_timestamp: int,
    strike_price: float,
    current_price: float,
    price_vs_strike_pct: float,
    # odds Polymarket — preço dos tokens no CLOB no momento da entrada
    token_yes_price: float,
    token_no_price: float,
    market_url: str,
    expires_at: str,
    direction: str,
    prob_up: float,
    prob_down: float,
    confidence_pct: float,
    edge: float,
    rsi_signal: float,
    rsi_value: float,
    ema_signal: float,
    ema5: float,
    ema15: float,
    vol_signal: float,
    vol_ratio: float,
    atr_signal: float,
    atr_value: float,
    kelly_fraction: float,
    kelly_fraction_full: float,
    kelly_position_usd: float,
    bankroll: float,
    price_1m_pct: float,
    price_5m_pct: float,
    price_15m_pct: float,
) -> str:
    """Registra nova entrada. Retorna trade_id."""
    trade_id = f"{symbol}_{interval_minutes}m_{end_timestamp}_{int(time.time())}"

    trade = {
        "trade_id": trade_id,
        "market": {
            "slug":             slug,
            "symbol":           symbol,
            "interval_minutes": interval_minutes,
            "url":              market_url,
            "expires_at":       expires_at,
            "end_timestamp":    end_timestamp,
        },
        "entry": {
            "timestamp": _now_iso(),
            "prices": {
                "strike":           round(strike_price, 4),
                "current":          round(current_price, 4),
                "vs_strike_pct":    round(price_vs_strike_pct, 4),
                "momentum_1m_pct":  round(price_1m_pct, 4),
                "momentum_5m_pct":  round(price_5m_pct, 4),
                "momentum_15m_pct": round(price_15m_pct, 4),
            },
            "odds": {
                "token_yes_price":   round(token_yes_price, 4),
                "token_no_price":    round(token_no_price, 4),
                "implied_prob_up":   round(token_yes_price * 100, 2),
                "implied_prob_down": round(token_no_price * 100, 2),
            },
            "bayes": {
                "direction":      direction,
                "prob_up_pct":    round(prob_up * 100, 2),
                "prob_down_pct":  round(prob_down * 100, 2),
                "confidence_pct": round(confidence_pct, 2),
                "edge":           round(edge, 4),
                "signals": {
                    "rsi":    {"prob": round(rsi_signal, 4), "weight": 0.30, "rsi_value": round(rsi_value, 2)},
                    "ema":    {"prob": round(ema_signal, 4), "weight": 0.35, "ema5": round(ema5, 4), "ema15": round(ema15, 4)},
                    "volume": {"prob": round(vol_signal, 4), "weight": 0.20, "vol_ratio": round(vol_ratio, 2)},
                    "atr":    {"prob": round(atr_signal, 4), "weight": 0.15, "atr_value": round(atr_value, 4)},
                },
            },
            "kelly": {
                "fraction":                 round(kelly_fraction, 4),
                "fraction_full":            round(kelly_fraction_full, 4),
                "position_usd":             round(kelly_position_usd, 2),
                "bankroll_usd":             round(bankroll, 2),
                "position_pct_of_bankroll": round(kelly_position_usd / bankroll * 100, 2) if bankroll > 0 else 0,
            },
        },
        "resolution": None,
    }

    data = _load()
    data["trades"].append(trade)
    _recalc_summary(data)
    _save(data)

    logger.info(
        f"📝 Trade registrado: {trade_id}  {direction}  conf={confidence_pct:.1f}%  "
        f"odds_yes={token_yes_price:.2f}  pos=${kelly_position_usd:.2f}"
    )
    return trade_id


def resolve_trade(
    trade_id: str,
    final_price: float,
    strike_price: float,
    resolved_direction: str,
    resolution_source: str = "binance_kline",
) -> bool:
    """
    Fecha o trade com resultado real + PnL calculado pelas odds salvas na entrada.
    Retorna True se encontrado e atualizado.
    """
    data = _load()

    for trade in data["trades"]:
        if trade["trade_id"] != trade_id:
            continue

        # Já resolvido?
        if trade.get("resolution") and trade["resolution"].get("resolved"):
            logger.warning(f"Trade {trade_id} já foi resolvido anteriormente.")
            return True

        entry     = trade["entry"]
        predicted = entry["bayes"]["direction"]
        correct   = predicted == resolved_direction

        token_yes = entry["odds"]["token_yes_price"]
        token_no  = entry["odds"]["token_no_price"]
        position  = entry["kelly"]["position_usd"]

        pnl_usd, pnl_pct = calc_pnl(
            direction=predicted,
            token_yes_price=token_yes,
            token_no_price=token_no,
            position_usd=position,
            correct=correct,
        )

        price_change_pct = ((final_price - strike_price) / strike_price * 100) if strike_price > 0 else 0.0

        trade["resolution"] = {
            "resolved":            True,
            "resolved_at":         _now_iso(),
            "final_price":         round(final_price, 4),
            "strike_price":        round(strike_price, 4),
            "price_change_pct":    round(price_change_pct, 4),
            "resolved_direction":  resolved_direction,
            "predicted_direction": predicted,
            "correct_prediction":  correct,
            "position_usd":        round(position, 2),
            "pnl_usd":             pnl_usd,
            "pnl_pct":             pnl_pct,
            "resolution_source":   resolution_source,
            "outcome_label":       "✅ WIN" if correct else "❌ LOSS",
        }

        _recalc_summary(data)
        _save(data)

        label    = "✅ WIN" if correct else "❌ LOSS"
        pnl_sign = "+" if pnl_usd >= 0 else ""
        logger.info(
            f"🏁 {label}  {trade_id}  "
            f"Previsto={predicted}  Real={resolved_direction}  "
            f"PnL={pnl_sign}${pnl_usd:.2f} ({pnl_sign}{pnl_pct:.1f}%)"
        )
        return True

    logger.warning(f"⚠️  Trade não encontrado para resolução: {trade_id}")
    return False


def get_pending_trades() -> list[dict]:
    data = _load()
    return [t for t in data["trades"] if not (t.get("resolution") and t["resolution"].get("resolved"))]


def get_summary() -> dict:
    return _load()["summary"]


def print_summary() -> None:
    s   = get_summary()
    G   = "\033[92m"; R = "\033[91m"; Y = "\033[93m"; RST = "\033[0m"; B = "\033[1m"
    wr  = s["win_rate_pct"]
    roi = s["roi_pct"]
    wc  = G if wr >= 55 else Y if wr >= 50 else R
    rc  = G if roi >= 0 else R

    print(f"\n{'='*62}")
    print(f"  {B}📊 RESUMO DO BACKTEST{RST}")
    print(f"{'='*62}")
    print(f"  Total de entradas      : {B}{s['total_trades']}{RST}")
    print(f"  Resolvidos             : {s['resolved_count']}")
    print(f"  Pendentes              : {s['pending_resolution']}")
    print(f"  Vitórias               : {G}{s['wins']}{RST}")
    print(f"  Derrotas               : {R}{s['losses']}{RST}")
    print(f"  Win Rate               : {wc}{B}{wr:.1f}%{RST}")
    print(f"  Acurácia Bayes         : {wc}{s['bayes_accuracy_pct']:.1f}%{RST}")
    print(f"{'─'*62}")
    ps = "+" if s['total_pnl_usd'] >= 0 else ""
    print(f"  PnL Total              : {rc}{B}{ps}${s['total_pnl_usd']:.2f}{RST}")
    print(f"  Total Apostado         : ${s['total_wagered_usd']:.2f}")
    print(f"  ROI                    : {rc}{B}{ps}{roi:.1f}%{RST}")
    print(f"  PnL médio / trade      : {rc}{s['avg_pnl_per_trade_usd']:+.2f} USD  ({s['avg_pnl_pct_per_trade']:+.1f}%){RST}")
    print(f"  Melhor trade           : {G}+${s['best_trade_pnl_usd']:.2f}{RST}")
    print(f"  Pior trade             : {R}${s['worst_trade_pnl_usd']:.2f}{RST}")
    print(f"{'─'*62}")
    print(f"  Kelly médio (fração)   : {s['kelly_avg_fraction']:.4f}  ({s['kelly_avg_fraction']*100:.2f}% bankroll)")
    print(f"  Kelly médio ($)        : ${s['kelly_avg_position_usd']:.2f}")
    print(f"{'='*62}\n")