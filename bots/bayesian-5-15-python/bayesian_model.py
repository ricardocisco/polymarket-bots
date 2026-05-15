"""
Modelo Bayesiano Otimizado para predição de direção de preço
Com pesos dinâmicos, penalidades e limites mais rigorosos
"""
import logging
import numpy as np
from typing import Dict, List, Tuple, Optional
from dataclasses import dataclass
from datetime import datetime
import json
import os

from config import BAYESIAN_PARAMS, FEATURES_CONFIG
from binance_ws import BinanceKline

logger = logging.getLogger(__name__)


@dataclass
class BayesianSignal:
    """Sinal bayesiano com probabilidade"""
    name: str
    p_up: float
    p_down: float
    confidence: float
    weight: float
    raw_value: float  # Valor original do indicador
    timestamp: datetime


@dataclass
class BayesianPrediction:
    """Predição final do modelo"""
    symbol: str
    p_up: float
    p_down: float
    confidence: float
    edge: float
    signals: List[BayesianSignal]
    strike_price: Optional[float]
    current_price: Optional[float]
    timestamp: datetime
    
    def should_trade(self) -> bool:
        """Verifica se deve fazer trade baseado em EDGE real (|p_up - p_down|)"""
        min_edge = BAYESIAN_PARAMS.get('min_trade_edge', 0.08)
        return self.edge >= min_edge
    
    def get_direction(self) -> str:
        # Requer diferença mínima de 3% para evitar bias
        if abs(self.p_up - self.p_down) < 0.03:
            # Se muito próximo, favorece DOWN (que está sub-representado)
            return "DOWN" if self.p_down >= self.p_up else "UP"
        return "UP" if self.p_up > self.p_down else "DOWN"


class OptimizedBayesianModel:
    """
    Modelo Bayesiano Otimizado com pesos dinâmicos e penalidades
    """
    
    def __init__(self, trades_log_path: str = "trades_log.json"):
        self.prior_up = BAYESIAN_PARAMS['prior_up']
        self.prior_down = BAYESIAN_PARAMS['prior_down']
        
        # Pesos base
        self.base_weights = {
            'momentum': BAYESIAN_PARAMS['momentum_weight'],
            'volume': BAYESIAN_PARAMS['volume_weight'],
            'volatility': BAYESIAN_PARAMS['volatility_weight'],
            'trend': BAYESIAN_PARAMS['trend_weight']
        }
        
        # Thresholds otimizados
        self.rsi_overbought = FEATURES_CONFIG['momentum']['rsi_overbought']  # 65
        self.rsi_oversold = FEATURES_CONFIG['momentum']['rsi_oversold']      # 35
        self.rsi_extreme = FEATURES_CONFIG['momentum']['rsi_extreme']        # 75
        
        self.prediction_history: List[Dict] = []
        self.consecutive_losses = 0
        self.consecutive_wins = 0

        self.train_from_log(trades_log_path)

    def train_from_log(self, log_path: str, interval_filter: Optional[int] = None):
        """
        Aprende com o histórico de trades do trades_log.json para ajustar pesos
        e priors.
        
        Args:
            log_path: Caminho do arquivo de log
            interval_filter: Se especificado, treina apenas com trades desse intervalo (5 ou 15)
        """
        if not os.path.exists(log_path):
            logger.warning(f"Arquivo de log '{log_path}' não encontrado. Usando pesos padrão.")
            return

        try:
            with open(log_path, 'r', encoding='utf-8') as f:
                log_data = json.load(f)
        except (FileNotFoundError, json.JSONDecodeError) as e:
            logger.warning(f"Não foi possível carregar ou decodificar {log_path}: {e}. Usando pesos padrão.")
            return

        trades = log_data.get('trades', [])
        if not trades:
            logger.info("Nenhum trade histórico encontrado no log. Usando pesos padrão.")
            return

        resolved_trades = [
            t for t in trades if t and t.get("resolution") and t.get("resolution").get("resolved")
        ]
        
        # Filtra por intervalo se especificado
        if interval_filter:
            resolved_trades = [
                t for t in resolved_trades 
                if t.get('market', {}).get('interval_minutes') == interval_filter
            ]
            logger.info(f"📊 Filtrando trades de {interval_filter}min: {len(resolved_trades)} encontrados")
        
        if not resolved_trades:
            logger.info("Nenhum trade resolvido encontrado para treinamento.")
            return

        # Desempenho dos sinais usando VALORES RAW
        signal_performance = {
            'momentum': {'correct': 0, 'total': 0, 'profitable': 0},
            'trend': {'correct': 0, 'total': 0, 'profitable': 0},
            'volume': {'correct': 0, 'total': 0, 'profitable': 0},
            'volatility': {'correct': 0, 'total': 0, 'profitable': 0},
            'strike_distance': {'correct': 0, 'total': 0, 'profitable': 0}  # NOVO
        }
        
        # Mapeamento de nomes de sinais antigos para novos  
        signal_map = {'rsi': 'momentum', 'ema': 'trend', 'volume': 'volume', 'atr': 'volatility'}

        for trade in resolved_trades:
            entry = trade.get('entry', {})
            resolution = trade.get('resolution', {})
            signals_log = entry.get('bayes', {}).get('signals', {})
            actual_outcome = resolution.get('resolved_direction')
            is_profitable = resolution.get('pnl_usd', 0) > 0

            if not actual_outcome:
                continue

            # Analisa cada sinal usando valores RAW
            for log_name, model_name in signal_map.items():
                if log_name in signals_log:
                    # Usa o valor RAW para determinar direção
                    raw_value = signals_log[log_name].get('value', signals_log[log_name].get('raw_value'))
                    
                    if raw_value is not None:
                        # Determina direção baseado no valor RAW
                        if log_name == 'rsi':
                            signal_direction = "DOWN" if raw_value > 65 else "UP" if raw_value < 35 else None
                        elif log_name == 'ema':
                            signal_direction = "UP" if raw_value > 0 else "DOWN" if raw_value < 0 else None
                        elif log_name == 'volume':
                            # Volume não tem direção sozinho, usa mudança de preço
                            price_change = entry.get('price', {}).get('change_1m_pct', 0)
                            if raw_value > 1.5:  # Volume alto
                                signal_direction = "UP" if price_change > 0 else "DOWN"
                            else:
                                signal_direction = None
                        elif log_name == 'atr':
                            # ATR alto = mais volatilidade, não define direção sozinho
                            signal_direction = None
                        else:
                            signal_direction = None
                        
                        if signal_direction and model_name in signal_performance:
                            signal_performance[model_name]['total'] += 1
                            if signal_direction == actual_outcome:
                                signal_performance[model_name]['correct'] += 1
                            if is_profitable:
                                signal_performance[model_name]['profitable'] += 1
            
            # NOVO: Analisa distância do strike
            strike_price = entry.get('price', {}).get('strike_price', 0)
            current_price = entry.get('price', {}).get('current_price', 0)
            predicted_direction = entry.get('bayes', {}).get('direction', '')
            
            if strike_price > 0 and current_price > 0 and predicted_direction:
                strike_distance_pct = (current_price - strike_price) / strike_price * 100
                
                signal_performance['strike_distance']['total'] += 1
                
                # Analisa se a distância do strike foi um bom indicador
                if predicted_direction == "UP" and strike_distance_pct > -0.5:  # Perto ou acima
                    if actual_outcome == "UP":
                        signal_performance['strike_distance']['correct'] += 1
                elif predicted_direction == "DOWN" and strike_distance_pct < 0.5:  # Perto ou abaixo
                    if actual_outcome == "DOWN":
                        signal_performance['strike_distance']['correct'] += 1
                
                if is_profitable:
                    signal_performance['strike_distance']['profitable'] += 1
        
        # Atualiza pesos baseado na performance E lucratividade
        accuracies = {}
        total_accuracy_sum = 0
        
        for signal, perf in signal_performance.items():
            if perf['total'] > 30:  # Reduzido para 30 trades
                accuracy = perf['correct'] / perf['total']
                profit_rate = perf['profitable'] / perf['total'] if perf['total'] > 0 else 0.5
                
                # Combina acurácia e lucratividade (60% acurácia, 40% profit)
                combined_score = (accuracy * 0.6) + (profit_rate * 0.4)
                
                accuracies[signal] = combined_score
                total_accuracy_sum += combined_score
                
                logger.info(
                    f"📊 Sinal '{signal}': "
                    f"Acurácia={accuracy:.1%} Lucro={profit_rate:.1%} "
                    f"Score={combined_score:.1%} (n={perf['total']})"
                )
            else:
                accuracies[signal] = 0.5
                total_accuracy_sum += 0.5

        # Normaliza os pesos - dá mais peso aos sinais que funcionam
        if total_accuracy_sum > 0:
            original_total_weight = sum(self.base_weights.values())
            for signal, score in accuracies.items():
                if signal in self.base_weights:
                    # Amplifica diferenças: sinais ruins recebem muito menos peso
                    if score < 0.48:  # Pior que aleatório
                        self.base_weights[signal] = 0.05  # Peso mínimo
                    elif score > 0.55:  # Melhor que aleatório
                        self.base_weights[signal] = (score / total_accuracy_sum) * original_total_weight * 1.5
                    else:
                        self.base_weights[signal] = (score / total_accuracy_sum) * original_total_weight
                elif signal == 'strike_distance':
                    # Adiciona peso para strike distance se ele for bom
                    if score > 0.52:
                        logger.info(f"✅ Strike distance é um bom preditor! Score: {score:.1%}")
        
        logger.info(f"🧠 Modelo treinado com {len(resolved_trades)} trades resolvidos.")
        formatted_weights = {k: f"{v:.3f}" for k, v in self.base_weights.items()}
        logger.info(f"⚖️ Novos pesos dos sinais: {formatted_weights}")
    
    def predict(
        self,
        symbol: str,
        recent_candles: List[BinanceKline],
        strike_price: Optional[float] = None,
        current_price: Optional[float] = None,
        minutes_to_expiry: Optional[float] = None
    ) -> BayesianPrediction:
        """Gera predição bayesiana otimizada com features críticas"""
        if len(recent_candles) < 15:
            return self._default_prediction(symbol, strike_price, current_price)
        
        signals = []
        raw_values = {}
        
        # FEATURE CRÍTICA 1: Distância do Strike
        if strike_price is not None and current_price is not None:
            strike_distance_pct = (current_price - strike_price) / strike_price * 100
            raw_values['strike_distance'] = strike_distance_pct
            
            # Penaliza fortemente se está muito longe na direção errada
            if abs(strike_distance_pct) > 0.5:  # Mais de 0.5% de distância
                strike_signal = self._calculate_strike_signal(strike_distance_pct)
                if strike_signal:
                    signals.append(strike_signal)
        
        # FEATURE CRÍTICA 2: Tempo até expiração
        if minutes_to_expiry is not None:
            raw_values['minutes_to_expiry'] = minutes_to_expiry
            
            # Em mercados de 5-15min, momentum de curtíssimo prazo importa mais
            if minutes_to_expiry < 3:  # Últimos 3 minutos
                # Aumenta peso do momentum de 1min
                if len(recent_candles) >= 3:
                    price_momentum_1m = (recent_candles[-1].close - recent_candles[-3].close) / recent_candles[-3].close * 100
                    if abs(price_momentum_1m) > 0.1:  # Movimento significativo
                        momentum_short_signal = self._calculate_short_momentum_signal(price_momentum_1m)
                        if momentum_short_signal:
                            signals.append(momentum_short_signal)
        
        # 1. Momentum (RSI) - MAIS AGRESSIVO
        if FEATURES_CONFIG['momentum']['enabled']:
            momentum_signal = self._calculate_momentum_signal(recent_candles)
            if momentum_signal:
                signals.append(momentum_signal)
                raw_values['rsi'] = momentum_signal.raw_value
        
        # 2. Volume - COM DETECÇÃO DE EXTREMOS
        if FEATURES_CONFIG['volume_profile']['enabled']:
            volume_signal = self._calculate_volume_signal(recent_candles)
            if volume_signal:
                signals.append(volume_signal)
                raw_values['volume_ratio'] = volume_signal.raw_value
        
        # 3. Volatilidade
        if FEATURES_CONFIG['volatility']['enabled']:
            volatility_signal = self._calculate_volatility_signal(recent_candles)
            if volatility_signal:
                signals.append(volatility_signal)
                raw_values['vol_ratio'] = volatility_signal.raw_value
        
        # 4. Trend - COM LIMITE MÁXIMO
        if FEATURES_CONFIG['trend']['enabled']:
            trend_signal = self._calculate_trend_signal(recent_candles)
            if trend_signal:
                signals.append(trend_signal)
                raw_values['ema_strength'] = trend_signal.raw_value
        
        # APLICA PESOS DINÂMICOS
        signals = self._apply_dynamic_weights(signals, raw_values)
        
        # COMBINA SINAIS
        p_up, p_down = self._combine_signals(signals)
        
        # APLICA PENALIDADE POR STRIKE
        if strike_price is not None and current_price is not None:
            p_up, p_down = self._apply_strike_penalty(
                p_up, p_down, strike_price, current_price
            )
        
        confidence = max(p_up, p_down)
        edge = abs(p_up - p_down)
        
        prediction = BayesianPrediction(
            symbol=symbol,
            p_up=p_up,
            p_down=p_down,
            confidence=confidence,
            edge=edge,
            signals=signals,
            strike_price=strike_price,
            current_price=current_price,
            timestamp=datetime.utcnow()
        )
        
        return prediction
    
    def _calculate_momentum_signal(self, candles: List[BinanceKline]) -> Optional[BayesianSignal]:
        """
        RSI calibrado para mercados binários UP/DOWN de 5-15min.

        Zona neutra (rsi_neutral_low a rsi_neutral_high): sem sinal — evita ruído.
        RSI extremo overbought (>rsi_extreme): forte momentum UP.  
        RSI overbought (>rsi_overbought): momentum UP.
        RSI oversold (<rsi_oversold): sinal UP contrarian — dados históricos mostram
          que preços sobvevendidos tendem a reverter (DOWN era apostado e errava 67%).
        """
        try:
            closes = np.array([c.close for c in candles])
            rsi = self._calculate_rsi(closes, 14)

            neutral_low  = FEATURES_CONFIG['momentum'].get('rsi_neutral_low',  42)
            neutral_high = FEATURES_CONFIG['momentum'].get('rsi_neutral_high', 58)

            if neutral_low <= rsi <= neutral_high:
                # Zona neutra — RSI não fornece sinal confiável
                return None

            if rsi > self.rsi_extreme:  # >78
                # Extremamente sobrecomprado → forte momentum UP
                extreme_factor = min((rsi - self.rsi_extreme) / 10, 0.15)
                p_up = min(0.82, 0.72 + extreme_factor)
                p_down = 1 - p_up
            elif rsi > self.rsi_overbought:  # 68-78
                # Sobrecomprado → momentum UP
                strength = (rsi - self.rsi_overbought) / (self.rsi_extreme - self.rsi_overbought)
                p_up = 0.65 + (strength * 0.07)
                p_down = 1 - p_up
            elif rsi > neutral_high:  # 58-68
                # Levemente bullish → leve viés UP
                p_up = 0.57
                p_down = 0.43
            elif rsi < self.rsi_oversold:  # <32
                # Sobrevendido — historicamente neste mercado o preço reverte UP
                # Apostar DOWN aqui estava errado 67% das vezes
                oversold_depth = (self.rsi_oversold - rsi) / self.rsi_oversold
                p_up = 0.60 + min(oversold_depth * 0.15, 0.12)  # contrarian UP
                p_down = 1 - p_up
            else:  # neutral_low-32 (32-42)
                # Levemente bearish mas sem sinal forte
                p_up = 0.54
                p_down = 0.46

            confidence = abs(rsi - 50) / 50

            return BayesianSignal(
                name='momentum',
                p_up=p_up,
                p_down=p_down,
                confidence=confidence,
                weight=self.base_weights['momentum'],
                raw_value=rsi,
                timestamp=datetime.utcnow()
            )

        except Exception as e:
            logger.error(f"Erro no momentum: {e}")
            return None
    
    def _calculate_trend_signal(self, candles: List[BinanceKline]) -> Optional[BayesianSignal]:
        """Trend EMA — exige gap mínimo para não gerar sinal de ruído."""
        try:
            closes = np.array([c.close for c in candles])

            ema_fast = self._calculate_ema(closes, 5)
            ema_slow = self._calculate_ema(closes, 15)

            ema_gap = (ema_fast - ema_slow) / ema_slow * 100  # em %
            max_conf  = FEATURES_CONFIG['trend']['max_confidence']  # 0.72
            min_gap   = FEATURES_CONFIG['trend'].get('min_gap_pct', 0.10)

            # Sem gap real → sem sinal (era a maior fonte de ruído)
            if abs(ema_gap) < min_gap:
                return None

            if ema_fast > ema_slow:
                strength = min(abs(ema_gap) / 0.4, 1.0)
                p_up = 0.55 + (strength * (max_conf - 0.55))
                p_down = 1 - p_up
            else:
                strength = min(abs(ema_gap) / 0.4, 1.0)
                p_down = 0.55 + (strength * (max_conf - 0.55))
                p_up = 1 - p_down

            confidence = min(abs(ema_gap) / 0.3, 1.0)

            return BayesianSignal(
                name='trend',
                p_up=p_up,
                p_down=p_down,
                confidence=confidence,
                weight=self.base_weights['trend'],
                raw_value=ema_gap,
                timestamp=datetime.utcnow()
            )

        except Exception as e:
            logger.error(f"Erro no trend: {e}")
            return None
    
    def _calculate_volume_signal(self, candles: List[BinanceKline]) -> Optional[BayesianSignal]:
        """Volume com detecção de extremos"""
        try:
            volumes = np.array([c.volume for c in candles])
            recent_volume = volumes[-5:].mean()
            avg_volume = volumes.mean()
            
            volume_ratio = recent_volume / avg_volume
            threshold = FEATURES_CONFIG['volume_profile']['relative_threshold']  # 1.5
            extreme_threshold = FEATURES_CONFIG['volume_profile'].get('extreme_threshold', 2.5)
            
            price_change = (candles[-1].close - candles[-5].close) / candles[-5].close
            
            # Volume extremo - sinal muito forte
            if volume_ratio > extreme_threshold:
                if price_change > 0.001:
                    p_up = 0.75
                    p_down = 0.25
                elif price_change < -0.001:
                    p_up = 0.25
                    p_down = 0.75
                else:
                    p_up = 0.60
                    p_down = 0.40
            # Volume alto normal
            elif volume_ratio > threshold:
                if price_change > 0:
                    p_up = 0.65
                    p_down = 0.35
                elif price_change < 0:
                    p_up = 0.35
                    p_down = 0.65
                else:
                    p_up = 0.55
                    p_down = 0.45
            else:
                p_up = 0.50
                p_down = 0.50
            
            confidence = min(volume_ratio / extreme_threshold, 1.0)
            
            return BayesianSignal(
                name='volume',
                p_up=p_up,
                p_down=p_down,
                confidence=confidence,
                weight=self.base_weights['volume'],
                raw_value=volume_ratio,
                timestamp=datetime.utcnow()
            )
            
        except Exception as e:
            logger.error(f"Erro no volume: {e}")
            return None

    def _calculate_volatility_signal(self, candles: List[BinanceKline]) -> Optional[BayesianSignal]:
        """Calcula o sinal de volatilidade usando ATR."""
        try:
            highs = np.array([c.high for c in candles])
            lows = np.array([c.low for c in candles])
            closes = np.array([c.close for c in candles])
            
            if len(candles) < 2:
                return None

            trs = []
            for i in range(1, len(candles)):
                h, l, pc = highs[i], lows[i], closes[i-1]
                trs.append(max(h - l, abs(h - pc), abs(l - pc)))
            
            atr = np.mean(trs[-14:]) # ATR de 14 períodos
            current_price = closes[-1]
            atr_pct = (atr / current_price) * 100 if current_price > 0 else 0

            # Lógica de decisão baseada na volatilidade
            price_change_5m = (closes[-1] - closes[-6]) / closes[-6] if len(closes) >= 6 else 0
            trend_dir = 1 if price_change_5m > 0 else -1

            if atr_pct > 0.15: # Alta volatilidade, favorece reversão
                p_up = 0.50 - trend_dir * 0.08
                p_down = 1 - p_up
            else: # Baixa volatilidade, favorece continuação
                p_up = 0.50 + trend_dir * 0.04
                p_down = 1 - p_up

            confidence = min(atr_pct / 0.2, 1.0) # Confiança aumenta com a volatilidade

            return BayesianSignal(
                name='volatility',
                p_up=p_up,
                p_down=p_down,
                confidence=confidence,
                weight=self.base_weights['volatility'],
                raw_value=atr,
                timestamp=datetime.utcnow()
            )

        except Exception as e:
            logger.error(f"Erro na volatilidade: {e}")
            return None
    
    def _calculate_strike_signal(self, strike_distance_pct: float) -> Optional[BayesianSignal]:
        """Calcula sinal baseado na distância do strike - FEATURE CRÍTICA"""
        try:
            # Se está acima do strike, mais provável subir (momentum)
            # Se está abaixo do strike, mais provável cair
            
            if abs(strike_distance_pct) < 0.1:  # Muito próximo do strike
                # Impossível prever com confiança
                p_up = 0.5
                p_down = 0.5
                confidence = 0.0
            elif strike_distance_pct > 0:  # Acima do strike
                # Mais provável continuar subindo (momentum)
                strength = min(abs(strike_distance_pct) / 2.0, 0.15)  # Max 15%
                p_up = 0.5 + strength
                p_down = 0.5 - strength
                confidence = min(abs(strike_distance_pct) / 1.0, 0.8)
            else:  # Abaixo do strike
                # Mais provável continuar caindo
                strength = min(abs(strike_distance_pct) / 2.0, 0.15)
                p_up = 0.5 - strength
                p_down = 0.5 + strength
                confidence = min(abs(strike_distance_pct) / 1.0, 0.8)
            
            return BayesianSignal(
                name='strike_distance',
                p_up=p_up,
                p_down=p_down,
                confidence=confidence,
                weight=0.25,  # Peso significativo
                raw_value=strike_distance_pct,
                timestamp=datetime.utcnow()
            )
        except Exception as e:
            logger.error(f"Erro no strike signal: {e}")
            return None
    
    def _calculate_short_momentum_signal(self, price_change_pct: float) -> Optional[BayesianSignal]:
        """Momentum de curtíssimo prazo (1-3min) - Importante para mercados rápidos"""
        try:
            # Movimento forte recente tende a continuar por alguns minutos
            strength = min(abs(price_change_pct) / 0.5, 0.2)  # Max 20%
            
            if price_change_pct > 0.1:  # Subindo forte
                p_up = 0.5 + strength
                p_down = 0.5 - strength
            elif price_change_pct < -0.1:  # Caindo forte
                p_up = 0.5 - strength
                p_down = 0.5 + strength
            else:
                return None
            
            confidence = min(abs(price_change_pct) / 1.0, 0.7)
            
            return BayesianSignal(
                name='short_momentum',
                p_up=p_up,
                p_down=p_down,
                confidence=confidence,
                weight=0.20,  # Peso moderado
                raw_value=price_change_pct,
                timestamp=datetime.utcnow()
            )
        except Exception as e:
            logger.error(f"Erro no short momentum: {e}")
            return None
    
    def _apply_dynamic_weights(self, signals: List[BayesianSignal], 
                               raw_values: Dict) -> List[BayesianSignal]:
        """Aplica pesos dinâmicos baseado em condições"""
        if not signals:
            return signals
        
        rsi = raw_values.get('rsi', 50)
        
        # Se RSI está extremo, aumenta peso do momentum
        if rsi > self.rsi_extreme or rsi < (100 - self.rsi_extreme):
            for signal in signals:
                if signal.name == 'momentum':
                    signal.weight *= BAYESIAN_PARAMS.get('rsi_extreme_boost', 1.5)
                    logger.debug(f"⚠️ RSI extremo: {rsi:.1f} - peso momentum = {signal.weight:.2f}")
        
        return signals
    
    def _apply_strike_penalty(self, p_up: float, p_down: float, 
                              strike: float, current: float) -> Tuple[float, float]:
        """Penaliza se preço está longe do strike na direção oposta"""
        diff_pct = (current - strike) / strike * 100  # em %
        predicted_up = p_up > p_down
        penalty_factor = BAYESIAN_PARAMS.get('strike_penalty_factor', 50)
        
        # Penalidade máxima de 10% (aumentado)
        if predicted_up and diff_pct < -0.5:  # Abaixo do strike mas prevê UP
            penalty = min(abs(diff_pct) / penalty_factor, 0.10)
            p_up -= penalty
            p_down += penalty
        elif not predicted_up and diff_pct > 0.5:  # Acima do strike mas prevê DOWN
            penalty = min(abs(diff_pct) / penalty_factor, 0.10)
            p_down -= penalty
            p_up += penalty
        
        # Garante limites
        p_up = np.clip(p_up, 0.01, 0.99)
        p_down = 1 - p_up
        
        return p_up, p_down
    
    def _combine_signals(self, signals: List[BayesianSignal]) -> Tuple[float, float]:
        """Combina sinais com Naive Bayes"""
        if not signals:
            return self.prior_up, self.prior_down
        
        log_odds_up = np.log(self.prior_up / self.prior_down)
        
        for signal in signals:
            if signal.p_down > 0:
                likelihood_ratio = signal.p_up / signal.p_down
                log_odds_up += signal.weight * np.log(likelihood_ratio)
        
        odds_up = np.exp(log_odds_up)
        p_up = odds_up / (1 + odds_up)
        p_down = 1 - p_up
        
        return p_up, p_down
    
    def _calculate_rsi(self, prices: np.ndarray, period: int) -> float:
        """Calcula RSI"""
        deltas = np.diff(prices)
        gains = np.where(deltas > 0, deltas, 0)
        losses = np.where(deltas < 0, -deltas, 0)
        
        avg_gain = np.mean(gains[-period:])
        avg_loss = np.mean(losses[-period:])
        
        if avg_loss == 0:
            return 100
        
        rs = avg_gain / avg_loss
        return 100 - (100 / (1 + rs))
    
    def _calculate_ema(self, prices: np.ndarray, period: int) -> float:
        """Calcula EMA"""
        multiplier = 2 / (period + 1)
        ema = prices[0]
        
        for price in prices[1:]:
            ema = (price * multiplier) + (ema * (1 - multiplier))
        
        return ema
    
    def _default_prediction(self, symbol: str, strike: Optional[float], 
                           current: Optional[float]) -> BayesianPrediction:
        """Predição padrão"""
        return BayesianPrediction(
            symbol=symbol,
            p_up=self.prior_up,
            p_down=self.prior_down,
            confidence=0.5,
            edge=0.0,
            signals=[],
            strike_price=strike,
            current_price=current,
            timestamp=datetime.utcnow()
        )
    
    def update_with_outcome(self, prediction: BayesianPrediction, actual_outcome: str):
        """
        Atualiza histórico e re-treina periodicamente com dados LIMPOS do trades_log.json
        
        MUDANÇA: Ao invés de aprender com predições em runtime (que estavam ruins),
        re-treina com o histórico completo de trades_log.json a cada 100 outcomes.
        """
        correct = prediction.get_direction() == actual_outcome
        
        self.prediction_history.append({
            'prediction': prediction,
            'actual': actual_outcome,
            'correct': correct,
            'confidence': prediction.confidence,
            'edge': prediction.edge
        })
        
        # Atualiza sequências
        if correct:
            self.consecutive_wins += 1
            self.consecutive_losses = 0
        else:
            self.consecutive_losses += 1
            self.consecutive_wins = 0
        
        # Log para análise
        logger.info(f"📊 Resultado: {'✅' if correct else '❌'} | "
                   f"Seq Wins: {self.consecutive_wins} | Seq Losses: {self.consecutive_losses}")
        
        if len(self.prediction_history) >= 1000:
            self._recalibrate_priors()
    
    def _recalibrate_priors(self):
        """Recalibra priors baseado em performance recente"""
        recent = self.prediction_history[-100:]
        
        up_pred = [p for p in recent if p['prediction'].get_direction() == 'UP']
        down_pred = [p for p in recent if p['prediction'].get_direction() == 'DOWN']
        
        up_acc = sum(1 for p in up_pred if p['correct']) / len(up_pred) if up_pred else 0.5
        down_acc = sum(1 for p in down_pred if p['correct']) / len(down_pred) if down_pred else 0.5
        
        logger.info(f"📈 Acurácia UP: {up_acc:.2%} | DOWN: {down_acc:.2%}")
        
        # Ajusta confiança mínima baseado em performance
        if up_acc < 0.5 and down_acc < 0.5:
            logger.warning("⚠️ Performance abaixo do esperado - aumentando seletividade")