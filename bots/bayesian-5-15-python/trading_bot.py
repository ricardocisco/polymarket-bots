"""
Trading Bot Principal
Integra Polymarket WebSocket, Binance WebSocket, Modelo Bayesiano e Kelly Criterion
"""
import asyncio
import logging
from typing import Dict, List, Optional
from datetime import datetime, timedelta
from dataclasses import dataclass, field
import requests

from config import (
    TARGET_MARKETS, EXECUTION_PARAMS, RISK_PARAMS, MONITOR_PARAMS,
    POLYMARKET_GAMMA_API, POLYMARKET_CLOB_API, TRADING_MODES, ACTIVE_MODE,
    ORDER_BOOK_PARAMS, POLYMARKET_API_KEY, POLYMARKET_SECRET, POLYMARKET_PASSPHRASE,
    PRIVATE_KEY, POLYMARKET_FUNDER, CLOB_SIGNATURE_TYPE,
)
from polymarket_ws import PolymarketWebSocket, MarketData
from binance_ws import BinanceWebSocket, BinanceKline
from bayesian_model import OptimizedBayesianModel, BayesianPrediction
from kelly_criterion import KellyCriterion, KellyResult
from market_discovery import MarketDiscovery, ActiveMarket
from trade_logger import log_entry as _log_trade_to_json

logger = logging.getLogger(__name__)


@dataclass
class Position:
    """Posição aberta no mercado"""
    market_id: str
    symbol: str
    direction: str  # "UP" ou "DOWN"
    entry_price: float
    position_size: float
    entry_time: datetime
    prediction: BayesianPrediction
    kelly_result: KellyResult
    current_price: Optional[float] = None
    pnl: float = 0.0
    status: str = "OPEN"  # OPEN, CLOSED, EXPIRED
    order_id: Optional[str] = None  # ID da ordem real no CLOB (None = paper trading)
    token_id: Optional[str] = None  # Token ID no Polymarket
    
    def update_pnl(self, current_price: float):
        """Atualiza P&L da posição"""
        self.current_price = current_price
        
        if self.direction == "UP":
            # Se apostamos UP e preço subiu, ganhamos
            self.pnl = (current_price - self.entry_price) * self.position_size
        else:
            # Se apostamos DOWN e preço caiu, ganhamos
            self.pnl = (self.entry_price - current_price) * self.position_size


@dataclass
class TradingState:
    """Estado atual do bot"""
    bankroll: float
    positions: List[Position] = field(default_factory=list)
    total_pnl: float = 0.0
    trades_count: int = 0
    wins: int = 0
    losses: int = 0
    consecutive_losses: int = 0
    last_trade_time: Optional[datetime] = None
    is_trading_allowed: bool = True
    
    def get_open_positions_count(self) -> int:
        return len([p for p in self.positions if p.status == "OPEN"])
    
    def get_win_rate(self) -> float:
        if self.trades_count == 0:
            return 0.0
        return (self.wins / self.trades_count) * 100


@dataclass
class OrderBookPrice:
    """Preços do melhor nível do order book (ask = preço para comprar)"""
    token_id: str
    best_ask: float
    best_bid: float
    mid_price: float
    spread: float
    ask_size: float
    bid_size: float
    tick_size: str
    neg_risk: bool


@dataclass
class OrderResult:
    """Resultado de uma ordem colocada no CLOB"""
    success: bool
    order_id: str
    price: float
    size_tokens: float
    size_usd: float
    side: str
    status: str
    error: str = ""


class TradingBot:
    """
    Bot de Trading Principal
    """

    def __init__(self, initial_bankroll: Optional[float] = None, dry_run: bool = False):
        # Bankroll: deve ser passado explicitamente via main.py
        if initial_bankroll is None:
            initial_bankroll = MONITOR_PARAMS.get('initial_bankroll')
            if not initial_bankroll:
                raise ValueError("Bankroll não definido. Execute o bot via main.py")
        self.state = TradingState(bankroll=initial_bankroll)
        self.dry_run = dry_run
        
        # Modelos
        self.bayesian_model = OptimizedBayesianModel()
        self.kelly = KellyCriterion(initial_bankroll)
        
        # Market Discovery - busca mercados automaticamente
        self.market_discovery = MarketDiscovery()
        
        # WebSockets
        self.poly_ws: Optional[PolymarketWebSocket] = None
        self.binance_ws: Optional[BinanceWebSocket] = None
        
        # Cache de mercados ativos
        self.active_markets: Dict[str, ActiveMarket] = {}

        # Cache de preços do order book (atualizado pelo WebSocket)
        # token_id -> MarketData (com bid/ask)
        self._book_cache: Dict[str, MarketData] = {}

        # Lock para evitar race condition: múltiplos klines chegando ao mesmo tempo
        # sem o lock, todos verificam positions==0 antes de qualquer um registrar
        self._trade_lock = asyncio.Lock()

        # Slugs já comprados neste ciclo de mercado (limpa quando mercados mudam)
        self._traded_slugs: set = set()

        # CLOB: SDK inicializado em _init_clob_sdk()
        self._clob_trading_client = None
        self._clob_public_client = None
        self._clob_sdk_available = False
        self._clob_session = requests.Session()
        self._clob_session.headers.update({"User-Agent": "PolymarketBot/1.0"})
        self._init_clob_sdk()

        # Carregar filtros do modo de trading atual
        trading_mode = EXECUTION_PARAMS.get('mode', ACTIVE_MODE)
        mode_config = TRADING_MODES.get(trading_mode, {})
        self.filters = mode_config.get('filters', {})

        logger.info(
            "🤖 Trading Bot inicializado — "
            + ("DRY-RUN" if self.dry_run else "LIVE TRADING")
        )
        if self.dry_run:
            logger.warning("🧪 DRY-RUN — ordens reais desabilitadas.")
        else:
            logger.warning("⚠️  LIVE TRADING — ordens reais serão executadas na Polymarket!")
        if self.filters.get('enabled'):
            logger.info(f"🎯 Filtros inteligentes: ATIVADOS")
    
    async def initialize(self):
        """Inicializa o bot - carrega mercados e conecta WebSockets"""
        logger.info("🚀 Inicializando Trading Bot...")
        
        # 1. Descobre mercados ativos automaticamente
        self.active_markets = await self.market_discovery.discover_all_markets()
        
        if not self.active_markets:
            logger.error("❌ Nenhum mercado ativo encontrado!")
            logger.info("💡 Verifique se há mercados UP/DOWN ativos na Polymarket")
            return
        
        # Mostra mercados encontrados
        self.market_discovery.print_active_markets()
        
        # 2. Configura WebSocket Polymarket
        token_ids = self.market_discovery.get_all_token_ids()
        
        if token_ids:
            self.poly_ws = PolymarketWebSocket(token_ids)
            
            # Registra callbacks
            self.poly_ws.on_price_change(self._on_poly_price_change)
            self.poly_ws.on_book(self._on_poly_book_update)
        
        # 3. Configura WebSocket Binance
        binance_streams = list(set([
            config['binance_stream']
            for config in TARGET_MARKETS.values()
        ]))
        
        self.binance_ws = BinanceWebSocket(binance_streams)
        self.binance_ws.on_kline(self._on_binance_kline)
        
        logger.info("✅ Bot inicializado com sucesso!")
    
    async def _on_poly_price_change(self, updates: List[MarketData]):
        """Callback quando preço muda na Polymarket"""
        for update in updates:
            # Encontra mercado correspondente
            market = self._find_market_by_token_id(update.asset_id)
            
            if market:
                logger.debug(
                    f"💹 {market.crypto} {market.interval}min: ${update.price:.4f}"
                )
                
                # Atualiza P&L de posições abertas
                await self._update_positions_pnl(update)
    
    async def _on_poly_book_update(self, updates: List[MarketData]):
        """Callback quando orderbook atualiza — cacheia bid/ask por token"""
        for update in updates:
            # Atualiza cache de preços
            if update.asset_id and update.ask is not None:
                self._book_cache[update.asset_id] = update

            market = self._find_market_by_token_id(update.asset_id)
            if market:
                ask_str = f"${update.ask:.4f}" if update.ask is not None else "N/A"
                bid_str = f"${update.bid:.4f}" if update.bid is not None else "N/A"
                logger.debug(
                    f"📖 {market.crypto} {market.interval}min: "
                    f"Bid={bid_str}, Ask={ask_str}"
                )
    
    def _find_market_by_token_id(self, token_id: str) -> Optional[ActiveMarket]:
        """Encontra mercado ativo pelo token ID"""
        for market in self.active_markets.values():
            if market.token_id_yes == token_id or market.token_id_no == token_id:
                return market
        return None
    
    async def _on_binance_kline(self, kline: BinanceKline):
        """Callback quando novo candle chega da Binance"""
        # Só processa candles fechados
        if not kline.is_closed:
            return
        
        logger.debug(
            f"🕯️  {kline.symbol}: ${kline.close:.2f} "
            f"(Vol: {kline.volume:.2f})"
        )
        
        # Analisa oportunidade de trade
        await self._analyze_trading_opportunity(kline.symbol)
    
    async def _analyze_trading_opportunity(self, symbol: str):
        """Analisa se há oportunidade de trade"""
        try:
            # Verifica se pode tradear
            if not self._can_trade():
                return
            
            # Refresca mercados se necessário
            # Quando mercados mudam (novo ciclo de 5/15min), limpa slugs já comprados
            slugs_before = set(self.market_discovery.active_markets.keys())
            await self.market_discovery.refresh_if_needed()
            slugs_after = set(self.market_discovery.active_markets.keys())
            if slugs_after != slugs_before:
                self._traded_slugs.clear()
                logger.info("🔄 Novo ciclo de mercados — slugs resetados")
            
            # Pega dados recentes da Binance
            recent_candles = self.binance_ws.get_recent_prices(symbol, 100)
            
            if len(recent_candles) < 30:
                logger.debug(f"Dados insuficientes para {symbol}")
                return
            
            # Gera predição Bayesiana
            prediction = self.bayesian_model.predict(symbol, recent_candles)
            
            # Verifica confiança mínima
            if not prediction.should_trade():
                logger.debug(
                    f"⏭️  {symbol}: Confiança insuficiente "
                    f"({prediction.confidence:.3f})"
                )
                return
            
            # Aplica filtros inteligentes
            can_trade, filter_reason = self._apply_filters(prediction, recent_candles)
            if not can_trade:
                logger.debug(f"🚫 {symbol}: {filter_reason}")
                return
            
            # Extrai cripto do symbol (ex: "BTCUSDT" -> "BTC")
            crypto = symbol.replace("USDT", "")

            # Busca mercados ativos para esta cripto — tenta 5min e 15min
            for interval in [5, 15]:
                market = self.market_discovery.get_active_market(crypto, interval)

                if not market or market.is_expired():
                    continue

                # Determina qual token usar baseado na predição
                direction = prediction.get_direction()
                token_id = market.token_id_yes if direction == "UP" else market.token_id_no

                # ── Preço real do order book ────────────────────────────────
                ob_price = self._get_book_price(token_id)

                if ob_price is None:
                    logger.warning(
                        f"⚠️  Sem preço do order book para "
                        f"{crypto} {interval}min ({direction}) — pulando"
                    )
                    continue

                ask = ob_price.best_ask

                # ── Filtro de faixa 50-55c ──────────────────────────────────
                min_price = ORDER_BOOK_PARAMS['min_buy_price']   # 0.50
                max_price = ORDER_BOOK_PARAMS['max_buy_price']   # 0.55
                min_liquidity = ORDER_BOOK_PARAMS['min_ask_size_usd']  # 5 USDC

                if not (min_price <= ask <= max_price):
                    logger.info(
                        f"⏭️  {crypto} {interval}min | {direction} | "
                        f"Ask=${ask:.3f} fora da faixa [{min_price:.2f}-{max_price:.2f}] — ignorado"
                    )
                    continue

                # Liquidez mínima no ask
                ask_size_usd = ob_price.ask_size * ask
                if ask_size_usd < min_liquidity:
                    logger.info(
                        f"⏭️  {crypto} {interval}min | Liquidez insuficiente no ask: "
                        f"${ask_size_usd:.2f} < ${min_liquidity:.2f}"
                    )
                    continue

                # ── Kelly Criterion com preços reais ────────────────────────
                if direction == "UP":
                    token_yes_price = ask
                    token_no_price = 1.0 - ask
                else:
                    token_no_price = ask
                    token_yes_price = 1.0 - ask

                kelly_result = self.kelly.calculate(
                    prediction,
                    ask,            # market_price: o que o mercado cobra pela nossa direção
                    token_yes_price,
                    token_no_price,
                )

                # Valida aposta
                should_bet, reason = self.kelly.validate_bet(
                    kelly_result,
                    self.state.get_open_positions_count()
                )

                if not should_bet:
                    logger.debug(f"❌ {symbol} ({interval}min): {reason}")
                    continue

                logger.info(
                    f"💰 {crypto} {interval}min | {direction} | "
                    f"Ask=${ask:.3f} | Edge={kelly_result.edge:.3f} | "
                    f"Size=${kelly_result.position_size:.2f}"
                )

                # EXECUTA TRADE — com lock para evitar race condition
                async with self._trade_lock:
                    # Re-verifica dentro do lock (pode ter mudado enquanto aguardava)
                    if market.slug in self._traded_slugs:
                        logger.info(f"⏭️  {crypto} {interval}min | Slug já comprado neste ciclo — ignorado")
                        break

                    open_pos = self.state.get_open_positions_count()
                    if open_pos >= RISK_PARAMS['max_concurrent_positions']:
                        logger.info(f"⏭️  {crypto} {interval}min | Máximo de posições ({open_pos}) atingido")
                        break

                    self._traded_slugs.add(market.slug)
                    await self._execute_trade(
                        market=market,
                        prediction=prediction,
                        kelly_result=kelly_result,
                        market_price=ask,
                        token_id=token_id,
                        recent_candles=recent_candles,
                        tick_size=ob_price.tick_size,
                        neg_risk=ob_price.neg_risk,
                    )

                # Só faz um trade por análise
                break

        except Exception as e:
            logger.error(f"Erro ao analisar {symbol}: {e}", exc_info=True)
    
    def _get_book_price(self, token_id: str) -> Optional[OrderBookPrice]:
        """
        Retorna preço do order book com tick_size e neg_risk.

        Preferência: cache do WebSocket (atualizado em tempo real via best_bid_ask).
        Fallback: consulta REST/SDK (retorna tick_size e neg_risk).
        """
        # Tenta cache do WebSocket (evento best_bid_ask ou book)
        cached = self._book_cache.get(token_id)
        if cached and cached.ask is not None and cached.bid is not None:
            age = (datetime.utcnow() - cached.timestamp).total_seconds()
            if age < 10:
                # Cache WS não tem tick_size/neg_risk — busca via SDK/REST para esses campos
                full = self._get_order_book_price(token_id)
                if full:
                    return full
                # Se a consulta falhar, retorna com defaults
                return OrderBookPrice(
                    token_id=token_id,
                    best_ask=cached.ask,
                    best_bid=cached.bid,
                    mid_price=(cached.ask + cached.bid) / 2,
                    spread=cached.ask - cached.bid,
                    ask_size=0.0,
                    bid_size=0.0,
                    tick_size="0.01",
                    neg_risk=False,
                )

        # Fallback: consulta completa via SDK/REST
        return self._get_order_book_price(token_id)

    async def _execute_trade(
        self,
        market: ActiveMarket,
        prediction: BayesianPrediction,
        kelly_result: KellyResult,
        market_price: float,
        token_id: str,
        recent_candles: List[BinanceKline],
        tick_size: str = "0.01",
        neg_risk: bool = False,
    ):
        """Executa trade no mercado — posição fixa de $1.00 para validação da ideia"""
        direction = prediction.get_direction()

        # ── Posição fixa de $1.00 por trade ────────────────────────────────────
        # Valor mínimo para validar o algoritmo sem risco significativo.
        # Ignora Kelly completamente enquanto observamos o desempenho.
        FLAT_USD = 1.00
        size_usd = FLAT_USD

        logger.info(
            f"\n{'='*60}\n"
            f"🎯 EXECUTANDO TRADE\n"
            f"{'='*60}\n"
            f"Mercado: {market.crypto} {market.interval}min\n"
            f"Slug: {market.slug}\n"
            f"Direção: {direction}\n"
            f"Ask (order book): ${market_price:.4f}\n"
            f"Tamanho Posição: ${size_usd:.2f} USDC fixo\n"
            f"Probabilidade modelo: {prediction.confidence:.1%}\n"
            f"Edge: {kelly_result.edge:.3f}\n"
            f"Expira em: {market.time_until_expiry()}\n"
            f"{'='*60}"
        )

        # ── Execução real via CLOB API ──────────────────────────────
        if self.dry_run:
            trade_id = self._log_trade_entry(
                market=market,
                prediction=prediction,
                kelly_result=kelly_result,
                market_price=market_price,
                size_usd=size_usd,
                direction=direction,
                recent_candles=recent_candles,
            )

            position = Position(
                market_id=market.slug,
                symbol=market.crypto,
                direction=direction,
                entry_price=market_price,
                position_size=size_usd,
                entry_time=datetime.utcnow(),
                prediction=prediction,
                kelly_result=kelly_result,
                order_id="DRY-RUN",
                token_id=token_id,
            )

            self.state.positions.append(position)
            self.state.trades_count += 1
            self.state.last_trade_time = datetime.utcnow()

            logger.info(
                f"[DRY-RUN] Trade simulado | {market.crypto} {direction} @ "
                f"${market_price:.3f} | ${size_usd:.2f} USDC | LogID={trade_id}"
            )
            return

        order = self._place_limit_order(
            token_id=token_id,
            price=market_price,
            size_usd=size_usd,
            side="BUY",
            tick_size=tick_size,
            neg_risk=neg_risk,
        )

        if order.success:
            # ── Registra entrada no JSON (trades_log_*.json) ───────────────────
            trade_id = self._log_trade_entry(
                market=market,
                prediction=prediction,
                kelly_result=kelly_result,
                market_price=market_price,
                size_usd=size_usd,
                direction=direction,
                recent_candles=recent_candles,
            )

            position = Position(
                market_id=market.slug,
                symbol=market.crypto,
                direction=direction,
                entry_price=market_price,
                position_size=size_usd,
                entry_time=datetime.utcnow(),
                prediction=prediction,
                kelly_result=kelly_result,
                order_id=order.order_id,
                token_id=token_id,
            )

            self.state.positions.append(position)
            self.state.trades_count += 1
            self.state.last_trade_time = datetime.utcnow()

            logger.info(
                f"✅ Ordem REAL colocada | {market.crypto} {direction} @ "
                f"${market_price:.3f} | ${size_usd:.2f} USDC | "
                f"OrderID={order.order_id} | LogID={trade_id}"
            )
        else:
            logger.error(
                f"❌ Falha ao colocar ordem real: {order.error}\n"
                f"   Token: {token_id[:20]}... | Price: {market_price:.3f} | "
                f"Size: ${size_usd:.2f}"
            )

    def _log_trade_entry(
        self,
        market: ActiveMarket,
        prediction: BayesianPrediction,
        kelly_result: KellyResult,
        market_price: float,
        size_usd: float,
        direction: str,
        recent_candles: List[BinanceKline],
    ) -> str:
        """Registra entrada no arquivo JSON de log (trade_logger). Retorna trade_id ou ''."""
        try:
            # Extrai sinal por nome da lista prediction.signals
            def _sig(name: str):
                for s in prediction.signals:
                    if s.name == name:
                        return s
                return None

            rsi_sig  = _sig("momentum")
            trend_sig = _sig("trend")
            vol_sig  = _sig("volume")
            atr_sig  = _sig("volatility")

            rsi_value = rsi_sig.raw_value if rsi_sig else 50.0
            rsi_prob  = rsi_sig.p_up      if rsi_sig else 0.5

            # EMA5 / EMA15 — médias simples das últimas N velas (boa aproximação)
            closes = [c.close for c in recent_candles]
            ema5  = sum(closes[-5:])  / min(5,  len(closes)) if closes else market_price
            ema15 = sum(closes[-15:]) / min(15, len(closes)) if closes else market_price
            ema_prob = trend_sig.p_up if trend_sig else 0.5

            vol_ratio = vol_sig.raw_value if vol_sig else 1.0
            vol_prob  = vol_sig.p_up      if vol_sig else 0.5

            atr_value = atr_sig.raw_value if atr_sig else 0.0
            atr_prob  = atr_sig.p_up      if atr_sig else 0.5

            # Momentum de preço
            def _pct(n: int) -> float:
                if len(recent_candles) > n:
                    base = recent_candles[-n - 1].close
                    return (recent_candles[-1].close - base) / base * 100 if base else 0.0
                return 0.0

            current_price = prediction.current_price or (recent_candles[-1].close if recent_candles else market_price)
            strike_price  = prediction.strike_price  or market.strike_price or current_price
            price_vs_strike_pct = ((current_price - strike_price) / strike_price * 100) if strike_price > 0 else 0.0

            # Preços dos tokens YES/NO
            if direction == "UP":
                token_yes_price = market_price
                token_no_price  = round(1.0 - market_price, 4)
            else:
                token_no_price  = market_price
                token_yes_price = round(1.0 - market_price, 4)

            end_ts = int(market.end_time.timestamp()) if market.end_time else 0
            expires_at = market.end_time.isoformat() if market.end_time else ""

            return _log_trade_to_json(
                slug=market.slug,
                interval_minutes=market.interval,
                symbol=market.crypto,
                end_timestamp=end_ts,
                strike_price=strike_price,
                current_price=current_price,
                price_vs_strike_pct=price_vs_strike_pct,
                token_yes_price=token_yes_price,
                token_no_price=token_no_price,
                market_url=f"https://polymarket.com/event/{market.slug}",
                expires_at=expires_at,
                direction=direction,
                prob_up=prediction.p_up,
                prob_down=prediction.p_down,
                confidence_pct=prediction.confidence * 100,
                edge=prediction.edge,
                rsi_signal=rsi_prob,
                rsi_value=rsi_value,
                ema_signal=ema_prob,
                ema5=ema5,
                ema15=ema15,
                vol_signal=vol_prob,
                vol_ratio=vol_ratio,
                atr_signal=atr_prob,
                atr_value=atr_value,
                kelly_fraction=kelly_result.kelly_fraction,
                kelly_fraction_full=kelly_result.kelly_fraction_full,
                kelly_position_usd=size_usd,
                bankroll=self.state.bankroll,
                price_1m_pct=_pct(1),
                price_5m_pct=_pct(5),
                price_15m_pct=_pct(15),
            )
        except Exception as exc:
            logger.error(f"❌ Erro ao registrar trade no JSON: {exc}", exc_info=True)
            return ""
    
    # -----------------------------------------------------------------------
    # CLOB SDK — inicialização e operações
    # -----------------------------------------------------------------------

    def _init_clob_sdk(self):
        """Inicializa py-clob-client (L1 + L2) para consultas e ordens."""
        try:
            from py_clob_client.client import ClobClient
            from py_clob_client.clob_types import ApiCreds
            from py_clob_client.constants import POLYGON

            self._clob_public_client = ClobClient(
                host=POLYMARKET_CLOB_API,
                chain_id=POLYGON,
            )

            if not PRIVATE_KEY:
                logger.warning(
                    "⚠️  POLYMARKET_PRIVATE_KEY não configurada — "
                    "consultas de order book OK, mas ordens reais desabilitadas."
                )
                return

            has_creds = bool(POLYMARKET_API_KEY and POLYMARKET_SECRET and POLYMARKET_PASSPHRASE)

            if not has_creds:
                logger.info("🔑 Derivando credenciais API da wallet via L1 (primeira vez)...")
                temp = ClobClient(host=POLYMARKET_CLOB_API, chain_id=POLYGON, key=PRIVATE_KEY)
                derived = temp.create_or_derive_api_key()
                api_key = derived.api_key
                api_secret = derived.api_secret
                api_passphrase = derived.api_passphrase
                logger.warning(
                    "✅ Credenciais derivadas! Salve no .env:\n"
                    f"   POLYMARKET_API_KEY={api_key}\n"
                    f"   POLYMARKET_SECRET={api_secret}\n"
                    f"   POLYMARKET_PASSPHRASE={api_passphrase}"
                )
            else:
                api_key = POLYMARKET_API_KEY
                api_secret = POLYMARKET_SECRET
                api_passphrase = POLYMARKET_PASSPHRASE

            funder = POLYMARKET_FUNDER or None
            if not funder:
                logger.warning("⚠️  POLYMARKET_FUNDER não configurado.")

            self._clob_trading_client = ClobClient(
                host=POLYMARKET_CLOB_API,
                chain_id=POLYGON,
                key=PRIVATE_KEY,
                creds=ApiCreds(
                    api_key=api_key,
                    api_secret=api_secret,
                    api_passphrase=api_passphrase,
                ),
                signature_type=CLOB_SIGNATURE_TYPE,
                funder=funder,
            )
            self._clob_sdk_available = True
            logger.info(
                f"✅ CLOB SDK pronto | signature_type={CLOB_SIGNATURE_TYPE} | "
                f"funder={'configurado' if funder else 'NÃO configurado'}"
            )

        except ImportError:
            logger.warning(
                "⚠️  py-clob-client não instalado — use: pip install py-clob-client"
            )
        except Exception as e:
            logger.error(f"❌ Erro ao inicializar CLOB SDK: {e}", exc_info=True)

    def _get_order_book_price(self, token_id: str) -> Optional[OrderBookPrice]:
        """Consulta order book via SDK (preferência) ou REST (fallback)."""
        if self._clob_public_client is not None:
            try:
                book = self._clob_public_client.get_order_book(token_id)
                if book and book.asks:
                    best_ask = float(book.asks[-1].price)
                    best_bid = float(book.bids[-1].price) if book.bids else best_ask
                    return OrderBookPrice(
                        token_id=token_id,
                        best_ask=best_ask,
                        best_bid=best_bid,
                        mid_price=(best_ask + best_bid) / 2,
                        spread=best_ask - best_bid,
                        ask_size=float(book.asks[-1].size),
                        bid_size=float(book.bids[-1].size) if book.bids else 0.0,
                        tick_size=str(getattr(book, "tick_size", "0.01")),
                        neg_risk=bool(getattr(book, "neg_risk", False)),
                    )
            except Exception as e:
                logger.debug(f"SDK order book falhou para {token_id[:20]}...: {e}")

        # Fallback REST público
        try:
            resp = self._clob_session.get(
                f"{POLYMARKET_CLOB_API}/book",
                params={"token_id": token_id},
                timeout=5,
            )
            if not resp.ok:
                return None
            data = resp.json()
            asks = data.get("asks", [])
            bids = data.get("bids", [])
            if not asks:
                return None
            best_ask = float(asks[-1]["price"])
            best_bid = float(bids[-1]["price"]) if bids else best_ask
            return OrderBookPrice(
                token_id=token_id,
                best_ask=best_ask,
                best_bid=best_bid,
                mid_price=(best_ask + best_bid) / 2,
                spread=best_ask - best_bid,
                ask_size=float(asks[-1]["size"]),
                bid_size=float(bids[-1]["size"]) if bids else 0.0,
                tick_size=str(data.get("tick_size", "0.01")),
                neg_risk=bool(data.get("neg_risk", False)),
            )
        except requests.RequestException as e:
            logger.error(f"Erro de rede ao consultar order book: {e}")
            return None
        except Exception as e:
            logger.error(f"Erro ao consultar order book ({token_id[:20]}...): {e}")
            return None

    def _place_limit_order(
        self,
        token_id: str,
        price: float,
        size_usd: float,
        side: str = "BUY",
        tick_size: str = "0.01",
        neg_risk: bool = False,
    ) -> OrderResult:
        """Coloca ordem limite GTC no CLOB (L1 assina, L2 envia)."""
        if not self._clob_sdk_available or not self._clob_trading_client:
            return OrderResult(
                success=False, order_id="", price=price,
                size_tokens=round(size_usd / max(price, 0.01), 2),
                size_usd=size_usd, side=side, status="error",
                error="CLOB SDK não disponível. Verifique POLYMARKET_PRIVATE_KEY e py-clob-client.",
            )

        try:
            from py_clob_client.clob_types import OrderArgs, OrderType, PartialCreateOrderOptions

            size_tokens = round(size_usd / price, 2)
            min_shares = ORDER_BOOK_PARAMS.get("min_shares", 5)
            if size_tokens < min_shares:
                return OrderResult(
                    success=False, order_id="", price=price,
                    size_tokens=size_tokens, size_usd=size_usd, side=side, status="error",
                    error=(
                        f"Tamanho muito pequeno: {size_tokens:.2f} tokens. "
                        f"Mínimo {min_shares} shares (≈${min_shares * price:.2f} USDC)."
                    ),
                )

            signed_order = self._clob_trading_client.create_order(
                OrderArgs(price=price, size=size_tokens, side=side, token_id=token_id),
                PartialCreateOrderOptions(tick_size=tick_size, neg_risk=neg_risk),
            )
            resp = self._clob_trading_client.post_order(signed_order, OrderType.GTC)

            order_id = resp.get("orderID", "") if isinstance(resp, dict) else ""
            status = resp.get("status", "unknown") if isinstance(resp, dict) else "error"

            if order_id:
                logger.info(
                    f"✅ Ordem colocada | ID={order_id} | status={status} | "
                    f"price={price:.3f} | tokens={size_tokens:.2f} | ${size_usd:.2f} USDC"
                )
                return OrderResult(
                    success=True, order_id=order_id, price=price,
                    size_tokens=size_tokens, size_usd=size_usd, side=side, status=status,
                )
            return OrderResult(
                success=False, order_id="", price=price,
                size_tokens=size_tokens, size_usd=size_usd, side=side, status="error",
                error=f"Sem orderID na resposta: {resp}",
            )

        except Exception as e:
            logger.error(f"❌ Erro ao colocar ordem CLOB: {e}", exc_info=True)
            return OrderResult(
                success=False, order_id="", price=price,
                size_tokens=round(size_usd / max(price, 0.01), 2),
                size_usd=size_usd, side=side, status="error", error=str(e),
            )

    def _can_trade(self) -> bool:
        """Verifica se pode fazer novo trade"""
        # Verifica cooldown
        if self.state.last_trade_time:
            time_since_last = datetime.utcnow() - self.state.last_trade_time
            min_time = timedelta(seconds=EXECUTION_PARAMS['min_time_between_trades'])
            
            if time_since_last < min_time:
                return False
        
        # Verifica perdas consecutivas
        if self.state.consecutive_losses >= RISK_PARAMS['max_consecutive_losses']:
            logger.warning("⚠️  Máximo de perdas consecutivas atingido")
            return False
        
        # Verifica drawdown
        if self.state.total_pnl < 0:
            drawdown_pct = abs(self.state.total_pnl / self.state.bankroll)
            if drawdown_pct > RISK_PARAMS['max_drawdown']:
                logger.warning(f"⚠️  Drawdown máximo atingido: {drawdown_pct:.1%}")
                return False
        
        return self.state.is_trading_allowed
    
    def _apply_filters(self, prediction: BayesianPrediction, recent_candles: List) -> tuple[bool, str]:
        """Aplica filtros inteligentes baseados em análise de dados reais
        
        Retorna:
            (bool, str): (pode_tradear, motivo_se_nao)
        """
        if not self.filters.get('enabled', False):
            return True, ""  # Filtros desabilitados
        
        # Filtro 1: Horário (baseado em análise: alguns horários têm <45% WR)
        if self.filters.get('filter_by_hour', False):
            current_hour = datetime.utcnow().hour
            blocked_hours = self.filters.get('blocked_hours', [])
            
            if current_hour in blocked_hours:
                return False, f"Horário bloqueado ({current_hour}h - baixo win rate histórico)"
            
            preferred_hours = self.filters.get('preferred_hours', [])
            if preferred_hours and current_hour not in preferred_hours:
                return False, f"Fora dos horários preferidos ({current_hour}h)"
        
        # Filtro 2: Direção DOWN (análise mostrou 42.9% WR vs 53.3% UP)
        if not self.filters.get('allow_down_trades', True):
            if prediction.get_direction() == 'DOWN':
                return False, "Trades DOWN desabilitados (baixo win rate histórico: 42.9%)"
        
        # Filtro 3: Price vs Strike (trades <0.5% do strike têm 37.8% WR)
        price_vs_strike_min = self.filters.get('price_vs_strike_min', 0.0)
        if price_vs_strike_min > 0 and recent_candles:
            current_price = recent_candles[-1].get('close', 0)
            # Nota: strike_price deveria vir do mercado, mas por enquanto skipamos
            # TODO: Adicionar strike_price ao contexto
        
        # Filtro 4: Momentum mínimo (trades com momentum >0.05% têm melhor WR)
        momentum_min = self.filters.get('momentum_5m_min', 0.0)
        if momentum_min > 0 and recent_candles and len(recent_candles) >= 5:
            # Calcula momentum dos últimos 5 candles
            recent_5 = recent_candles[-5:]
            if len(recent_5) >= 2:
                price_start = recent_5[0].get('close', 0)
                price_end = recent_5[-1].get('close', 0)
                if price_start > 0:
                    momentum_5m = abs((price_end - price_start) / price_start)
                    if momentum_5m < momentum_min:
                        return False, f"Momentum insuficiente ({momentum_5m*100:.3f}% < {momentum_min*100:.3f}%)"
        
        # Filtro 5: RSI extremos
        rsi_min = self.filters.get('rsi_min', 0)
        rsi_max = self.filters.get('rsi_max', 100)
        # RSI vem do prediction se disponível
        # TODO: Adicionar RSI ao contexto se necessário
        
        return True, ""  # Passou por todos os filtros
    
    async def _update_positions_pnl(self, update: MarketData):
        """Atualiza P&L das posições"""
        for position in self.state.positions:
            if position.market_id == update.asset_id and position.status == "OPEN":
                if update.price:
                    position.update_pnl(update.price)
    
    async def run(self):
        """Executa o bot principal"""
        logger.info("🚀 Iniciando Trading Bot...")
        
        # Inicializa
        await self.initialize()
        
        # Cria tasks para WebSockets
        tasks = [
            asyncio.create_task(self.poly_ws.start(), name="Polymarket WS"),
            asyncio.create_task(self.binance_ws.start(), name="Binance WS"),
            asyncio.create_task(self._monitoring_loop(), name="Monitoring")
        ]
        
        try:
            # Aguarda todas as tasks
            await asyncio.gather(*tasks)
            
        except KeyboardInterrupt:
            logger.info("\n⏸️  Parando bot...")
            
        finally:
            # Cleanup
            if self.poly_ws:
                await self.poly_ws.close()
            if self.binance_ws:
                await self.binance_ws.close()
            
            # Mostra resumo
            self._print_summary()
    
    async def _monitoring_loop(self):
        """Loop de monitoramento e logging"""
        while True:
            await asyncio.sleep(60)  # A cada 1 minuto
            
            # Log status
            open_positions = self.state.get_open_positions_count()
            win_rate = self.state.get_win_rate()
            
            logger.info(
                f"📊 Status: Bankroll=${self.state.bankroll:.2f}, "
                f"P&L=${self.state.total_pnl:.2f}, "
                f"Posições={open_positions}, "
                f"Trades={self.state.trades_count}, "
                f"Win Rate={win_rate:.1f}%"
            )
    
    def _print_summary(self):
        """Imprime resumo final"""
        print("\n" + "="*60)
        print("📈 RESUMO FINAL DO BOT")
        print("="*60)
        print(f"Bankroll Inicial: ${self.state.bankroll:.2f}")
        print(f"P&L Total: ${self.state.total_pnl:.2f}")
        print(f"Trades Executados: {self.state.trades_count}")
        print(f"Wins: {self.state.wins}")
        print(f"Losses: {self.state.losses}")
        print(f"Win Rate: {self.state.get_win_rate():.1f}%")
        print("="*60 + "\n")
