"""
Kelly Criterion Otimizado para dimensionamento de posição
Com ajuste dinâmico baseado em performance e gerenciamento de risco
"""
import logging
from typing import Optional, Tuple
from dataclasses import dataclass
from datetime import datetime, timedelta

from config import KELLY_PARAMS, RISK_PARAMS
from bayesian_model import BayesianPrediction

logger = logging.getLogger(__name__)


@dataclass
class KellyResult:
    """Resultado do cálculo de Kelly"""
    kelly_fraction: float
    position_size: float
    edge: float
    should_bet: bool
    reason: str
    confidence: float
    direction: str
    kelly_fraction_full: float = 0.0
    pnl_win_usd: float = 0.0
    pnl_lose_usd: float = 0.0
    adjusted_kelly: bool = False
    
    def __repr__(self):
        return (
            f"Kelly(dir={self.direction}, size=${self.position_size:.2f}, "
            f"fraction={self.kelly_fraction:.3f}, "
            f"edge={self.edge:.3f}, "
            f"conf={self.confidence:.1%}, "
            f"bet={self.should_bet})"
        )


class KellyCriterion:
    """
    Kelly Criterion Otimizado com ajustes dinâmicos
    
    Características adicionais:
    - Redução automática após losses consecutivos
    - Proteção contra drawdown
    - Ajuste por volatilidade do mercado
    - Limite de exposição diária
    """
    
    def __init__(self, bankroll: float):
        """
        Args:
            bankroll: Bankroll total disponível em USD
        """
        self.bankroll = bankroll
        self.peak_bankroll = bankroll  # NOVO: para calcular drawdown
        self.initial_bankroll = bankroll
        
        # Parâmetros base
        self.kelly_fraction_multiplier = KELLY_PARAMS['kelly_fraction']
        self.min_edge = KELLY_PARAMS['min_edge']
        self.max_position_size = KELLY_PARAMS['max_position_size']
        self.min_position_size = KELLY_PARAMS['min_position_size']
        self.max_bankroll_pct = KELLY_PARAMS['max_bankroll_per_trade']
        
        # NOVOS PARÂMETROS DE CONTROLE
        self.consecutive_losses = 0
        self.consecutive_wins = 0
        self.total_trades = 0
        self.wins = 0
        self.losses = 0
        
        # Histórico recente para análise
        self.recent_trades: list[dict] = []  # Últimos 20 trades
        self.daily_loss = 0.0
        self.last_trade_time = None
        
        # NOVOS LIMITES DO RISK_PARAMS
        self.max_consecutive_losses = RISK_PARAMS['max_consecutive_losses']
        self.loss_cooldown = RISK_PARAMS['loss_cooldown_minutes']
        self.max_drawdown = RISK_PARAMS['max_drawdown']
        
        logger.info(
            f"💰 Kelly Criterion Otimizado: Bankroll=${bankroll:.2f}, "
            f"Fração base={self.kelly_fraction_multiplier}"
        )
    
    def calculate(
        self,
        prediction: BayesianPrediction,
        market_price: float,
        token_yes_price: float,
        token_no_price: float
    ) -> KellyResult:
        """
        Calcula tamanho ótimo de posição com ajustes dinâmicos
        
        Args:
            prediction: Predição bayesiana
            market_price: Preço atual no mercado (0-1)
            token_yes_price: Preço do token "YES"
            token_no_price: Preço do token "NO"
        
        Returns:
            KellyResult com tamanho de posição recomendado
        """
        direction = prediction.get_direction()
        
        # Probabilidade prevista pelo modelo
        p = prediction.p_up if direction == "UP" else prediction.p_down
        confidence = prediction.confidence
        
        # Probabilidade do mercado
        market_prob = market_price if direction == "UP" else (1 - market_price)
        
        # Proteção contra odds extremas
        if market_prob <= 0.01 or market_prob >= 0.99:
            return self._no_bet_result(
                "Preço de mercado extremo", 
                confidence,
                edge=abs(p - market_prob),
                direction=direction
            )
        
        # Calcula edge base
        edge = p - market_prob
        
        # VALIDAÇÕES INICIAIS
        if not self._validate_market_conditions(prediction, market_prob):
            return self._no_bet_result(
                "Condições de mercado inválidas",
                confidence,
                edge,
                direction
            )
        
        # VERIFICA COOLDOWN APÓS PERDAS
        if not self._check_cooldown():
            return self._no_bet_result(
                "Em cooldown após perdas",
                confidence,
                edge,
                direction
            )
        
        # VERIFICA SEQUÊNCIA DE LOSSES
        if self.consecutive_losses >= self.max_consecutive_losses:
            logger.warning(
                f"⚠️ {self.consecutive_losses} losses consecutivos - pausando trades"
            )
            return self._no_bet_result(
                f"{self.consecutive_losses} losses consecutivos",
                confidence,
                edge,
                direction
            )
        
        # VERIFICA DRAWDOWN
        current_drawdown = self._get_current_drawdown()
        if current_drawdown >= self.max_drawdown:
            logger.warning(f"⚠️ Drawdown de {current_drawdown:.1%} - protegendo bankroll")
            return self._no_bet_result(
                f"Drawdown máximo atingido: {current_drawdown:.1%}",
                confidence,
                edge,
                direction
            )
        
        # Edge mínimo base
        if edge < self.min_edge:
            return self._no_bet_result(
                f"Edge insuficiente: {edge:.3f} < {self.min_edge:.3f}",
                confidence,
                edge,
                direction
            )
        
        # Calcula odds
        odds = (1 / market_prob) - 1
        
        # Kelly Criterion full
        q = 1 - p
        kelly_full = ((p * odds) - q) / odds
        
        if kelly_full <= 0:
            return self._no_bet_result(
                "Kelly negativo",
                confidence,
                edge,
                direction
            )
        
        # APLICA FATORES DE AJUSTE DINÂMICOS
        adjusted_multiplier = self._get_dynamic_multiplier(current_drawdown)
        
        # Kelly fracionário com ajustes
        kelly_fractional = kelly_full * adjusted_multiplier
        
        # Calcula posição base
        raw_position_size = self.bankroll * kelly_fractional
        
        # APLICA LIMITES ADICIONAIS
        position_size = self._apply_enhanced_limits(
            raw_position_size, 
            confidence,
            edge
        )

        # PnL esperado pelas odds reais do CLOB
        entry_price = token_yes_price if direction == "UP" else token_no_price
        entry_price = max(entry_price, 0.01)
        tokens      = position_size / entry_price
        pnl_win     = tokens * 1.0 - position_size
        pnl_lose    = -position_size
        
        # Verifica tamanho mínimo — usa piso em vez de rejeitar
        # Com bankroll pequeno, Kelly pode sugerir menos do que o mínimo de ordem do mercado
        # (5 shares × $0.50 = $2.50). Se o edge passou em tudo, usa o mínimo como piso.
        if position_size < self.min_position_size:
            logger.info(
                f"📏 Kelly: ${position_size:.2f} < mín ${self.min_position_size:.2f} "
                f"→ usando mínimo (over-Kelly por constraint de ordem)"
            )
            position_size = self.min_position_size

        # LOG DETALHADO DOS AJUSTES
        logger.info(
            f"✅ Kelly: Edge={edge:.3f}, Conf={confidence:.1%}, "
            f"Kelly full={kelly_full:.3f}, Multiplier={adjusted_multiplier:.2f}, "
            f"Final={kelly_fractional:.3f}, Size=${position_size:.2f}"
        )
        
        return KellyResult(
            kelly_fraction=kelly_fractional,
            position_size=position_size,
            edge=edge,
            confidence=confidence,
            should_bet=True,
            adjusted_kelly=(adjusted_multiplier != self.kelly_fraction_multiplier),
            reason="Kelly positivo com ajustes dinâmicos",
            direction=direction,
            kelly_fraction_full=kelly_full,
            pnl_win_usd=round(pnl_win, 2),
            pnl_lose_usd=round(pnl_lose, 2)
        )
    
    def _get_dynamic_multiplier(self, current_drawdown: float) -> float:
        """
        Calcula multiplicador dinâmico baseado em:
        - Sequência de losses/wins
        - Drawdown atual
        - Volatilidade implícita
        """
        multiplier = self.kelly_fraction_multiplier  # Base: 0.25
        
        # 1. AJUSTE POR SEQUÊNCIA DE LOSSES
        if self.consecutive_losses >= 2:
            # Reduz posição após 2+ losses
            loss_reduction = KELLY_PARAMS.get('loss_reduction_factor', 0.5)
            multiplier *= loss_reduction
            logger.debug(f"📉 Redução por losses: x{loss_reduction}")
        
        # 2. AJUSTE POR SEQUÊNCIA DE WINS (com limite)
        if self.consecutive_wins >= 3:
            # Aumenta posição após 3+ wins (mas limitado)
            win_increase = KELLY_PARAMS.get('win_increase_factor', 1.1)
            multiplier = min(multiplier * win_increase, 0.30)  # Máx 30%
            logger.debug(f"📈 Aumento por wins: x{win_increase}")
        
        # 3. AJUSTE POR DRAWDOWN
        if current_drawdown > 0.10:  # >10% drawdown
            drawdown_factor = 1.0 - (current_drawdown - 0.10) * 2
            drawdown_factor = max(drawdown_factor, 0.3)  # Mínimo 30% do normal
            multiplier *= drawdown_factor
            logger.debug(f"📊 Ajuste por drawdown ({current_drawdown:.1%}): x{drawdown_factor:.2f}")
        
        # 4. REDUÇÃO POR CONFIANÇA BAIXA
        # (já é considerado no modelo, mas podemos ser mais conservadores)
        
        return multiplier
    
    def _apply_enhanced_limits(
        self,
        raw_size: float,
        confidence: float,
        edge: float
    ) -> float:
        """
        Aplica limites avançados de posição
        """
        size = raw_size
        
        # 1. Limite absoluto
        size = min(size, self.max_position_size)
        
        # 2. Limite por % do bankroll
        max_by_bankroll = self.bankroll * self.max_bankroll_pct
        size = min(size, max_by_bankroll)
        
        # 3. LIMITE POR CONFIANÇA (apenas reduz, nunca aumenta)
        confidence_multiplier = min(confidence / 0.55, 1.0)  # nunca exceede 1.0x
        size = min(size, self.max_position_size * confidence_multiplier)
        
        # 4. HARD CAP FINAL: bankroll% — deve ser sempre a última barreira
        # Garante que nenhum fator anterior (confiança, edge) pode ultrapassar este limite
        hard_cap = self.bankroll * self.max_bankroll_pct
        size = min(size, hard_cap)
        
        # Arredonda
        size = round(size, 2)
        
        if size != raw_size:
            logger.debug(f"Posição limitada: ${raw_size:.2f} -> ${size:.2f}")
        
        return size
    
    def _validate_market_conditions(
        self,
        prediction: BayesianPrediction,
        market_prob: float
    ) -> bool:
        """
        Valida condições de mercado adicionais.
        Exige ao menos 1 sinal FORTE (prob > min_signal_strength) na direção prevista.
        Isso evita que o bot entre puramente com base no prior sem sinal real.
        """
        # Verifica se as probabilidades são razoáveis
        if market_prob < 0.05 or market_prob > 0.95:
            logger.debug("⚠️ Probabilidade de mercado extrema - cautela")

        # Mínimo de sinais
        if len(prediction.signals) < 1:
            logger.debug("⚠️ Nenhum sinal disponível")
            return False

        # Exige ao menos 1 sinal forte na direção prevista
        from config import BAYESIAN_PARAMS as _bp
        min_strength = _bp.get('min_signal_strength', 0.60)
        predicting_up = prediction.p_up > prediction.p_down
        has_strong = any(
            (predicting_up and s.p_up >= min_strength) or
            (not predicting_up and s.p_down >= min_strength)
            for s in prediction.signals
        )
        if not has_strong:
            logger.debug(
                f"⚠️ Nenhum sinal forte para {'UP' if predicting_up else 'DOWN'} "
                f"(min={min_strength:.0%}) — ignorando entrada baseada só em prior"
            )
            return False

        return True
    
    def _check_cooldown(self) -> bool:
        """
        Verifica se está em período de cooldown após perdas
        """
        if self.consecutive_losses == 0 or not self.last_trade_time:
            return True
        
        # Cooldown apenas após 2+ losses consecutivos
        if self.consecutive_losses >= 2:
            time_since_last = datetime.utcnow() - self.last_trade_time
            cooldown_seconds = self.loss_cooldown * 60
            
            if time_since_last.total_seconds() < cooldown_seconds:
                remaining = cooldown_seconds - time_since_last.total_seconds()
                logger.debug(f"⏳ Cooldown: {remaining:.0f}s restantes")
                return False
        
        return True
    
    def _get_current_drawdown(self) -> float:
        """Calcula drawdown atual em relação ao pico"""
        if self.peak_bankroll <= 0:
            return 0.0
        return (self.peak_bankroll - self.bankroll) / self.peak_bankroll
    
    def _no_bet_result(self, reason: str, confidence: float, edge: float, direction: str) -> KellyResult:
        """Cria resultado indicando não apostar"""
        return KellyResult(
            kelly_fraction=0.0,
            position_size=0.0,
            edge=edge,
            confidence=confidence,
            should_bet=False,
            reason=reason,
            direction=direction
        )
    
    def update_after_trade(
        self,
        won: bool,
        position_size: float,
        payout: float,
        prediction: Optional[BayesianPrediction] = None
    ):
        """
        Atualiza estado após trade - VERSÃO MELHORADA
        
        Args:
            won: Se ganhou o trade
            position_size: Tamanho da posição
            payout: Valor recebido (se ganhou)
            prediction: Predição original (opcional, para análise)
        """
        # Calcula P&L
        if won:
            pnl = payout - position_size
            self.wins += 1
            self.consecutive_wins += 1
            self.consecutive_losses = 0
        else:
            pnl = -position_size
            self.losses += 1
            self.consecutive_losses += 1
            self.consecutive_wins = 0
        
        self.total_trades += 1
        self.bankroll += pnl
        self.peak_bankroll = max(self.peak_bankroll, self.bankroll)
        self.last_trade_time = datetime.utcnow()
        
        # Atualiza daily loss
        if pnl < 0:
            self.daily_loss += abs(pnl)
        
        # Registra no histórico recente
        self.recent_trades.append({
            'won': won,
            'pnl': pnl,
            'size': position_size,
            'time': self.last_trade_time,
            'confidence': prediction.confidence if prediction else None
        })
        
        # Mantém apenas últimos 20
        if len(self.recent_trades) > 20:
            self.recent_trades.pop(0)
        
        # Log detalhado
        win_rate = (self.wins / self.total_trades * 100) if self.total_trades > 0 else 0
        logger.info(
            f"💰 Trade {'✅' if won else '❌'} | "
            f"P&L: ${pnl:+.2f} | Bankroll: ${self.bankroll:.2f} | "
            f"WR: {win_rate:.1f}% | "
            f"Seq: W{self.consecutive_wins} L{self.consecutive_losses}"
        )
    
    def get_risk_metrics(self) -> dict:
        """
        Retorna métricas de risco atuais
        """
        win_rate = (self.wins / self.total_trades * 100) if self.total_trades > 0 else 0
        
        # Calcula Sharpe ratio aproximado
        if len(self.recent_trades) >= 5:
            returns = [t['pnl'] / t['size'] for t in self.recent_trades if t['size'] > 0]
            avg_return = np.mean(returns) if returns else 0
            std_return = np.std(returns) if len(returns) > 1 else 1
            sharpe = avg_return / std_return if std_return > 0 else 0
        else:
            sharpe = 0
        
        return {
            'bankroll': self.bankroll,
            'peak_bankroll': self.peak_bankroll,
            'drawdown': self._get_current_drawdown(),
            'total_trades': self.total_trades,
            'wins': self.wins,
            'losses': self.losses,
            'win_rate': win_rate,
            'consecutive_wins': self.consecutive_wins,
            'consecutive_losses': self.consecutive_losses,
            'daily_loss': self.daily_loss,
            'sharpe_ratio': sharpe,
            'in_cooldown': not self._check_cooldown()
        }
    
    def reset_daily(self):
        """Reseta contadores diários"""
        self.daily_loss = 0.0
        logger.info("📅 Reset diário realizado")
    
    def get_max_loss(self, position_size: float) -> float:
        """Calcula perda máxima da posição"""
        return position_size
    
    def get_expected_value(
        self,
        position_size: float,
        p_win: float,
        market_price: float
    ) -> float:
        """
        Calcula valor esperado da aposta
        """
        profit = position_size * ((1 / market_price) - 1)
        loss = position_size
        ev = (p_win * profit) - ((1 - p_win) * loss)
        return ev
    
    def validate_bet(
        self,
        kelly_result: KellyResult,
        current_positions: int
    ) -> Tuple[bool, str]:
        """
        Valida se deve executar a aposta considerando contexto
        
        Args:
            kelly_result: Resultado do cálculo de Kelly
            current_positions: Número de posições abertas
        
        Returns:
            (should_bet, reason)
        """
        from config import RISK_PARAMS
        max_positions = RISK_PARAMS['max_concurrent_positions']
        
        # Já decidiu não apostar
        if not kelly_result.should_bet:
            return False, kelly_result.reason
        
        # Verifica número de posições
        if current_positions >= max_positions:
            return False, f"Máximo de posições atingido ({current_positions}/{max_positions})"
        
        # Verifica exposição
        position_pct = (kelly_result.position_size / self.bankroll) * 100
        if position_pct > self.max_bankroll_pct * 100:
            return False, f"Posição muito grande: {position_pct:.1f}% do bankroll"
        
        # Verifica se não está em cooldown
        if not self._check_cooldown():
            return False, "Em período de cooldown"
        
        return True, "Validações passaram"
    
    def calculate_roi(self, won: bool, position_size: float, payout: float) -> float:
        """Calcula ROI da aposta"""
        if won:
            profit = payout - position_size
            return (profit / position_size) * 100
        return -100.0