//! Binance WebSocket message types.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Aggregate trade event from Binance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggTrade {
    /// Event type.
    #[serde(rename = "e")]
    pub event_type: String,
    /// Event time.
    #[serde(rename = "E")]
    pub event_time: u64,
    /// Symbol.
    #[serde(rename = "s")]
    pub symbol: String,
    /// Aggregate trade ID.
    #[serde(rename = "a")]
    pub agg_trade_id: u64,
    /// Price.
    #[serde(rename = "p")]
    pub price: String,
    /// Quantity.
    #[serde(rename = "q")]
    pub quantity: String,
    /// First trade ID.
    #[serde(rename = "f")]
    pub first_trade_id: u64,
    /// Last trade ID.
    #[serde(rename = "l")]
    pub last_trade_id: u64,
    /// Trade time.
    #[serde(rename = "T")]
    pub trade_time: u64,
    /// Is buyer the maker.
    #[serde(rename = "m")]
    pub is_buyer_maker: bool,
}

impl AggTrade {
    /// Parse price as Decimal.
    pub fn price_decimal(&self) -> Option<Decimal> {
        self.price.parse().ok()
    }

    /// Parse quantity as Decimal.
    pub fn quantity_decimal(&self) -> Option<Decimal> {
        self.quantity.parse().ok()
    }
}

/// Kline/Candlestick event from Binance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KlineEvent {
    /// Event type.
    #[serde(rename = "e")]
    pub event_type: String,
    /// Event time.
    #[serde(rename = "E")]
    pub event_time: u64,
    /// Symbol.
    #[serde(rename = "s")]
    pub symbol: String,
    /// Kline data.
    #[serde(rename = "k")]
    pub kline: KlineData,
}

/// Kline data within a KlineEvent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KlineData {
    /// Kline start time.
    #[serde(rename = "t")]
    pub start_time: u64,
    /// Kline close time.
    #[serde(rename = "T")]
    pub close_time: u64,
    /// Symbol.
    #[serde(rename = "s")]
    pub symbol: String,
    /// Interval.
    #[serde(rename = "i")]
    pub interval: String,
    /// Open price.
    #[serde(rename = "o")]
    pub open: String,
    /// Close price.
    #[serde(rename = "c")]
    pub close: String,
    /// High price.
    #[serde(rename = "h")]
    pub high: String,
    /// Low price.
    #[serde(rename = "l")]
    pub low: String,
    /// Base asset volume.
    #[serde(rename = "v")]
    pub volume: String,
    /// Number of trades.
    #[serde(rename = "n")]
    pub num_trades: u64,
    /// Is this kline closed.
    #[serde(rename = "x")]
    pub is_closed: bool,
    /// Quote asset volume.
    #[serde(rename = "q")]
    pub quote_volume: String,
}

impl KlineData {
    /// Parse close price as Decimal.
    pub fn close_decimal(&self) -> Option<Decimal> {
        self.close.parse().ok()
    }

    /// Parse open price as Decimal.
    pub fn open_decimal(&self) -> Option<Decimal> {
        self.open.parse().ok()
    }

    /// Parse high price as Decimal.
    pub fn high_decimal(&self) -> Option<Decimal> {
        self.high.parse().ok()
    }

    /// Parse low price as Decimal.
    pub fn low_decimal(&self) -> Option<Decimal> {
        self.low.parse().ok()
    }

    /// Parse volume as Decimal.
    pub fn volume_decimal(&self) -> Option<Decimal> {
        self.volume.parse().ok()
    }
}

/// Account update from user data stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountUpdate {
    /// Event type.
    #[serde(rename = "e")]
    pub event_type: String,
    /// Event time.
    #[serde(rename = "E")]
    pub event_time: u64,
    /// Balances.
    #[serde(rename = "B", default)]
    pub balances: Vec<BalanceUpdate>,
}

/// Balance update within AccountUpdate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceUpdate {
    /// Asset.
    #[serde(rename = "a")]
    pub asset: String,
    /// Free balance.
    #[serde(rename = "f")]
    pub free: String,
    /// Locked balance.
    #[serde(rename = "l")]
    pub locked: String,
}

impl BalanceUpdate {
    /// Parse free balance as Decimal.
    pub fn free_decimal(&self) -> Option<Decimal> {
        self.free.parse().ok()
    }

    /// Parse locked balance as Decimal.
    pub fn locked_decimal(&self) -> Option<Decimal> {
        self.locked.parse().ok()
    }
}

/// Order update from user data stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderUpdate {
    /// Event type.
    #[serde(rename = "e")]
    pub event_type: String,
    /// Event time.
    #[serde(rename = "E")]
    pub event_time: u64,
    /// Symbol.
    #[serde(rename = "s")]
    pub symbol: String,
    /// Client order ID.
    #[serde(rename = "c")]
    pub client_order_id: String,
    /// Side (BUY/SELL).
    #[serde(rename = "S")]
    pub side: String,
    /// Order type (LIMIT, MARKET, etc.).
    #[serde(rename = "o")]
    pub order_type: String,
    /// Time in force.
    #[serde(rename = "f")]
    pub time_in_force: String,
    /// Order quantity.
    #[serde(rename = "q")]
    pub quantity: String,
    /// Order price.
    #[serde(rename = "p")]
    pub price: String,
    /// Stop price.
    #[serde(rename = "P")]
    pub stop_price: String,
    /// Current order status.
    #[serde(rename = "X")]
    pub status: String,
    /// Order ID.
    #[serde(rename = "i")]
    pub order_id: u64,
    /// Last executed quantity.
    #[serde(rename = "l")]
    pub last_executed_qty: String,
    /// Last executed price.
    #[serde(rename = "L")]
    pub last_executed_price: String,
    /// Cumulative filled quantity.
    #[serde(rename = "z")]
    pub cumulative_qty: String,
}

impl OrderUpdate {
    /// Parse order price as Decimal.
    pub fn price_decimal(&self) -> Option<Decimal> {
        self.price.parse().ok()
    }

    /// Parse quantity as Decimal.
    pub fn quantity_decimal(&self) -> Option<Decimal> {
        self.quantity.parse().ok()
    }

    /// Returns true if order is filled.
    pub fn is_filled(&self) -> bool {
        self.status == "FILLED"
    }

    /// Returns true if order is cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.status == "CANCELED" || self.status == "CANCELLED"
    }
}

/// Parsed WebSocket message.
#[derive(Debug, Clone)]
pub enum WsMessage {
    /// Aggregate trade.
    AggTrade(AggTrade),
    /// Kline update.
    Kline(KlineEvent),
    /// Account update.
    Account(AccountUpdate),
    /// Order update.
    Order(OrderUpdate),
    /// Ping frame received.
    Ping(Vec<u8>),
    /// Connection closed.
    Closed,
    /// Unknown/unparsed message.
    Unknown(String),
}

/// Price tick event for strategy integration.
#[derive(Debug, Clone)]
pub struct PriceTick {
    /// Symbol.
    pub symbol: String,
    /// Current price.
    pub price: Decimal,
    /// Timestamp.
    pub timestamp: u64,
}

/// Order event for execution engine.
#[derive(Debug, Clone)]
pub enum OrderEvent {
    /// Order filled.
    Filled {
        symbol: String,
        order_id: u64,
        side: String,
        price: Decimal,
        quantity: Decimal,
    },
    /// Order cancelled.
    Cancelled { symbol: String, order_id: u64 },
    /// Order rejected.
    Rejected {
        symbol: String,
        order_id: u64,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_agg_trade() {
        let json = r#"{
            "e": "aggTrade",
            "E": 1234567890,
            "s": "BTCUSDT",
            "a": 12345,
            "p": "50000.00",
            "q": "1.5",
            "f": 100,
            "l": 105,
            "T": 1234567890,
            "m": true
        }"#;

        let trade: AggTrade = serde_json::from_str(json).unwrap();
        assert_eq!(trade.symbol, "BTCUSDT");
        assert_eq!(trade.price_decimal().unwrap().to_string(), "50000.00");
    }

    #[test]
    fn test_parse_kline() {
        let json = r#"{
            "e": "kline",
            "E": 1234567890,
            "s": "BTCUSDT",
            "k": {
                "t": 1234567800,
                "T": 1234567899,
                "s": "BTCUSDT",
                "i": "15m",
                "o": "49000.00",
                "c": "50000.00",
                "h": "50500.00",
                "l": "48500.00",
                "v": "100.5",
                "n": 1000,
                "x": true,
                "q": "5000000.00"
            }
        }"#;

        let kline: KlineEvent = serde_json::from_str(json).unwrap();
        assert_eq!(kline.kline.interval, "15m");
        assert!(kline.kline.is_closed);
    }

    #[test]
    fn test_parse_order_update() {
        let json = r#"{
            "e": "executionReport",
            "E": 1234567890,
            "s": "BTCUSDT",
            "c": "my_order_123",
            "S": "BUY",
            "o": "LIMIT",
            "f": "GTC",
            "q": "1.0",
            "p": "50000.00",
            "P": "0.0",
            "X": "FILLED",
            "i": 12345,
            "l": "1.0",
            "L": "50000.00",
            "z": "1.0"
        }"#;

        let order: OrderUpdate = serde_json::from_str(json).unwrap();
        assert!(order.is_filled());
        assert!(!order.is_cancelled());
    }
}
