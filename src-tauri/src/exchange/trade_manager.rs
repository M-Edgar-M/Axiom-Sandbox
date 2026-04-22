//! In-trade position management with state machine.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::types::{Position, Price, Volume};

use super::config;
use super::orders::{OcoOrderRequest, OrderSide};

/// State of an active trade position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeState {
    /// Position just opened, initial SL/TP placed.
    Open,
    /// Price reached 1.5R, SL moved to breakeven, 33% closed.
    FirstTpHit,
    /// Price reached 2.5R, trailing stop activated.
    TrailingActive,
    /// Position is closed.
    Closed,
}

impl Default for TradeState {
    fn default() -> Self {
        TradeState::Open
    }
}

/// Managed position with in-trade management state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedPosition {
    /// Trading symbol.
    pub symbol: String,
    /// Position direction.
    pub direction: Position,
    /// Entry price.
    pub entry_price: Price,
    /// Original stop loss price.
    pub original_stop_loss: Price,
    /// Current stop loss price (may be moved to breakeven).
    pub current_stop_loss: Price,
    /// Take profit price.
    pub take_profit: Price,
    /// Original position size.
    pub original_quantity: Volume,
    /// Current remaining quantity.
    pub current_quantity: Volume,
    /// Risk amount (entry - stop loss in price).
    pub risk_per_unit: Price,
    /// Current ATR value for trailing stop.
    pub atr_value: Decimal,
    /// Current trade state.
    pub state: TradeState,
    /// OCO order list ID (if active).
    pub oco_order_list_id: Option<u64>,
    /// Stop loss order ID.
    pub stop_loss_order_id: Option<u64>,
    /// Take profit order ID.
    pub take_profit_order_id: Option<u64>,
    /// Trailing stop order ID (if activated).
    pub trailing_stop_order_id: Option<u64>,
    /// Position opened at.
    pub opened_at: DateTime<Utc>,
    /// Position closed at.
    pub closed_at: Option<DateTime<Utc>>,
    /// Total realized P&L.
    pub realized_pnl: Decimal,
}

impl ManagedPosition {
    /// Creates a new managed position.
    pub fn new(
        symbol: impl Into<String>,
        direction: Position,
        entry_price: Price,
        stop_loss: Price,
        take_profit: Price,
        quantity: Volume,
        atr_value: Decimal,
    ) -> Self {
        let risk_per_unit = (entry_price - stop_loss).abs();

        Self {
            symbol: symbol.into(),
            direction,
            entry_price,
            original_stop_loss: stop_loss,
            current_stop_loss: stop_loss,
            take_profit,
            original_quantity: quantity,
            current_quantity: quantity,
            risk_per_unit,
            atr_value,
            state: TradeState::Open,
            oco_order_list_id: None,
            stop_loss_order_id: None,
            take_profit_order_id: None,
            trailing_stop_order_id: None,
            opened_at: Utc::now(),
            closed_at: None,
            realized_pnl: Decimal::ZERO,
        }
    }

    /// Calculates current profit in R-multiples.
    pub fn current_r_multiple(&self, current_price: Price) -> Decimal {
        if self.risk_per_unit == Decimal::ZERO {
            return Decimal::ZERO;
        }

        let pnl = match self.direction {
            Position::Long => current_price - self.entry_price,
            Position::Short => self.entry_price - current_price,
            Position::None => Decimal::ZERO,
        };

        pnl / self.risk_per_unit
    }

    /// Calculates unrealized P&L at current price.
    pub fn unrealized_pnl(&self, current_price: Price) -> Decimal {
        let pnl_per_unit = match self.direction {
            Position::Long => current_price - self.entry_price,
            Position::Short => self.entry_price - current_price,
            Position::None => Decimal::ZERO,
        };

        pnl_per_unit * self.current_quantity
    }

    /// Checks if position should transition to FirstTpHit state.
    pub fn should_hit_first_tp(&self, current_price: Price) -> bool {
        self.state == TradeState::Open
            && self.current_r_multiple(current_price) >= config::FIRST_TP_R_MULTIPLE
    }

    /// Checks if position should transition to TrailingActive state.
    pub fn should_activate_trailing(&self, current_price: Price) -> bool {
        (self.state == TradeState::Open || self.state == TradeState::FirstTpHit)
            && self.current_r_multiple(current_price) >= config::TRAILING_ACTIVATION_R
    }

    /// Calculates quantity to close at first TP (33%).
    pub fn first_tp_close_quantity(&self) -> Volume {
        self.original_quantity * config::FIRST_TP_CLOSE_PCT
    }

    /// Calculates trailing stop offset based on ATR.
    pub fn trailing_stop_offset(&self) -> Decimal {
        self.atr_value * config::TRAILING_ATR_MULTIPLIER
    }

    /// Returns the order side needed to close this position.
    pub fn close_side(&self) -> OrderSide {
        match self.direction {
            Position::Long => OrderSide::Sell,
            Position::Short => OrderSide::Buy,
            Position::None => OrderSide::Sell, // Shouldn't happen
        }
    }

    /// Transitions to FirstTpHit state after partial close.
    pub fn transition_to_first_tp(&mut self, closed_quantity: Volume, realized_pnl: Decimal) {
        self.state = TradeState::FirstTpHit;
        self.current_quantity -= closed_quantity;
        self.current_stop_loss = self.entry_price; // Move SL to breakeven
        self.realized_pnl += realized_pnl;
    }

    /// Transitions to TrailingActive state.
    pub fn transition_to_trailing(&mut self, trailing_order_id: u64) {
        self.state = TradeState::TrailingActive;
        self.trailing_stop_order_id = Some(trailing_order_id);
        // Cancel fixed SL/TP orders
        self.stop_loss_order_id = None;
        self.take_profit_order_id = None;
    }

    /// Closes the position completely.
    pub fn close(&mut self, final_pnl: Decimal) {
        self.state = TradeState::Closed;
        self.current_quantity = Decimal::ZERO;
        self.realized_pnl += final_pnl;
        self.closed_at = Some(Utc::now());
    }

    /// Returns true if position is still active.
    pub fn is_active(&self) -> bool {
        self.state != TradeState::Closed && self.current_quantity > Decimal::ZERO
    }

    /// Creates OCO order request for this position.
    pub fn create_oco_request(&self, slippage: Decimal) -> OcoOrderRequest {
        match self.direction {
            Position::Long => OcoOrderRequest::close_long(
                &self.symbol,
                self.current_quantity,
                self.take_profit,
                self.current_stop_loss,
                slippage,
            ),
            Position::Short => OcoOrderRequest::close_short(
                &self.symbol,
                self.current_quantity,
                self.take_profit,
                self.current_stop_loss,
                slippage,
            ),
            Position::None => panic!("Cannot create OCO for None position"),
        }
    }
}

/// Action to take based on position state and current price.
#[derive(Debug, Clone, PartialEq)]
pub enum ManagementAction {
    /// No action needed.
    None,
    /// Execute first TP: move SL to breakeven, close 33%.
    ExecuteFirstTp {
        quantity_to_close: Volume,
        new_stop_loss: Price,
    },
    /// Activate trailing stop.
    ActivateTrailingStop { offset: Decimal },
    /// Close position completely.
    ClosePosition { reason: String },
}

/// Evaluates what action to take for a managed position.
pub fn evaluate_position(position: &ManagedPosition, current_price: Price) -> ManagementAction {
    if !position.is_active() {
        return ManagementAction::None;
    }

    // Check for trailing stop activation first (higher priority)
    if position.should_activate_trailing(current_price) {
        return ManagementAction::ActivateTrailingStop {
            offset: position.trailing_stop_offset(),
        };
    }

    // Check for first TP
    if position.should_hit_first_tp(current_price) {
        return ManagementAction::ExecuteFirstTp {
            quantity_to_close: position.first_tp_close_quantity(),
            new_stop_loss: position.entry_price,
        };
    }

    ManagementAction::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn create_long_position() -> ManagedPosition {
        ManagedPosition::new(
            "BTCUSDT",
            Position::Long,
            dec!(50000), // entry
            dec!(49000), // stop loss (1R = 1000)
            dec!(53000), // take profit
            dec!(1),     // quantity
            dec!(500),   // ATR
        )
    }

    #[test]
    fn test_r_multiple_calculation() {
        let position = create_long_position();

        // At entry
        assert_eq!(position.current_r_multiple(dec!(50000)), dec!(0));

        // At 1R profit
        assert_eq!(position.current_r_multiple(dec!(51000)), dec!(1));

        // At 1.5R profit
        assert_eq!(position.current_r_multiple(dec!(51500)), dec!(1.5));

        // At 1R loss
        assert_eq!(position.current_r_multiple(dec!(49000)), dec!(-1));
    }

    #[test]
    fn test_first_tp_trigger() {
        let position = create_long_position();

        // Below 1.5R - should not trigger
        assert!(!position.should_hit_first_tp(dec!(51400)));

        // At 1.5R - should trigger
        assert!(position.should_hit_first_tp(dec!(51500)));

        // Above 1.5R - should trigger
        assert!(position.should_hit_first_tp(dec!(52000)));
    }

    #[test]
    fn test_trailing_stop_trigger() {
        let position = create_long_position();

        // Below 2.5R - should not trigger
        assert!(!position.should_activate_trailing(dec!(52400)));

        // At 2.5R - should trigger
        assert!(position.should_activate_trailing(dec!(52500)));
    }

    #[test]
    fn test_first_tp_close_quantity() {
        let position = create_long_position();

        // 33% of 1 = 0.33
        assert_eq!(position.first_tp_close_quantity(), dec!(0.33));
    }

    #[test]
    fn test_trailing_stop_offset() {
        let position = create_long_position();

        // ATR 500 * 2.0 = 1000
        assert_eq!(position.trailing_stop_offset(), dec!(1000));
    }

    #[test]
    fn test_state_transition_first_tp() {
        let mut position = create_long_position();

        position.transition_to_first_tp(dec!(0.33), dec!(495));

        assert_eq!(position.state, TradeState::FirstTpHit);
        assert_eq!(position.current_quantity, dec!(0.67));
        assert_eq!(position.current_stop_loss, dec!(50000)); // Breakeven
        assert_eq!(position.realized_pnl, dec!(495));
    }

    #[test]
    fn test_evaluate_position_first_tp() {
        let position = create_long_position();

        let action = evaluate_position(&position, dec!(51500));

        match action {
            ManagementAction::ExecuteFirstTp {
                quantity_to_close,
                new_stop_loss,
            } => {
                assert_eq!(quantity_to_close, dec!(0.33));
                assert_eq!(new_stop_loss, dec!(50000));
            }
            _ => panic!("Expected ExecuteFirstTp action"),
        }
    }

    #[test]
    fn test_evaluate_position_trailing() {
        let position = create_long_position();

        let action = evaluate_position(&position, dec!(52500));

        match action {
            ManagementAction::ActivateTrailingStop { offset } => {
                assert_eq!(offset, dec!(1000));
            }
            _ => panic!("Expected ActivateTrailingStop action"),
        }
    }

    // ── Short position tests ──

    fn create_short_position() -> ManagedPosition {
        // Short: entry=50000, SL=51000 (above), TP=47000 (below)
        // risk_per_unit = |50000 - 51000| = 1000
        ManagedPosition::new(
            "BTCUSDT",
            Position::Short,
            dec!(50000),
            dec!(51000),
            dec!(47000),
            dec!(1),
            dec!(500),
        )
    }

    #[test]
    fn test_short_r_multiple_calculation() {
        let position = create_short_position();

        // At entry: 0R
        assert_eq!(position.current_r_multiple(dec!(50000)), dec!(0));

        // Price drops 1000 => +1R for Short
        assert_eq!(position.current_r_multiple(dec!(49000)), dec!(1));

        // Price drops 1500 => +1.5R
        assert_eq!(position.current_r_multiple(dec!(48500)), dec!(1.5));

        // Price rises 1000 => -1R
        assert_eq!(position.current_r_multiple(dec!(51000)), dec!(-1));
    }

    #[test]
    fn test_short_first_tp_trigger() {
        let position = create_short_position();

        // Below 1.5R — should not trigger
        assert!(!position.should_hit_first_tp(dec!(48600)));

        // At 1.5R (price drops to 48500) — should trigger
        assert!(position.should_hit_first_tp(dec!(48500)));

        // Above 1.5R — should trigger
        assert!(position.should_hit_first_tp(dec!(48000)));
    }

    #[test]
    fn test_short_trailing_stop_trigger() {
        let position = create_short_position();

        // Below 2.5R — should not trigger
        assert!(!position.should_activate_trailing(dec!(47600)));

        // At 2.5R (price drops to 47500) — should trigger
        assert!(position.should_activate_trailing(dec!(47500)));
    }

    #[test]
    fn test_short_evaluate_position_first_tp() {
        let position = create_short_position();

        // At 1.5R for Short (price=48500)
        let action = evaluate_position(&position, dec!(48500));

        match action {
            ManagementAction::ExecuteFirstTp {
                quantity_to_close,
                new_stop_loss,
            } => {
                assert_eq!(quantity_to_close, dec!(0.33));
                assert_eq!(new_stop_loss, dec!(50000)); // Breakeven
            }
            _ => panic!("Expected ExecuteFirstTp action"),
        }
    }

    #[test]
    fn test_short_evaluate_position_trailing() {
        let position = create_short_position();

        // At 2.5R for Short (price=47500)
        let action = evaluate_position(&position, dec!(47500));

        match action {
            ManagementAction::ActivateTrailingStop { offset } => {
                // ATR 500 * 2.0 = 1000
                assert_eq!(offset, dec!(1000));
            }
            _ => panic!("Expected ActivateTrailingStop action"),
        }
    }
}
