"""
view_trades.py — Visualiza trades_log.json no terminal

Uso:
    python view_trades.py                  # tudo
    python view_trades.py --pending        # só pendentes
    python view_trades.py --resolved       # só resolvidos
    python view_trades.py --summary        # só métricas
    python view_trades.py --last 10        # últimos N
    python view_trades.py --symbol BTC     # filtra símbolo
    python view_trades.py --wins           # só vitórias
    python view_trades.py --losses         # só derrotas
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

LOG_FILE = Path("trades_log.json")

G = "\033[92m"; R = "\033[91m"; Y = "\033[93m"
C = "\033[96m"; DIM = "\033[2m"; RST = "\033[0m"; BOLD = "\033[1m"


def load():
    if not LOG_FILE.exists():
        print(f"{Y}⚠️  trades_log.json não encontrado. Rode o monitor primeiro.{RST}")
        sys.exit(0)
    with open(LOG_FILE, "r", encoding="utf-8") as f:
        return json.load(f)


def print_summary(data: dict) -> None:
    s  = data["summary"]
    wr = s["win_rate_pct"]
    ro = s["roi_pct"]
    wc = G if wr >= 55 else Y if wr >= 50 else R
    rc = G if ro >= 0 else R
    ps = "+" if ro >= 0 else ""

    print(f"\n{'═'*62}")
    print(f"  {BOLD}📊 RESUMO DO BACKTEST{RST}")
    print(f"{'═'*62}")
    print(f"  Total de entradas      : {BOLD}{s['total_trades']}{RST}")
    print(f"  Resolvidos             : {s['resolved_count']}")
    print(f"  Pendentes              : {s['pending_resolution']}")
    print(f"  Vitórias               : {G}{s['wins']}{RST}")
    print(f"  Derrotas               : {R}{s['losses']}{RST}")
    print(f"  Win Rate               : {wc}{BOLD}{wr:.1f}%{RST}")
    print(f"  Acurácia Bayes         : {wc}{s['bayes_accuracy_pct']:.1f}%{RST}")
    print(f"{'─'*62}")
    pps = "+" if s['total_pnl_usd'] >= 0 else ""
    print(f"  PnL Total              : {rc}{BOLD}{pps}${s['total_pnl_usd']:.2f}{RST}")
    print(f"  Total Apostado         : ${s['total_wagered_usd']:.2f}")
    print(f"  ROI                    : {rc}{BOLD}{ps}{ro:.1f}%{RST}")
    print(f"  PnL médio / trade      : {rc}{s['avg_pnl_per_trade_usd']:+.2f} USD  ({s['avg_pnl_pct_per_trade']:+.1f}%){RST}")
    print(f"  Melhor trade           : {G}+${s['best_trade_pnl_usd']:.2f}{RST}")
    print(f"  Pior trade             : {R}${s['worst_trade_pnl_usd']:.2f}{RST}")
    print(f"{'─'*62}")
    print(f"  Kelly médio (fração)   : {s['kelly_avg_fraction']:.4f}  ({s['kelly_avg_fraction']*100:.2f}% bankroll)")
    print(f"  Kelly médio ($)        : ${s['kelly_avg_position_usd']:.2f}")
    print(f"{'═'*62}\n")


def print_trade(t: dict, index: int) -> None:
    m = t["market"]
    e = t["entry"]
    r = t.get("resolution")

    # Status
    if r and r.get("resolved"):
        correct = r["correct_prediction"]
        status  = f"{G}✅ WIN{RST}" if correct else f"{R}❌ LOSS{RST}"
    else:
        status = f"{Y}⏳ PENDENTE{RST}"

    direction = e["bayes"]["direction"]
    dc = G if direction == "UP" else R
    confidence = e["bayes"]["confidence_pct"]
    edge = e["bayes"]["edge"]

    # Odds salvas
    odds      = e.get("odds", {})
    yes_price = odds.get("token_yes_price", 0.5)
    no_price  = odds.get("token_no_price", 0.5)

    print(f"\n  ┌── #{index:03d}  {C}{m['symbol']} {m['interval_minutes']}min{RST}  {status}")
    print(f"  │  Trade ID    : {DIM}{t['trade_id']}{RST}")
    print(f"  │  Mercado     : {DIM}{m['url']}{RST}")
    print(f"  │  Entrada em  : {e['timestamp']}")
    print(f"  │  Expira em   : {m['expires_at']}")
    print(f"  │")

    # Preços
    p = e["prices"]
    print(f"  │  Strike      : ${p['strike']:,.4f}")
    print(f"  │  Preço entry : ${p['current']:,.4f}  "
          f"(vs strike: {G if p['vs_strike_pct']>=0 else R}{p['vs_strike_pct']:+.4f}%{RST})")
    print(f"  │  Momentum    : 1m={p['momentum_1m_pct']:+.3f}%  "
          f"5m={p['momentum_5m_pct']:+.3f}%  15m={p['momentum_15m_pct']:+.3f}%")
    print(f"  │")

    # Odds
    print(f"  │  Odds CLOB   : YES={Y}{yes_price:.2f}{RST} ({yes_price*100:.0f}%)  "
          f"NO={Y}{no_price:.2f}{RST} ({no_price*100:.0f}%)")
    print(f"  │")

    # Bayes
    b = e["bayes"]
    print(f"  │  {BOLD}Previsão Bayes{RST}")
    print(f"  │  Direção     : {dc}{BOLD}{direction}{RST}  "
          f"(conf={confidence:.1f}%  edge={edge:+.3f})")
    print(f"  │  P(UP)       : {b['prob_up_pct']:.1f}%   P(DOWN): {b['prob_down_pct']:.1f}%")
    print(f"  │")
    print(f"  │  {BOLD}Sinais{RST}")
    sigs = b["signals"]
    print(f"  │  RSI    : {sigs['rsi']['prob']*100:.1f}%  (RSI={sigs['rsi']['rsi_value']:.1f})")
    print(f"  │  EMA    : {sigs['ema']['prob']*100:.1f}%  "
          f"(EMA5={sigs['ema']['ema5']:.2f}  EMA15={sigs['ema']['ema15']:.2f})")
    print(f"  │  Volume : {sigs['volume']['prob']*100:.1f}%  (vol_ratio={sigs['volume']['vol_ratio']:.2f}x)")
    print(f"  │  ATR    : {sigs['atr']['prob']*100:.1f}%  (ATR={sigs['atr']['atr_value']:.4f})")
    print(f"  │")

    # Kelly
    kl = e["kelly"]
    entry_price = yes_price if direction == "UP" else no_price
    tokens = kl['position_usd'] / entry_price if entry_price > 0 else 0
    pnl_win_exp  = tokens * 1.0 - kl['position_usd']
    print(f"  │  {BOLD}Kelly{RST}")
    print(f"  │  Posição     : ${kl['position_usd']:.2f}  ({kl['position_pct_of_bankroll']:.2f}% bankroll)")
    print(f"  │  Fração      : f*={kl['fraction_full']:.4f} × 0.25 = {kl['fraction']:.4f}")
    print(f"  │  Tokens      : {tokens:.4f} tokens @ ${entry_price:.2f}")
    print(f"  │  PnL esperado: {G}+${pnl_win_exp:.2f} se WIN{RST}  /  {R}-${kl['position_usd']:.2f} se LOSS{RST}")

    # Resolução
    if r and r.get("resolved"):
        print(f"  │")
        correct = r["correct_prediction"]
        rc = G if correct else R
        pnl     = r.get("pnl_usd", 0)
        pnl_pct = r.get("pnl_pct", 0)
        pnl_c   = G if pnl >= 0 else R
        print(f"  │  {BOLD}Resolução{RST}")
        print(f"  │  Preço final : ${r['final_price']:,.4f}  "
              f"(var: {G if r['price_change_pct']>=0 else R}{r['price_change_pct']:+.4f}%{RST})")
        print(f"  │  Resultado   : {rc}{BOLD}{r['resolved_direction']}{RST}  "
              f"(previsto: {direction})")
        print(f"  │  Correto?    : {rc}{BOLD}{r['outcome_label']}{RST}")
        print(f"  │  PnL         : {pnl_c}{BOLD}{'+' if pnl>=0 else ''}${pnl:.2f}  "
              f"({'+' if pnl_pct>=0 else ''}{pnl_pct:.1f}%){RST}")
        print(f"  │  Fonte       : {DIM}{r['resolution_source']}{RST}")
    else:
        print(f"  │  {Y}⏳ Aguardando resolução...{RST}")

    print(f"  └{'─'*62}┘")


def main() -> None:
    args = sys.argv[1:]
    data = load()
    trades = data["trades"]

    show_only_pending  = "--pending"  in args
    show_only_resolved = "--resolved" in args
    show_only_wins     = "--wins"     in args
    show_only_losses   = "--losses"   in args
    show_summary_only  = "--summary"  in args
    last_n = None
    symbol_filter = None

    if "--last" in args:
        idx = args.index("--last")
        if idx + 1 < len(args):
            last_n = int(args[idx + 1])

    if "--symbol" in args:
        idx = args.index("--symbol")
        if idx + 1 < len(args):
            symbol_filter = args[idx + 1].upper()

    print_summary(data)

    if show_summary_only:
        return

    # Aplica filtros
    if show_only_pending:
        trades = [t for t in trades if not (t.get("resolution") and t["resolution"].get("resolved"))]
    elif show_only_resolved:
        trades = [t for t in trades if t.get("resolution") and t["resolution"].get("resolved")]

    if show_only_wins:
        trades = [t for t in trades if t.get("resolution") and t["resolution"].get("correct_prediction")]
    elif show_only_losses:
        trades = [t for t in trades if t.get("resolution") and t["resolution"].get("correct_prediction") is False]

    if symbol_filter:
        trades = [t for t in trades if t["market"]["symbol"].upper() == symbol_filter]

    if last_n:
        trades = trades[-last_n:]

    if not trades:
        print(f"  {Y}Nenhum trade encontrado com os filtros aplicados.{RST}")
        return

    print(f"  {BOLD}Exibindo {len(trades)} trade(s):{RST}")
    for i, t in enumerate(trades, 1):
        print_trade(t, i)

    print(f"\n  Total exibido: {len(trades)} trade(s)\n")


if __name__ == "__main__":
    main()