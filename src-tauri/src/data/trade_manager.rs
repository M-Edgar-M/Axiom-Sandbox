//! Trade manager for orchestrating trade lifecycle and phase transitions.

use rust_decimal::Decimal;
use thiserror::Error;

use super::csv_logger::{CsvError, CsvLogger};
use super::trade_record::{DECIMAL_PLACES, TradeDirection, TradePhase, TradeRecord, TradeStatus};

/// Configuration constants for trade management.
pub mod config {
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    /// R-multiple at which to trigger Phase 2 (move SL to breakeven, close 33%).
    pub const PHASE2_R_MULTIPLE: Decimal = dec!(1.5);

    /// Percentage of position to close at Phase 2.
    pub const PHASE2_CLOSE_PERCENT: Decimal = dec!(0.33);

    /// R-multiple at which to trigger Phase 3 (trailing stop).
    pub const PHASE3_R_MULTIPLE: Decimal = dec!(2.5);

    /// ATR multiplier for trailing stop calculation.
    pub const TRAILING_STOP_ATR_MULT: Decimal = dec!(2.0);
}

/// Errors that can occur during trade management.
#[derive(Debug, Error)]
pub enum TradeManagerError {
    #[error("CSV error: {0}")]
    Csv(#[from] CsvError),

    #[error("Trade not found: {id}")]
    TradeNotFound { id: String },

    #[error("Trade already closed: {id}")]
    TradeAlreadyClosed { id: String },

    #[error("Invalid phase transition from {from} to {to}")]
    InvalidPhaseTransition { from: String, to: String },
}

/// Result of evaluating a trade's current state.
#[derive(Debug, Clone, PartialEq)]
pub enum TradeAction {
    /// No action required.
    None,
    /// Transition to Phase 2: move SL to breakeven, close 33%.
    TransitionPhase2 {
        close_quantity: Decimal,
        new_stop_loss: Decimal,
    },
    /// Transition to Phase 3: activate trailing stop.
    TransitionPhase3 { trailing_stop: Decimal },
    /// Close the trade (stop loss or take profit hit).
    CloseTrade { exit_price: Decimal, reason: String },
}

/// Trade manager for handling trade lifecycle.
pub struct TradeManager {
    /// CSV logger for persisting trade records.
    logger: CsvLogger,
    /// In-memory cache of active trades.
    active_trades: Vec<TradeRecord>,
}

impl TradeManager {
    /// Creates a new TradeManager with the given CSV path.
    pub fn new(csv_path: impl AsRef<std::path::Path>) -> Result<Self, TradeManagerError> {
        let logger = CsvLogger::new(csv_path)?;
        let active_trades = logger.find_open_trades()?;

        Ok(Self {
            logger,
            active_trades,
        })
    }

    /// Creates a TradeManager with default CSV path (trades_log.csv).
    pub fn default_path() -> Result<Self, TradeManagerError> {
        Self::new("trades_log.csv")
    }

    /// Opens a new trade and logs it to CSV.
    pub fn open_trade(
        &mut self,
        id: impl Into<String>,
        symbol: impl Into<String>,
        direction: TradeDirection,
        entry_price: Decimal,
        stop_loss: Decimal,
        take_profit: Decimal,
        position_size: Decimal,
        atr_value: Decimal,
        entry_rsi: Decimal,
        entry_adx: Decimal,
        trend_condition: String,
    ) -> Result<TradeRecord, TradeManagerError> {
        let record = TradeRecord::new(
            id,
            symbol,
            direction,
            entry_price,
            stop_loss,
            take_profit,
            position_size,
            atr_value,
            entry_rsi,
            entry_adx,
            trend_condition,
        );

        self.logger.append_record(&record)?;
        self.active_trades.push(record.clone());

        Ok(record)
    }

    /// Evaluates what action should be taken for a trade at the current price.
    pub fn evaluate_trade(&self, trade_id: &str, current_price: Decimal) -> TradeAction {
        let trade = match self.find_trade(trade_id) {
            Some(t) => t,
            None => return TradeAction::None,
        };

        if trade.status != TradeStatus::Open {
            return TradeAction::None;
        }

        let r_multiple = trade.current_r_multiple(current_price);

        // Check for stop loss hit
        let stop_hit = match trade.direction {
            TradeDirection::Long => current_price <= trade.stop_loss,
            TradeDirection::Short => current_price >= trade.stop_loss,
        };

        if stop_hit {
            return TradeAction::CloseTrade {
                exit_price: trade.stop_loss,
                reason: "Stop Loss".to_string(),
            };
        }

        // Check for take profit hit
        let tp_hit = match trade.direction {
            TradeDirection::Long => current_price >= trade.take_profit,
            TradeDirection::Short => current_price <= trade.take_profit,
        };

        if tp_hit {
            return TradeAction::CloseTrade {
                exit_price: trade.take_profit,
                reason: "Take Profit".to_string(),
            };
        }

        // Check phase transitions (only upward)
        match trade.phase {
            TradePhase::Phase1 => {
                // Check for Phase 3 first (higher priority if both thresholds crossed)
                if r_multiple >= config::PHASE3_R_MULTIPLE {
                    return TradeAction::TransitionPhase3 {
                        trailing_stop: trade
                            .trailing_stop_price(current_price, config::TRAILING_STOP_ATR_MULT),
                    };
                }
                // Check for Phase 2
                if r_multiple >= config::PHASE2_R_MULTIPLE {
                    let close_quantity = (trade.position_size * config::PHASE2_CLOSE_PERCENT)
                        .round_dp(DECIMAL_PLACES);
                    return TradeAction::TransitionPhase2 {
                        close_quantity,
                        new_stop_loss: trade.entry_price, // Breakeven
                    };
                }
            }
            TradePhase::Phase2 => {
                // Check for Phase 3
                if r_multiple >= config::PHASE3_R_MULTIPLE {
                    return TradeAction::TransitionPhase3 {
                        trailing_stop: trade
                            .trailing_stop_price(current_price, config::TRAILING_STOP_ATR_MULT),
                    };
                }
            }
            TradePhase::Phase3 => {
                // Update trailing stop if price moved favorably
                let new_trailing =
                    trade.trailing_stop_price(current_price, config::TRAILING_STOP_ATR_MULT);
                let should_update = match trade.direction {
                    TradeDirection::Long => new_trailing > trade.stop_loss,
                    TradeDirection::Short => new_trailing < trade.stop_loss,
                };

                if should_update {
                    // Return trailing stop update as Phase3 transition
                    return TradeAction::TransitionPhase3 {
                        trailing_stop: new_trailing,
                    };
                }
            }
        }

        TradeAction::None
    }

    /// Executes a trade action and updates records.
    pub fn execute_action(
        &mut self,
        trade_id: &str,
        action: TradeAction,
        current_price: Decimal,
    ) -> Result<(), TradeManagerError> {
        match action {
            TradeAction::None => Ok(()),
            TradeAction::TransitionPhase2 {
                close_quantity,
                new_stop_loss,
            } => self.execute_phase2(trade_id, close_quantity, new_stop_loss, current_price),
            TradeAction::TransitionPhase3 { trailing_stop } => {
                self.execute_phase3(trade_id, trailing_stop)
            }
            TradeAction::CloseTrade { exit_price, reason } => {
                self.close_trade(trade_id, exit_price, &reason)
            }
        }
    }

    /// Executes Phase 2 transition: move SL to breakeven, close 33%.
    fn execute_phase2(
        &mut self,
        trade_id: &str,
        close_quantity: Decimal,
        new_stop_loss: Decimal,
        current_price: Decimal,
    ) -> Result<(), TradeManagerError> {
        let trade = self
            .find_trade_mut(trade_id)
            .ok_or(TradeManagerError::TradeNotFound {
                id: trade_id.to_string(),
            })?;

        if trade.phase != TradePhase::Phase1 {
            return Err(TradeManagerError::InvalidPhaseTransition {
                from: trade.phase.to_string(),
                to: "Phase2".to_string(),
            });
        }

        // Calculate realized PnL for closed portion
        let pnl_per_unit = match trade.direction {
            TradeDirection::Long => current_price - trade.entry_price,
            TradeDirection::Short => trade.entry_price - current_price,
        };
        let realized_pnl = (pnl_per_unit * close_quantity).round_dp(DECIMAL_PLACES);

        // Update trade
        trade.transition_to_phase2(new_stop_loss);
        trade.reduce_position(close_quantity, realized_pnl);

        // Update CSV
        let remaining_size = trade.position_size;
        let total_pnl = trade.realized_pnl;

        self.logger.update_record(
            trade_id,
            None,
            Some(total_pnl),
            None,
            Some(TradePhase::Phase2),
            Some(new_stop_loss),
            Some(remaining_size),
        )?;

        Ok(())
    }

    /// Executes Phase 3 transition: activate/update trailing stop.
    fn execute_phase3(
        &mut self,
        trade_id: &str,
        trailing_stop: Decimal,
    ) -> Result<(), TradeManagerError> {
        let trade = self
            .find_trade_mut(trade_id)
            .ok_or(TradeManagerError::TradeNotFound {
                id: trade_id.to_string(),
            })?;

        // Transition to Phase 3 if not already
        if trade.phase != TradePhase::Phase3 {
            trade.transition_to_phase3();
        }

        // Update stop loss to trailing stop
        trade.stop_loss = trailing_stop.round_dp(DECIMAL_PLACES);

        // Update CSV
        self.logger.update_record(
            trade_id,
            None,
            None,
            None,
            Some(TradePhase::Phase3),
            Some(trailing_stop),
            None,
        )?;

        Ok(())
    }

    /// Closes a trade and calculates final PnL.
    pub fn close_trade(
        &mut self,
        trade_id: &str,
        exit_price: Decimal,
        _reason: &str,
    ) -> Result<(), TradeManagerError> {
        let trade = self
            .find_trade_mut(trade_id)
            .ok_or(TradeManagerError::TradeNotFound {
                id: trade_id.to_string(),
            })?;

        if trade.status == TradeStatus::Closed {
            return Err(TradeManagerError::TradeAlreadyClosed {
                id: trade_id.to_string(),
            });
        }

        // Calculate final PnL (including any previously realized)
        let remaining_pnl = trade.unrealized_pnl(exit_price);
        let total_pnl = trade.realized_pnl + remaining_pnl;

        trade.close(exit_price);
        trade.realized_pnl = total_pnl.round_dp(DECIMAL_PLACES);

        // Update CSV
        self.logger.update_record(
            trade_id,
            Some(exit_price),
            Some(total_pnl),
            Some(TradeStatus::Closed),
            None,
            None,
            Some(Decimal::ZERO), // Position closed
        )?;

        // Remove from active trades
        self.active_trades.retain(|t| t.id != trade_id);

        Ok(())
    }

    /// Finds a trade by ID.
    pub fn find_trade(&self, trade_id: &str) -> Option<&TradeRecord> {
        self.active_trades.iter().find(|t| t.id == trade_id)
    }

    /// Finds a trade by ID (mutable).
    fn find_trade_mut(&mut self, trade_id: &str) -> Option<&mut TradeRecord> {
        self.active_trades.iter_mut().find(|t| t.id == trade_id)
    }

    /// Returns all active trades.
    pub fn active_trades(&self) -> &[TradeRecord] {
        &self.active_trades
    }

    /// Returns the CSV logger.
    pub fn logger(&self) -> &CsvLogger {
        &self.logger
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use tempfile::tempdir;

    fn create_test_manager() -> TradeManager {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_trades.csv");
        // Keep dir alive by leaking it (for tests only)
        std::mem::forget(dir);
        TradeManager::new(path).unwrap()
    }

    #[test]
    fn test_open_trade() {
        let mut manager = create_test_manager();

        let trade = manager
            .open_trade(
                "trade-001",
                "BTCUSDT",
                TradeDirection::Long,
                dec!(50000),
                dec!(49000),
                dec!(53000),
                dec!(1),
                dec!(500),
                dec!(55), // RSI
                dec!(25), // ADX
                "StrongUptrend".to_string(),
            )
            .unwrap();

        assert_eq!(trade.id, "trade-001");
        assert_eq!(trade.phase, TradePhase::Phase1);
        assert_eq!(manager.active_trades().len(), 1);
        assert_eq!(trade.entry_rsi, dec!(55));
    }

    #[test]
    fn test_evaluate_phase2_trigger() {
        let mut manager = create_test_manager();

        manager
            .open_trade(
                "trade-001",
                "BTCUSDT",
                TradeDirection::Long,
                dec!(50000),
                dec!(49000),
                dec!(53000),
                dec!(1),
                dec!(500),
                dec!(0),
                dec!(0),
                "Neutral".to_string(),
            )
            .unwrap();

        // At 1.5R (51500)
        let action = manager.evaluate_trade("trade-001", dec!(51500));

        match action {
            TradeAction::TransitionPhase2 {
                close_quantity,
                new_stop_loss,
            } => {
                assert_eq!(close_quantity, dec!(0.33));
                assert_eq!(new_stop_loss, dec!(50000)); // Breakeven
            }
            _ => panic!("Expected Phase2 transition"),
        }
    }

    #[test]
    fn test_execute_phase2() {
        let mut manager = create_test_manager();

        manager
            .open_trade(
                "trade-001",
                "BTCUSDT",
                TradeDirection::Long,
                dec!(50000),
                dec!(49000),
                dec!(53000),
                dec!(1),
                dec!(500),
                dec!(0),
                dec!(0),
                "Neutral".to_string(),
            )
            .unwrap();

        let action = manager.evaluate_trade("trade-001", dec!(51500));
        manager
            .execute_action("trade-001", action, dec!(51500))
            .unwrap();

        let trade = manager.find_trade("trade-001").unwrap();
        assert_eq!(trade.phase, TradePhase::Phase2);
        assert_eq!(trade.stop_loss, dec!(50000));
        assert_eq!(trade.position_size, dec!(0.67));
    }

    #[test]
    fn test_evaluate_phase3_trigger() {
        let mut manager = create_test_manager();

        manager
            .open_trade(
                "trade-001",
                "BTCUSDT",
                TradeDirection::Long,
                dec!(50000),
                dec!(49000),
                dec!(53000),
                dec!(1),
                dec!(500),
                dec!(0),
                dec!(0),
                "Neutral".to_string(),
            )
            .unwrap();

        // At 2.5R (52500)
        let action = manager.evaluate_trade("trade-001", dec!(52500));

        match action {
            TradeAction::TransitionPhase3 { trailing_stop } => {
                // Trailing stop = 52500 - (500 * 2.0) = 51500
                assert_eq!(trailing_stop, dec!(51500));
            }
            _ => panic!("Expected Phase3 transition"),
        }
    }

    #[test]
    fn test_stop_loss_exit() {
        let mut manager = create_test_manager();

        manager
            .open_trade(
                "trade-001",
                "BTCUSDT",
                TradeDirection::Long,
                dec!(50000),
                dec!(49000),
                dec!(53000),
                dec!(1),
                dec!(500),
                dec!(0),
                dec!(0),
                "Neutral".to_string(),
            )
            .unwrap();

        // Price hits stop loss
        let action = manager.evaluate_trade("trade-001", dec!(48500));

        match action {
            TradeAction::CloseTrade { exit_price, reason } => {
                assert_eq!(exit_price, dec!(49000));
                assert_eq!(reason, "Stop Loss");
            }
            _ => panic!("Expected CloseTrade action"),
        }
    }

    #[test]
    fn test_take_profit_exit() {
        let mut manager = create_test_manager();

        manager
            .open_trade(
                "trade-001",
                "BTCUSDT",
                TradeDirection::Long,
                dec!(50000),
                dec!(49000),
                dec!(53000),
                dec!(1),
                dec!(500),
                dec!(0),
                dec!(0),
                "Neutral".to_string(),
            )
            .unwrap();

        // Price hits take profit
        let action = manager.evaluate_trade("trade-001", dec!(53500));

        match action {
            TradeAction::CloseTrade { exit_price, reason } => {
                assert_eq!(exit_price, dec!(53000));
                assert_eq!(reason, "Take Profit");
            }
            _ => panic!("Expected CloseTrade action"),
        }
    }

    #[test]
    fn test_close_trade_pnl() {
        let mut manager = create_test_manager();

        manager
            .open_trade(
                "trade-001",
                "BTCUSDT",
                TradeDirection::Long,
                dec!(50000),
                dec!(49000),
                dec!(53000),
                dec!(1),
                dec!(500),
                dec!(0),
                dec!(0),
                "Neutral".to_string(),
            )
            .unwrap();

        manager
            .close_trade("trade-001", dec!(51500), "Manual")
            .unwrap();

        // Trade should be removed from active
        assert!(manager.find_trade("trade-001").is_none());
        assert_eq!(manager.active_trades().len(), 0);
    }

    // ── Short position tests ──

    fn open_short_trade(manager: &mut TradeManager, id: &str) {
        // Short: entry=50000, SL=51000 (above), TP=47000 (below)
        manager
            .open_trade(
                id,
                "BTCUSDT",
                TradeDirection::Short,
                dec!(50000),
                dec!(51000), // SL above entry
                dec!(47000), // TP below entry
                dec!(1),
                dec!(500),
                dec!(0),
                dec!(0),
                "Downtrend".to_string(),
            )
            .unwrap();
    }

    #[test]
    fn test_short_evaluate_phase2_trigger() {
        let mut manager = create_test_manager();
        open_short_trade(&mut manager, "short-001");

        // Price drops to 1.5R: entry(50000) - 1.5 * risk(1000) = 48500
        let action = manager.evaluate_trade("short-001", dec!(48500));

        match action {
            TradeAction::TransitionPhase2 {
                close_quantity,
                new_stop_loss,
            } => {
                assert_eq!(close_quantity, dec!(0.33));
                assert_eq!(new_stop_loss, dec!(50000)); // Breakeven
            }
            _ => panic!("Expected Phase2 transition, got {:?}", action),
        }
    }

    #[test]
    fn test_short_stop_loss_exit() {
        let mut manager = create_test_manager();
        open_short_trade(&mut manager, "short-001");

        // Price rises above stop loss (51000)
        let action = manager.evaluate_trade("short-001", dec!(51500));

        match action {
            TradeAction::CloseTrade { exit_price, reason } => {
                assert_eq!(exit_price, dec!(51000));
                assert_eq!(reason, "Stop Loss");
            }
            _ => panic!("Expected CloseTrade action, got {:?}", action),
        }
    }

    #[test]
    fn test_short_take_profit_exit() {
        let mut manager = create_test_manager();
        open_short_trade(&mut manager, "short-001");

        // Price drops below take profit (47000)
        let action = manager.evaluate_trade("short-001", dec!(46500));

        match action {
            TradeAction::CloseTrade { exit_price, reason } => {
                assert_eq!(exit_price, dec!(47000));
                assert_eq!(reason, "Take Profit");
            }
            _ => panic!("Expected CloseTrade action, got {:?}", action),
        }
    }

    #[test]
    fn test_short_execute_phase2() {
        let mut manager = create_test_manager();
        open_short_trade(&mut manager, "short-001");

        let action = manager.evaluate_trade("short-001", dec!(48500));
        manager
            .execute_action("short-001", action, dec!(48500))
            .unwrap();

        let trade = manager.find_trade("short-001").unwrap();
        assert_eq!(trade.phase, TradePhase::Phase2);
        assert_eq!(trade.stop_loss, dec!(50000)); // Breakeven
        assert_eq!(trade.position_size, dec!(0.67));
    }

    #[test]
    fn test_short_evaluate_phase3_trigger() {
        let mut manager = create_test_manager();
        open_short_trade(&mut manager, "short-001");

        // Price drops to 2.5R: entry(50000) - 2.5 * risk(1000) = 47500
        let action = manager.evaluate_trade("short-001", dec!(47500));

        match action {
            TradeAction::TransitionPhase3 { trailing_stop } => {
                // Short trailing stop = 47500 + (500 * 2.0) = 48500
                assert_eq!(trailing_stop, dec!(48500));
            }
            _ => panic!("Expected Phase3 transition, got {:?}", action),
        }
    }
}
