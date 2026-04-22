//! Risk manager orchestration with circuit breaker.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::types::{Balance, Price};

use super::config;
use super::sizing::{RiskError, SizingInput, SizingResult, calculate_position_size};

/// Trade result for recording P&L.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeResult {
    /// Symbol traded.
    pub symbol: String,
    /// Realized profit/loss.
    pub pnl: Balance,
    /// Trade close time.
    pub closed_at: DateTime<Utc>,
}

/// Daily trading statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DailyStats {
    /// Total realized P&L for the day.
    pub realized_pnl: Balance,
    /// Number of trades taken.
    pub trade_count: u32,
    /// Number of winning trades.
    pub wins: u32,
    /// Number of losing trades.
    pub losses: u32,
    /// Date these stats are for.
    pub date: Option<DateTime<Utc>>,
}

impl DailyStats {
    /// Records a trade result and updates statistics.
    pub fn record_trade(&mut self, result: &TradeResult) {
        self.realized_pnl += result.pnl;
        self.trade_count += 1;

        if result.pnl > Decimal::ZERO {
            self.wins += 1;
        } else if result.pnl < Decimal::ZERO {
            self.losses += 1;
        }
    }

    /// Returns win rate as a percentage (0.0 to 1.0).
    pub fn win_rate(&self) -> Decimal {
        if self.trade_count == 0 {
            return Decimal::ZERO;
        }
        Decimal::from(self.wins) / Decimal::from(self.trade_count)
    }

    /// Resets statistics for a new day.
    pub fn reset(&mut self, date: DateTime<Utc>) {
        self.realized_pnl = Decimal::ZERO;
        self.trade_count = 0;
        self.wins = 0;
        self.losses = 0;
        self.date = Some(date);
    }
}

/// Risk manager for enforcing position sizing and circuit breaker rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskManager {
    /// Starting equity at the beginning of the day.
    starting_equity: Balance,
    /// Current account equity.
    current_equity: Balance,
    /// Daily statistics.
    daily_stats: DailyStats,
    /// Whether trading is halted due to circuit breaker.
    circuit_breaker_active: bool,
    /// Time when circuit breaker was triggered.
    circuit_breaker_triggered_at: Option<DateTime<Utc>>,
}

impl RiskManager {
    /// Creates a new risk manager with the given starting equity.
    pub fn new(starting_equity: Balance) -> Self {
        Self {
            starting_equity,
            current_equity: starting_equity,
            daily_stats: DailyStats::default(),
            circuit_breaker_active: false,
            circuit_breaker_triggered_at: None,
        }
    }

    /// Returns current account equity.
    pub fn current_equity(&self) -> Balance {
        self.current_equity
    }

    /// Returns starting equity for the day.
    pub fn starting_equity(&self) -> Balance {
        self.starting_equity
    }

    /// Returns daily statistics.
    pub fn daily_stats(&self) -> &DailyStats {
        &self.daily_stats
    }

    /// Returns true if circuit breaker is active.
    pub fn is_trading_halted(&self) -> bool {
        self.circuit_breaker_active
    }

    /// Returns the daily loss percentage.
    pub fn daily_loss_percentage(&self) -> Decimal {
        if self.starting_equity == Decimal::ZERO {
            return Decimal::ZERO;
        }
        let loss = self.starting_equity - self.current_equity;
        if loss <= Decimal::ZERO {
            Decimal::ZERO
        } else {
            loss / self.starting_equity
        }
    }

    /// Calculates position size for a trade, enforcing all risk constraints.
    ///
    /// # Errors
    /// - `CircuitBreakerTriggered` if daily loss limit exceeded
    /// - `VolatilityTooHigh` if ATR ratio > 3.0
    /// - `InvalidStopLoss` or `InvalidEquity` for input validation
    pub fn calculate_position_size(
        &self,
        entry_price: Price,
        stop_loss: Price,
        atr_15m: Decimal,
        atr_4h: Decimal,
        market_cap: Option<Decimal>,
    ) -> Result<SizingResult, RiskError> {
        // Check circuit breaker first
        if self.circuit_breaker_active {
            let daily_loss_pct = self.daily_loss_percentage() * Decimal::from(100);
            return Err(RiskError::CircuitBreakerTriggered {
                daily_loss_pct,
                limit: config::DAILY_LOSS_LIMIT * Decimal::from(100),
            });
        }

        let input = SizingInput {
            equity: self.current_equity,
            entry_price,
            stop_loss,
            atr_15m,
            atr_4h,
            market_cap,
        };

        calculate_position_size(&input)
    }

    /// Records a trade result and updates equity and statistics.
    ///
    /// Triggers circuit breaker if daily loss limit is exceeded.
    pub fn record_trade_result(&mut self, result: TradeResult) {
        // Update equity
        self.current_equity += result.pnl;

        // Record in daily stats
        self.daily_stats.record_trade(&result);

        // Check circuit breaker
        self.check_circuit_breaker();
    }

    /// Updates current equity (e.g., from external source).
    pub fn update_equity(&mut self, equity: Balance) {
        self.current_equity = equity;
        self.check_circuit_breaker();
    }

    /// Checks if circuit breaker should be triggered.
    fn check_circuit_breaker(&mut self) {
        if self.circuit_breaker_active {
            return;
        }

        let loss_pct = self.daily_loss_percentage();
        if loss_pct >= config::DAILY_LOSS_LIMIT {
            self.circuit_breaker_active = true;
            self.circuit_breaker_triggered_at = Some(Utc::now());
        }
    }

    /// Resets for a new trading day.
    ///
    /// Call this at the start of each trading day to:
    /// - Reset circuit breaker
    /// - Update starting equity
    /// - Clear daily statistics
    pub fn reset_daily(&mut self) {
        self.starting_equity = self.current_equity;
        self.circuit_breaker_active = false;
        self.circuit_breaker_triggered_at = None;
        self.daily_stats.reset(Utc::now());
    }

    /// Manually triggers the circuit breaker.
    pub fn trigger_circuit_breaker(&mut self) {
        self.circuit_breaker_active = true;
        self.circuit_breaker_triggered_at = Some(Utc::now());
    }

    /// Manually resets the circuit breaker (use with caution).
    pub fn reset_circuit_breaker(&mut self) {
        self.circuit_breaker_active = false;
        self.circuit_breaker_triggered_at = None;
    }

    /// Returns the maximum position size allowed based on current equity.
    pub fn max_position_value(&self) -> Balance {
        self.current_equity * config::MAX_POSITION_EQUITY_PCT
    }

    /// Returns the risk amount per trade based on current equity.
    pub fn risk_per_trade(&self) -> Balance {
        self.current_equity * config::RISK_PER_TRADE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_risk_manager_creation() {
        let manager = RiskManager::new(dec!(100_000));

        assert_eq!(manager.current_equity(), dec!(100_000));
        assert_eq!(manager.starting_equity(), dec!(100_000));
        assert!(!manager.is_trading_halted());
    }

    #[test]
    fn test_position_sizing() {
        let manager = RiskManager::new(dec!(100_000));

        let result = manager
            .calculate_position_size(
                dec!(100),                  // entry
                dec!(0),                    // stop loss (100% risk)
                dec!(1),                    // 15m ATR
                dec!(1),                    // 4h ATR
                Some(dec!(10_000_000_000)), // $10B market cap
            )
            .unwrap();

        // (100,000 * 0.05) / 100 = 50
        assert_eq!(result.size, dec!(50));
    }

    #[test]
    fn test_trade_recording() {
        let mut manager = RiskManager::new(dec!(100_000));

        let result = TradeResult {
            symbol: "BTCUSDT".to_string(),
            pnl: dec!(500),
            closed_at: Utc::now(),
        };

        manager.record_trade_result(result);

        assert_eq!(manager.current_equity(), dec!(100_500));
        assert_eq!(manager.daily_stats().trade_count, 1);
        assert_eq!(manager.daily_stats().wins, 1);
    }

    #[test]
    fn test_circuit_breaker_trigger() {
        let mut manager = RiskManager::new(dec!(100_000));

        // Lose 3% (greater than 2.5% limit)
        let result = TradeResult {
            symbol: "BTCUSDT".to_string(),
            pnl: dec!(-3000),
            closed_at: Utc::now(),
        };

        manager.record_trade_result(result);

        assert!(manager.is_trading_halted());

        // Should not be able to calculate new position
        let sizing =
            manager.calculate_position_size(dec!(50_000), dec!(49_000), dec!(100), dec!(100), None);

        assert!(matches!(
            sizing,
            Err(RiskError::CircuitBreakerTriggered { .. })
        ));
    }

    #[test]
    fn test_daily_reset() {
        let mut manager = RiskManager::new(dec!(100_000));

        // Record a loss
        manager.record_trade_result(TradeResult {
            symbol: "BTCUSDT".to_string(),
            pnl: dec!(-3000),
            closed_at: Utc::now(),
        });

        assert!(manager.is_trading_halted());
        assert_eq!(manager.current_equity(), dec!(97_000));

        // Reset for new day
        manager.reset_daily();

        assert!(!manager.is_trading_halted());
        assert_eq!(manager.starting_equity(), dec!(97_000));
        assert_eq!(manager.daily_stats().trade_count, 0);
    }

    #[test]
    fn test_daily_loss_percentage() {
        let mut manager = RiskManager::new(dec!(100_000));

        // Lose 2000 = 2%
        manager.update_equity(dec!(98_000));

        let loss_pct = manager.daily_loss_percentage();
        assert_eq!(loss_pct, dec!(0.02));
    }

    #[test]
    fn test_max_position_value() {
        let manager = RiskManager::new(dec!(100_000));

        // 500% of 100,000 = 500,000
        assert_eq!(manager.max_position_value(), dec!(500000));
    }

    #[test]
    fn test_risk_per_trade() {
        let manager = RiskManager::new(dec!(100_000));

        // 5% of 100,000 = 5,000
        assert_eq!(manager.risk_per_trade(), dec!(5000));
    }
}
