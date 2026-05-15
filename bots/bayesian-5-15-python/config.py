"""
Configurações do Bot de Trading Polymarket
"""
import os
from typing import Dict, List

# Carrega variáveis do arquivo .env (se existir)
try:
    from dotenv import load_dotenv
    load_dotenv()
except ImportError:
    pass  # python-dotenv não instalado — usa apenas variáveis de ambiente do sistema

# ==================== CONFIGURAÇÕES GERAIS ====================
BOT_NAME = "PolymarketBayesKellyBot"
VERSION = "1.0.0"

# ==================== CREDENCIAIS ====================
# IMPORTANTE: Definir estas variáveis de ambiente antes de executar
PRIVATE_KEY = os.getenv("POLYMARKET_PRIVATE_KEY")          # Chave privada Polygon (L1 auth)
POLYMARKET_API_KEY = os.getenv("POLYMARKET_API_KEY", "")                    # L2 API Key
POLYMARKET_SECRET = os.getenv("POLYMARKET_API_SECRET", os.getenv("POLYMARKET_SECRET", ""))          # L2 API Secret
POLYMARKET_PASSPHRASE = os.getenv("POLYMARKET_API_PASSPHRASE", os.getenv("POLYMARKET_PASSPHRASE", ""))  # L2 API Passphrase

# ── Carteira financiadora (funder) ──────────────────────────────────────────
# O endereço exibido em polymarket.com/profile é a proxy wallet = funder.
# Se você usa EOA simples (sem conta no site), funder = endereço da sua EOA.
# Veja: https://docs.polymarket.com/api-reference/authentication#signature-types
POLYMARKET_FUNDER = os.getenv("POLYMARKET_FUNDER", "")

# ── Tipo de assinatura ────────────────────────────────────────────────────────
# 0 = EOA  (carteira MetaMask padrão, sem conta no Polymarket.com)
# 1 = POLY_PROXY  (conta criada via Magic Link/Google no Polymarket.com)
# 2 = GNOSIS_SAFE (proxy mais comum para contas novas no site)
# Se você criou conta via Polymarket.com, use 1. Se instalou MetaMask sem conta no site, use 0.
CLOB_SIGNATURE_TYPE: int = int(os.getenv("POLYMARKET_SIGNATURE_TYPE", "1"))

# ==================== URLS API ====================
POLYMARKET_CLOB_API = "https://clob.polymarket.com"
POLYMARKET_GAMMA_API = "https://gamma-api.polymarket.com"
POLYMARKET_WSS = "wss://ws-subscriptions-clob.polymarket.com/ws/market"

# URLs Binance
BINANCE_WSS_BASE = "wss://stream.binance.com:9443"

# ==================== MERCADOS ALVO ====================
# Configuração de mercados para busca automática
# IMPORTANTE: Slugs mudam a cada 5/15 min, então buscamos automaticamente
TARGET_MARKETS = {
    "BTC": {
        "symbol": "BTCUSDT",
        "binance_stream": "btcusdt@kline_1m",
        "intervals": [5, 15],  # minutos
        "search_terms": ["btc", "bitcoin", "up", "down"]
    },
    "ETH": {
        "symbol": "ETHUSDT",
        "binance_stream": "ethusdt@kline_1m",
        "intervals": [5, 15],
        "search_terms": ["eth", "ethereum", "up", "down"]
    },
    "SOL": {
        "symbol": "SOLUSDT",
        "binance_stream": "solusdt@kline_1m",
        "intervals": [5, 15],
        "search_terms": ["sol", "solana", "up", "down"]
    },
    "XRP": {
        "symbol": "XRPUSDT",
        "binance_stream": "xrpusdt@kline_1m",
        "intervals": [5, 15],
        "search_terms": ["xrp", "ripple", "up", "down"]
    }
}

# Padrões de slug para detecção automática
MARKET_SLUG_PATTERNS = {
    5: r"(btc|eth|sol|xrp)-?updown-?5m?-?\d+",
    15: r"(btc|eth|sol|xrp)-?updown-?15m?-?\d+"
}

# Filtros para encontrar mercados ativos
MARKET_FILTERS = {
    "active": True,  # Apenas mercados ativos
    "closed": False,  # Não incluir fechados
    "archived": False,  # Não incluir arquivados
    "enable_order_book": True  # Apenas com orderbook ativo
}

# ==================== PARÂMETROS BAYESIANOS ====================
BAYESIAN_PARAMS = {
    # PRIOR CALIBRADO À REALIDADE — Dados históricos mostram UP=66.3%, DOWN=33.7%
    # O prior errado era a causa raiz: prior_down=0.55 fazia o modelo apostar DOWN
    # em ~89% dos trades quando a realidade é UP na maioria das vezes.
    "prior_up": 0.62,
    "prior_down": 0.38,

    # PESOS — mais ênfase em momentum e trend (sinais mais informativos)
    "momentum_weight": 0.40,
    "volume_weight": 0.10,
    "volatility_weight": 0.10,
    "trend_weight": 0.40,

    # Janelas de tempo
    "short_window": 5,
    "medium_window": 15,
    "long_window": 30,

    # LIMIAR DE EDGE REAL para should_trade() — diferença mínima |p_up - p_down|
    "min_confidence": 0.63,   # threshold de confiança para saída de relatório
    "min_trade_edge": 0.08,   # edge mínimo para allow trade (|p_up - p_down|)

    # PARÂMETROS DE SINAL
    "rsi_extreme_boost": 1.5,
    "max_trend_confidence": 0.72,  # sinal de trend pode ser mais agressivo
    "strike_penalty_factor": 30,   # menor = penalidade maior por distância do strike
    "min_signal_strength": 0.60,   # prob mínima de um sinal para contar como 'forte'
}

# ==================== PARÂMETROS KELLY CRITERION ====================
KELLY_PARAMS = {
    # Kelly fracionário — 0.35 garante posições ≥$2.50 com $38 mesmo no pior caso (ask=0.55)
    "kelly_fraction": 0.35,

    # Limites de posição (USD)
    # Com $38 de bankroll: max $5 por trade = ~13% da banca
    #   min = 5 × $0.50 = $2.50  |  max = 10 × $0.50 = $5.00
    "max_position_size": 5.0,
    "min_position_size": 2.50,

    # Edge mínimo — 7% de vantagem real sobre o mercado é necessário
    "min_edge": 0.07,

    # Máximo % do bankroll por trade: 13% de $38 = $4.94
    "max_bankroll_per_trade": 0.13,

    # Redução automática após losses
    "loss_reduction_factor": 0.5,
    "win_increase_factor": 1.1,
}

# ==================== GERENCIAMENTO DE RISCO ====================
RISK_PARAMS = {
    # Stop loss
    "stop_loss_pct": 0.15,  # 15% de perda máxima por posição
    
    # Máximo de posições simultâneas — 2 para não expor >26% da banca de uma vez
    "max_concurrent_positions": 2,
    
    # Drawdown máximo permitido
    "max_drawdown": 0.25,  # 25%
    
    # Cooldown após perda
    "loss_cooldown_minutes": 30,
    
    # Limite de perdas consecutivas
    "max_consecutive_losses": 3
}

# ==================== MONITORAMENTO ====================
MONITOR_PARAMS = {
    # Bankroll é definido pelo usuário no console ao iniciar o bot
    # Não armazenar valor padrão aqui para evitar hardcoding acidental

    # Intervalo de atualização do monitor (segundos)
    "refresh_seconds": 30,

    # Número de velas para análise
    "candles_needed": 60
}

# ==================== CONFIGURAÇÕES DE EXECUÇÃO ====================
EXECUTION_PARAMS = {
    # Slippage máximo tolerado
    "max_slippage": 0.02,  # 2%
    
    # Timeout para ordens
    "order_timeout_seconds": 30,
    
    # Tempo mínimo entre trades
    "min_time_between_trades": 60,  # segundos

    # Logging verboso
    "verbose": True
}

# ==================== FEATURES PARA MODELO BAYESIANO ====================
FEATURES_CONFIG = {
    "price_changes": {
        "enabled": True,
        "windows": [1, 3, 5, 10, 15]  # minutos
    },
    "volume_profile": {
        "enabled": True,
        "relative_threshold": 1.5,  # 150% do volume médio
        "extreme_threshold": 2.5,  # NOVO: volume extremamente alto
    },
    "volatility": {
        "enabled": True,
        "method": "std",  # standard deviation
        "window": 15
    },
    "momentum": {
        "enabled": True,
        "rsi_period": 14,
        "rsi_overbought": 68,   # RSI > 68 = sobrecomprado com sinal real
        "rsi_oversold": 32,    # RSI < 32 = sobrevendido com sinal real
        "rsi_extreme": 78,     # RSI > 78 = extremo — sinal forte
        "rsi_neutral_low": 42, # Zona neutra inferior — sem sinal
        "rsi_neutral_high": 58, # Zona neutra superior — sem sinal
    },
    "trend": {
        "enabled": True,
        "ema_fast": 5,
        "ema_slow": 15,
        "max_confidence": 0.72,  # sinal de trend pode ser mais forte
        "min_gap_pct": 0.10,     # gap mínimo EMA (%) para gerar sinal — evita ruído
    }
}

# ==================== PARÂMETROS DO ORDER BOOK ====================
# Faixa de preço para compra no order book da Polymarket.
# O bot só entra se o best ask do token estiver nessa faixa.
# Racional: comprar entre 50c e 55c garante margem positiva quando o modelo
# projeta probabilidade >55%, e reflete os melhores resultados históricos.
ORDER_BOOK_PARAMS = {
    "min_buy_price": 0.50,    # Mínimo: 50 centavos (break-even teórico a 50% WR)
    "max_buy_price": 0.58,    # Máximo: 58 centavos (lucrativo com WR >58%)
    "min_ask_size_usd": 5.0,  # Liquidez mínima no ask para entrar (em USDC)
    "min_shares": 1,          # Mínimo de 1 share — permite ordens de $1
    "order_type": "GTC",      # Good Till Cancelled
}

# ==================== WEBSOCKET SETTINGS ====================
WS_SETTINGS = {
    "reconnect_delay": 5,  # segundos
    "max_reconnect_attempts": 10,
    "ping_interval": 30,
    "ping_timeout": 10,
    "message_queue_size": 1000
}

# ==================== LOGGING ====================
LOG_CONFIG = {
    "level": "INFO",
    "format": "%(asctime)s - %(name)s - %(levelname)s - %(message)s",
    "file": "polymarket_bot.log",
    "console": True
}

# ==================== MODOS DE OPERAÇÃO ====================
"""
Três perfis de risco/retorno:
  - CONSERVATIVE: Menor risco, posições pequenas, alta confiança necessária
  - AGGRESSIVE: Risco moderado, posições médias, confiança média
  - DEGEN: Alto risco, posições grandes, baixa confiança (yolo mode)
"""

TRADING_MODES = {
    "CONSERVATIVE": {
        "name": "Conservador 🛡️",
        "description": "Baixo risco, alta seletividade",
        "log_file": "trades_log_conservative.json",
        "bayesian": {
            "prior_up": 0.45,
            "prior_down": 0.55,
            "momentum_weight": 0.35,
            "volume_weight": 0.20,
            "volatility_weight": 0.10,
            "trend_weight": 0.35,
            "short_window": 5,
            "medium_window": 15,
            "long_window": 30,
            "min_confidence": 0.60,  # Alta confiança necessária
            "rsi_extreme_boost": 1.5,
            "max_trend_confidence": 0.65,
            "strike_penalty_factor": 50,
        },
        "kelly": {
            "kelly_fraction": 0.20,  # Conservador para $38
            "max_position_size": 5.0,   # ~13% de $38
            "min_position_size": 2.50,  # 5 shares × $0.50
            "min_edge": 0.05,
            "max_bankroll_per_trade": 0.15,
            "loss_reduction_factor": 0.5,
            "win_increase_factor": 1.05,
        },
        "risk": {
            "stop_loss_pct": 0.10,
            "max_concurrent_positions": 2,
            "max_drawdown": 0.15,
            "loss_cooldown_minutes": 60,
            "max_consecutive_losses": 2,
        },
        "monitor": {
            "refresh_seconds": 30,
            "candles_needed": 60,
        }
    },
    
    "AGGRESSIVE": {
        "name": "Agressivo ⚔️",
        "description": "Risco moderado, retorno balanceado",
        "log_file": "trades_log_aggressive.json",
        "bayesian": {
            "prior_up": 0.45,
            "prior_down": 0.55,
            "momentum_weight": 0.35,
            "volume_weight": 0.20,
            "volatility_weight": 0.10,
            "trend_weight": 0.35,
            "short_window": 5,
            "medium_window": 15,
            "long_window": 30,
            "min_confidence": 0.62,  # AUMENTADO de 0.55 para ser mais seletivo
            "rsi_extreme_boost": 1.5,
            "max_trend_confidence": 0.65,
            "strike_penalty_factor": 50,
        },
        "kelly": {
            "kelly_fraction": 0.35,
            "max_position_size": 5.00,   # ~13% de $38
            "min_position_size": 2.50,   # 5 shares × $0.50
            "min_edge": 0.05,
            "max_bankroll_per_trade": 0.13,
            "loss_reduction_factor": 0.5,
            "win_increase_factor": 1.1,
        },
        "risk": {
            "stop_loss_pct": 0.15,
            "max_concurrent_positions": 2,
            "max_drawdown": 0.25,
            "loss_cooldown_minutes": 30,
            "max_consecutive_losses": 3,
        },
        "monitor": {
            "refresh_seconds": 30,
            "candles_needed": 60,
        }
    },
    
    "AGGRESSIVE_OPTIMIZED": {
        "name": "Agressivo Otimizado 🎯",
        "description": "Baseado em análise de 575 trades - Filtros inteligentes",
        "log_file": "trades_log_aggressive_optimized.json",
        "bayesian": {
            "prior_up": 0.55,  # UP tem melhor performance (53.3% WR vs 42.9% DOWN)
            "prior_down": 0.45,
            "momentum_weight": 0.35,
            "volume_weight": 0.15,  # Reduzido (volume alto correlaciona com piores trades)
            "volatility_weight": 0.15,
            "trend_weight": 0.35,
            "short_window": 5,
            "medium_window": 15,
            "long_window": 30,
            "min_confidence": 0.54,  # 54% é o ponto ótimo (74 trades, 52.7% WR, $166 lucro)
            "rsi_extreme_boost": 1.5,
            "max_trend_confidence": 0.65,
            "strike_penalty_factor": 50,
        },
        "kelly": {
            "kelly_fraction": 0.35,  # Calibrado para $38 de banca
            "max_position_size": 5.00,   # ~13% de $38
            "min_position_size": 2.50,   # 5 shares × $0.50 (mínimo da Polymarket)
            "min_edge": 0.05,
            "max_bankroll_per_trade": 0.13,
            "loss_reduction_factor": 0.4,
            "win_increase_factor": 1.05,
        },
        "risk": {
            "stop_loss_pct": 0.12,
            "max_concurrent_positions": 2,
            "max_drawdown": 0.20,
            "loss_cooldown_minutes": 45,
            "max_consecutive_losses": 3,
        },
        "filters": {
            # FILTROS BASEADOS EM ANÁLISE DE DADOS REAIS
            "enabled": True,
            "price_vs_strike_min": 0.005,  # 0.5% mínimo (trades <0.5% têm 37.8% WR)
            "momentum_5m_min": 0.0005,  # 0.05% mínimo
            "allow_down_trades": False,  # DOWN tem 42.9% WR vs 53.3% UP
            "volume_ratio_max": 1.5,  # Volume muito alto correlaciona com piores trades
            "rsi_min": 35,
            "rsi_max": 85,
            # Horários - pode habilitar/desabilitar para testar
            "filter_by_hour": True,  # Mude para False para desabilitar filtro de horário
            "blocked_hours": [2, 3, 5, 12, 18, 19, 22],  # Horários com WR < 45%
            "preferred_hours": [],  # Vazio = aceita qualquer (exceto blocked)
        },
        "monitor": {
            "refresh_seconds": 30,
            "candles_needed": 60,
        }
    },
    
    "DEGEN": {
        "name": "Degen 🚀💎",
        "description": "Alto risco, máximo retorno (YOLO mode)",
        "log_file": "trades_log_degen.json",
        "bayesian": {
            "prior_up": 0.45,
            "prior_down": 0.55,
            "momentum_weight": 0.40,  # Mais peso em momentum
            "volume_weight": 0.25,
            "volatility_weight": 0.05,  # Menos preocupação com volatilidade
            "trend_weight": 0.30,
            "short_window": 5,
            "medium_window": 15,
            "long_window": 30,
            "min_confidence": 0.51,  # Confiança baixa (aceita mais trades)
            "rsi_extreme_boost": 2.0,  # Maior boost em extremos
            "max_trend_confidence": 0.70,
            "strike_penalty_factor": 30,  # Menos penalidade
        },
        "kelly": {
            "kelly_fraction": 0.50,  # Degen agressivo
            "max_position_size": 15.0,   # ~40% de $38
            "min_position_size": 2.50,   # 5 shares × $0.50
            "min_edge": 0.01,
            "max_bankroll_per_trade": 0.40,
            "loss_reduction_factor": 0.7,
            "win_increase_factor": 1.2,
        },
        "risk": {
            "stop_loss_pct": 0.25,
            "max_concurrent_positions": 6,
            "max_drawdown": 0.40,
            "loss_cooldown_minutes": 15,
            "max_consecutive_losses": 5,
        },
        "monitor": {
            "refresh_seconds": 30,
            "candles_needed": 60,
        }
    }
}

# Modo ativo (será definido em runtime)
ACTIVE_MODE = "CONSERVATIVE"  # Padrão

def set_active_mode(mode: str) -> bool:
    """
    Define o modo ativo e atualiza as configurações globais
    
    Args:
        mode: "CONSERVATIVE", "AGGRESSIVE" ou "DEGEN"
        
    Returns:
        True se modo válido, False caso contrário
    """
    global ACTIVE_MODE, BAYESIAN_PARAMS, KELLY_PARAMS, RISK_PARAMS, MONITOR_PARAMS
    
    if mode not in TRADING_MODES:
        print(f"❌ Modo inválido: {mode}")
        print(f"   Modos disponíveis: {', '.join(TRADING_MODES.keys())}")
        return False
    
    ACTIVE_MODE = mode
    config = TRADING_MODES[mode]
    
    # Atualiza configurações globais
    BAYESIAN_PARAMS.update(config["bayesian"])
    KELLY_PARAMS.update(config["kelly"])
    RISK_PARAMS.update(config["risk"])
    MONITOR_PARAMS.update(config["monitor"])
    
    return True

def get_log_file() -> str:
    """Retorna o arquivo de log do modo ativo"""
    return TRADING_MODES[ACTIVE_MODE]["log_file"]

def get_mode_info() -> dict:
    """Retorna informações do modo ativo"""
    return TRADING_MODES[ACTIVE_MODE]

# ==================== VALIDAÇÃO ====================
def validate_config() -> bool:
    """Valida configurações essenciais"""
    issues = []
    warnings = []
    dry_run_enabled = os.getenv("BAYESIAN_DRY_RUN", os.getenv("DRY_RUN", "false")).lower() in (
        "1",
        "true",
        "yes",
        "on",
    )

    # Chave privada é obrigatória
    if not PRIVATE_KEY and not dry_run_enabled:
        issues.append("POLYMARKET_PRIVATE_KEY não definida")

    if not PRIVATE_KEY and dry_run_enabled:
        warnings.append("POLYMARKET_PRIVATE_KEY nao definida; OK em dry-run")

    if KELLY_PARAMS["kelly_fraction"] > 1.0:
        issues.append("kelly_fraction não pode ser maior que 1.0")

    if BAYESIAN_PARAMS["min_confidence"] <= 0.5:
        issues.append("min_confidence deve ser maior que 0.5")

    # Valida faixa de preço do order book
    min_p = ORDER_BOOK_PARAMS["min_buy_price"]
    max_p = ORDER_BOOK_PARAMS["max_buy_price"]
    if min_p >= max_p:
        issues.append(f"ORDER_BOOK_PARAMS: min_buy_price ({min_p}) deve ser menor que max_buy_price ({max_p})")
    if min_p < 0.01 or max_p > 0.99:
        issues.append("ORDER_BOOK_PARAMS: preços devem estar entre 0.01 e 0.99")

    if warnings:
        print("ℹ️  AVISOS DE CONFIGURAÇÃO:")
        for w in warnings:
            print(f"  - {w}")

    if issues:
        for issue in issues:
            print(f"  - {issue}")
        return False
    
    return True
