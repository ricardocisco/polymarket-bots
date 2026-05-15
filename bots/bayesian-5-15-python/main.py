"""
Script principal para executar o bot de trading Polymarket
Suporta 3 modos: CONSERVATIVE, AGGRESSIVE, DEGEN
"""
import asyncio
import argparse
import logging
import os
import sys
from datetime import datetime

from config import (
    LOG_CONFIG,
    validate_config,
    EXECUTION_PARAMS,
    TRADING_MODES,
    set_active_mode,
    get_log_file,
    get_mode_info,
    ORDER_BOOK_PARAMS,
)
from trading_bot import TradingBot
from trade_logger import set_log_file as set_trade_log_file


def setup_logging():
    """Configura sistema de logging"""
    # Forca UTF-8 no stdout para suportar emojis no Windows (codepage CP1252)
    if hasattr(sys.stdout, 'reconfigure'):
        sys.stdout.reconfigure(encoding='utf-8')

    handlers = [logging.FileHandler(LOG_CONFIG['file'], encoding='utf-8')]
    if LOG_CONFIG['console']:
        import io
        utf8_stdout = io.TextIOWrapper(sys.stdout.buffer, encoding='utf-8', errors='replace')
        handlers.append(logging.StreamHandler(utf8_stdout))

    logging.basicConfig(
        level=getattr(logging, LOG_CONFIG['level']),
        format=LOG_CONFIG['format'],
        handlers=handlers,
    )

    # Silencia logs de bibliotecas externas
    logging.getLogger('websockets').setLevel(logging.WARNING)
    logging.getLogger('urllib3').setLevel(logging.WARNING)


def print_banner(dry_run: bool = False):
    """Imprime banner do bot"""
    banner = """
    ╔══════════════════════════════════════════════════════════╗
    ║                                                          ║
    ║        🤖 POLYMARKET BAYESIAN TRADING BOT 🎯            ║
    ║                                                          ║
    ║        Teorema de Bayes + Critério de Kelly             ║
    ║        WebSocket: Polymarket + Binance                   ║
    ║                                                          ║
    ╚══════════════════════════════════════════════════════════╝
    """
    print(banner)
    print(f"    Versão: 1.0.0")
    print(f"    Data: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    print(f"    Modo: 💰 Live Trading")
    if dry_run:
        print("    Modo efetivo: Dry-run")
    print("    " + "="*54)
    print()


def select_trading_mode():
    """Permite selecionar o modo de trading"""
    print("\n" + "="*60)
    print("  🎯 SELECIONE O MODO DE OPERAÇÃO")
    print("="*60 + "\n")
    
    modes = list(TRADING_MODES.keys())
    for idx, mode_key in enumerate(modes, 1):
        mode = TRADING_MODES[mode_key]
        print(f"  [{idx}] {mode['name']}")
        print(f"      {mode['description']}")
        print(f"      Min Confiança: {mode['bayesian']['min_confidence']*100:.0f}%  |  "
              f"Kelly: {mode['kelly']['kelly_fraction']*100:.0f}%  |  "
              f"Max/Trade: {mode['kelly']['max_bankroll_per_trade']*100:.0f}%")
        print(f"      Log: {mode['log_file']}")
        print()
    
    while True:
        try:
            choice = input(f"Escolha o modo [1-{len(modes)}]: ").strip()
            if not choice:
                print("❌ Escolha inválida. Tente novamente.")
                continue
            
            idx = int(choice) - 1
            if 0 <= idx < len(modes):
                selected_mode = modes[idx]
                mode_info = TRADING_MODES[selected_mode]
                print(f"\n✅ Modo selecionado: {mode_info['name']}")
                return selected_mode
            else:
                print("❌ Número inválido. Escolha 1, 2 ou 3.")
        except ValueError:
            print("❌ Digite apenas números.")
        except KeyboardInterrupt:
            print("\n\n👋 Operação cancelada")
            sys.exit(0)
def select_bankroll() -> float:
    """Pergunta a banca inicial ao usuário — sem valor padrão"""
    print("\n" + "="*60)
    print("  💰 QUAL É A SUA BANCA? (BANKROLL)")
    print("="*60 + "\n")

    while True:
        try:
            raw = input("  Bankroll em USD: $").strip()
            value = float(raw.replace(',', '.').replace('$', ''))
            if value < 5.0:
                print("  \u274c Mínimo de $5.00 para operar.")
                continue
            print(f"  \u2705 Banca: ${value:.2f}")
            return value
        except ValueError:
            print("  \u274c Digite um número válido (ex: 38 ou 38.50)")
        except KeyboardInterrupt:
            print("\n\n\U0001f44b Operação cancelada")
            sys.exit(0)


def _parse_bool(value: str | None, default: bool = False) -> bool:
    if value is None:
        return default
    return value.strip().lower() in {"1", "true", "yes", "on"}


def _parse_bankroll_value(value: str) -> float:
    parsed = float(value.replace(',', '.').replace('$', '').strip())
    if parsed < 5.0:
        raise argparse.ArgumentTypeError("bankroll minimo e 5.0")
    return parsed


def _env_bankroll() -> float | None:
    for name in ("BAYESIAN_BANKROLL", "BANKROLL"):
        value = os.getenv(name)
        if value:
            return _parse_bankroll_value(value)
    return None


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    default_mode = os.getenv("BAYESIAN_MODE") or os.getenv("TRADING_MODE")
    default_dry_run = _parse_bool(os.getenv("BAYESIAN_DRY_RUN", os.getenv("DRY_RUN")), False)

    parser = argparse.ArgumentParser(description="Polymarket Bayesian Trading Bot")
    parser.add_argument(
        "--mode",
        choices=list(TRADING_MODES.keys()),
        default=default_mode,
        help="Modo de trading. Se omitido em terminal interativo, pergunta no console.",
    )
    parser.add_argument(
        "--bankroll",
        type=_parse_bankroll_value,
        default=_env_bankroll(),
        help="Banca inicial em USD. Se omitida em terminal interativo, pergunta no console.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        default=default_dry_run,
        help="Simula trades sem enviar ordens reais.",
    )
    parser.add_argument(
        "--live",
        action="store_true",
        help="Forca live trading mesmo se DRY_RUN/BAYESIAN_DRY_RUN estiver true.",
    )
    args = parser.parse_args(argv)
    if args.mode and args.mode not in TRADING_MODES:
        parser.error(f"--mode invalido: {args.mode}")
    return args


def resolve_trading_mode(configured_mode: str | None) -> str:
    if configured_mode:
        mode_info = TRADING_MODES[configured_mode]
        print(f"\nModo selecionado: {mode_info['name']} (configurado)")
        return configured_mode
    if sys.stdin.isatty():
        return select_trading_mode()

    default_mode = "AGGRESSIVE_OPTIMIZED"
    mode_info = TRADING_MODES[default_mode]
    print(f"\nModo selecionado: {mode_info['name']} (padrao nao interativo)")
    return default_mode


def resolve_bankroll(configured_bankroll: float | None) -> float:
    if configured_bankroll is not None:
        print(f"  Banca: ${configured_bankroll:.2f} (configurada)")
        return configured_bankroll
    if sys.stdin.isatty():
        return select_bankroll()

    default_bankroll = 20.0
    print(f"  Banca: ${default_bankroll:.2f} (padrao nao interativo)")
    return default_bankroll


async def main(argv: list[str] | None = None):
    """Funcao principal."""
    args = parse_args(argv)
    dry_run = False if args.live else bool(args.dry_run)
    os.environ["DRY_RUN"] = "true" if dry_run else "false"
    # Setup
    print_banner(dry_run=dry_run)
    
    # Seleciona modo de operação
    mode = resolve_trading_mode(args.mode)
    
    # Configura modo
    if not set_active_mode(mode):
        sys.exit(1)
    
    # Configura arquivo de log
    log_file = get_log_file()
    log_dir = os.getenv("LOG_DIR")
    if log_dir:
        os.makedirs(log_dir, exist_ok=True)
        LOG_CONFIG['file'] = os.path.join(log_dir, os.path.basename(LOG_CONFIG['file']))
        log_file = os.path.join(log_dir, os.path.basename(log_file))
    set_trade_log_file(log_file)
    
    mode_info = get_mode_info()
    print(f"\n{'='*60}")
    print(f"  🎯 Modo Ativo: {mode_info['name']}")
    print(f"  📝 {mode_info['description']}")
    print(f"  📁 Arquivo de Log: {log_file}")
    print(f"  {'='*60}\n")

    # Pergunta a banca ao usuário (sem default — usuário define)
    from config import MONITOR_PARAMS
    initial_bankroll = resolve_bankroll(args.bankroll)
    MONITOR_PARAMS['initial_bankroll'] = initial_bankroll

    setup_logging()
    
    logger = logging.getLogger(__name__)
    logger.info("🚀 Iniciando bot...")
    
    # Valida configurações
    if not validate_config():
        logger.error("❌ Configurações inválidas!")
        sys.exit(1)
    
    # Cria e executa bot
    try:
        # Informações sobre faixa de compra
        min_p = ORDER_BOOK_PARAMS['min_buy_price']
        max_p = ORDER_BOOK_PARAMS['max_buy_price']
        print(f"\n  🎯 Faixa de compra: {min_p:.2f}c – {max_p:.2f}c no order book")
        print(f"     (bot só entra se o best ask do token estiver nessa faixa)")

        bot = TradingBot(initial_bankroll=initial_bankroll, dry_run=dry_run)

        logger.info(f"🎯 Modo: {mode_info['name']}")
        logger.info(
            f"📊 Faixa de compra: {min_p:.2f} – {max_p:.2f} (order book)"
        )
        if dry_run:
            logger.warning("🧪 DRY-RUN — nenhuma ordem real será enviada para a Polymarket.")
        else:
            logger.warning("⚠️  LIVE TRADING — ordens reais serão enviadas para a Polymarket!")
        
        # Executa bot
        await bot.run()
        
    except KeyboardInterrupt:
        logger.info("\n👋 Bot encerrado pelo usuário")
        
    except Exception as e:
        logger.error(f"❌ Erro fatal: {e}", exc_info=True)
        sys.exit(1)


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("\n👋 Até logo!")
