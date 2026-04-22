//! Order types and builders for exchange integration.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::{Price, Volume};

/// Order side (buy or sell).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderSide {
    Buy,
    Sell,
}

impl OrderSide {
    /// Returns the opposite side.
    pub fn opposite(&self) -> Self {
        match self {
            OrderSide::Buy => OrderSide::Sell,
            OrderSide::Sell => OrderSide::Buy,
        }
    }
}

/// Order type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    /// Standard limit order.
    Limit,
    /// Market order (not recommended, for emergency only).
    Market,
    /// Stop loss limit order.
    StopLossLimit,
    /// Take profit limit order.
    TakeProfitLimit,
    /// Trailing stop order.
    TrailingStop,
}

/// Order status from exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    /// Order is pending/new.
    New,
    /// Order is partially filled.
    PartiallyFilled,
    /// Order is completely filled.
    Filled,
    /// Order was canceled.
    Canceled,
    /// Order was rejected.
    Rejected,
    /// Order expired.
    Expired,
}

impl OrderStatus {
    /// Returns true if order is still active.
    pub fn is_active(&self) -> bool {
        matches!(self, OrderStatus::New | OrderStatus::PartiallyFilled)
    }

    /// Returns true if order is terminal (no more changes expected).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            OrderStatus::Filled
                | OrderStatus::Canceled
                | OrderStatus::Rejected
                | OrderStatus::Expired
        )
    }
}

/// Request to place an order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderRequest {
    /// Trading symbol (e.g., "BTCUSDT").
    pub symbol: String,
    /// Order side.
    pub side: OrderSide,
    /// Order type.
    pub order_type: OrderType,
    /// Quantity to trade.
    pub quantity: Volume,
    /// Limit price (required for limit orders).
    pub price: Option<Price>,
    /// Stop price (for stop loss/take profit orders).
    pub stop_price: Option<Price>,
    /// Trailing delta for trailing stop (in price units).
    pub trailing_delta: Option<Price>,
    /// Client order ID for tracking.
    pub client_order_id: Option<String>,
    /// Time in force (GTC, IOC, FOK).
    pub time_in_force: TimeInForce,
}

/// Time in force for orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TimeInForce {
    /// Good till canceled.
    #[default]
    GTC,
    /// Immediate or cancel.
    IOC,
    /// Fill or kill.
    FOK,
}

impl OrderRequest {
    /// Creates a new limit order request.
    pub fn limit(
        symbol: impl Into<String>,
        side: OrderSide,
        quantity: Volume,
        price: Price,
    ) -> Self {
        Self {
            symbol: symbol.into(),
            side,
            order_type: OrderType::Limit,
            quantity,
            price: Some(price),
            stop_price: None,
            trailing_delta: None,
            client_order_id: None,
            time_in_force: TimeInForce::GTC,
        }
    }

    /// Creates a stop loss limit order request.
    pub fn stop_loss_limit(
        symbol: impl Into<String>,
        side: OrderSide,
        quantity: Volume,
        price: Price,
        stop_price: Price,
    ) -> Self {
        Self {
            symbol: symbol.into(),
            side,
            order_type: OrderType::StopLossLimit,
            quantity,
            price: Some(price),
            stop_price: Some(stop_price),
            trailing_delta: None,
            client_order_id: None,
            time_in_force: TimeInForce::GTC,
        }
    }

    /// Creates a take profit limit order request.
    pub fn take_profit_limit(
        symbol: impl Into<String>,
        side: OrderSide,
        quantity: Volume,
        price: Price,
        stop_price: Price,
    ) -> Self {
        Self {
            symbol: symbol.into(),
            side,
            order_type: OrderType::TakeProfitLimit,
            quantity,
            price: Some(price),
            stop_price: Some(stop_price),
            trailing_delta: None,
            client_order_id: None,
            time_in_force: TimeInForce::GTC,
        }
    }

    /// Sets a custom client order ID.
    pub fn with_client_id(mut self, id: impl Into<String>) -> Self {
        self.client_order_id = Some(id.into());
        self
    }
}

/// Result of an order placement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderResult {
    /// Exchange order ID.
    pub order_id: u64,
    /// Client order ID (if provided).
    pub client_order_id: Option<String>,
    /// Symbol traded.
    pub symbol: String,
    /// Order status.
    pub status: OrderStatus,
    /// Side.
    pub side: OrderSide,
    /// Order type.
    pub order_type: OrderType,
    /// Requested quantity.
    pub quantity: Volume,
    /// Filled quantity.
    pub filled_quantity: Volume,
    /// Average fill price.
    pub avg_price: Price,
    /// Timestamp.
    pub timestamp: i64,
}

impl OrderResult {
    /// Returns true if order is completely filled.
    pub fn is_filled(&self) -> bool {
        self.status == OrderStatus::Filled
    }

    /// Returns the unfilled quantity.
    pub fn remaining_quantity(&self) -> Volume {
        self.quantity - self.filled_quantity
    }
}

/// OCO (One-Cancels-Other) order request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcoOrderRequest {
    /// Trading symbol.
    pub symbol: String,
    /// Order side.
    pub side: OrderSide,
    /// Quantity for both legs.
    pub quantity: Volume,
    /// Take profit limit price.
    pub take_profit_price: Price,
    /// Stop loss trigger price.
    pub stop_loss_trigger: Price,
    /// Stop loss limit price (slightly worse than trigger).
    pub stop_loss_limit: Price,
    /// Client order list ID.
    pub list_client_order_id: Option<String>,
}

impl OcoOrderRequest {
    /// Creates a new OCO order for closing a long position.
    ///
    /// - Take profit: SELL at `take_profit_price`
    /// - Stop loss: SELL when price drops to `stop_loss_trigger`
    pub fn close_long(
        symbol: impl Into<String>,
        quantity: Volume,
        take_profit_price: Price,
        stop_loss_trigger: Price,
        slippage: Decimal,
    ) -> Self {
        // Stop loss limit slightly below trigger to ensure fill
        let stop_loss_limit = stop_loss_trigger * (Decimal::ONE - slippage);

        Self {
            symbol: symbol.into(),
            side: OrderSide::Sell,
            quantity,
            take_profit_price,
            stop_loss_trigger,
            stop_loss_limit,
            list_client_order_id: None,
        }
    }

    /// Creates a new OCO order for closing a short position.
    ///
    /// - Take profit: BUY at `take_profit_price`
    /// - Stop loss: BUY when price rises to `stop_loss_trigger`
    pub fn close_short(
        symbol: impl Into<String>,
        quantity: Volume,
        take_profit_price: Price,
        stop_loss_trigger: Price,
        slippage: Decimal,
    ) -> Self {
        // Stop loss limit slightly above trigger to ensure fill
        let stop_loss_limit = stop_loss_trigger * (Decimal::ONE + slippage);

        Self {
            symbol: symbol.into(),
            side: OrderSide::Buy,
            quantity,
            take_profit_price,
            stop_loss_trigger,
            stop_loss_limit,
            list_client_order_id: None,
        }
    }
}

/// Result of an OCO order placement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcoOrderResult {
    /// Order list ID.
    pub order_list_id: u64,
    /// Client order list ID.
    pub list_client_order_id: Option<String>,
    /// Symbol.
    pub symbol: String,
    /// Take profit order ID.
    pub take_profit_order_id: u64,
    /// Stop loss order ID.
    pub stop_loss_order_id: u64,
    /// Status of the order list.
    pub status: OcoStatus,
}

/// Status of an OCO order list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OcoStatus {
    /// Both orders are active.
    Executing,
    /// One order filled, other canceled.
    AllDone,
    /// Order list rejected.
    Reject,
}

/// Errors that can occur during order execution.
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum ExecutionError {
    /// Funding rate exceeds maximum allowed.
    #[error("Funding rate {rate} exceeds limit {limit}")]
    FundingRateTooHigh { rate: Decimal, limit: Decimal },

    /// Slippage exceeds maximum allowed.
    #[error("Slippage exceeded: expected {expected}, got {actual}")]
    SlippageExceeded { expected: Price, actual: Price },

    /// Order was rejected by exchange.
    #[error("Order rejected: {reason}")]
    OrderRejected { reason: String },

    /// Insufficient balance for order.
    #[error("Insufficient balance: need {required}, have {available}")]
    InsufficientBalance {
        required: Decimal,
        available: Decimal,
    },

    /// Position not found.
    #[error("Position not found: {symbol}")]
    PositionNotFound { symbol: String },

    /// Exchange API error.
    #[error("Exchange error: {message}")]
    ExchangeError { message: String },

    /// Network/connection error.
    #[error("Connection error: {message}")]
    ConnectionError { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_order_side_opposite() {
        assert_eq!(OrderSide::Buy.opposite(), OrderSide::Sell);
        assert_eq!(OrderSide::Sell.opposite(), OrderSide::Buy);
    }

    #[test]
    fn test_order_status_active() {
        assert!(OrderStatus::New.is_active());
        assert!(OrderStatus::PartiallyFilled.is_active());
        assert!(!OrderStatus::Filled.is_active());
        assert!(!OrderStatus::Canceled.is_active());
    }

    #[test]
    fn test_limit_order_creation() {
        let order = OrderRequest::limit("BTCUSDT", OrderSide::Buy, dec!(0.1), dec!(50000));

        assert_eq!(order.symbol, "BTCUSDT");
        assert_eq!(order.side, OrderSide::Buy);
        assert_eq!(order.order_type, OrderType::Limit);
        assert_eq!(order.quantity, dec!(0.1));
        assert_eq!(order.price, Some(dec!(50000)));
    }

    #[test]
    fn test_oco_close_long() {
        let oco = OcoOrderRequest::close_long(
            "BTCUSDT",
            dec!(0.1),
            dec!(55000), // TP
            dec!(48000), // SL trigger
            dec!(0.002), // 0.2% slippage
        );

        assert_eq!(oco.side, OrderSide::Sell);
        assert_eq!(oco.take_profit_price, dec!(55000));
        assert_eq!(oco.stop_loss_trigger, dec!(48000));
        // SL limit = 48000 * 0.998 = 47904
        assert_eq!(oco.stop_loss_limit, dec!(47904));
    }

    #[test]
    fn test_oco_close_short() {
        let oco = OcoOrderRequest::close_short(
            "BTCUSDT",
            dec!(0.1),
            dec!(45000), // TP
            dec!(52000), // SL trigger
            dec!(0.002), // 0.2% slippage
        );

        assert_eq!(oco.side, OrderSide::Buy);
        // SL limit = 52000 * 1.002 = 52104
        assert_eq!(oco.stop_loss_limit, dec!(52104));
    }
}
