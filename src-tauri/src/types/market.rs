//! Market data types for OHLCV candlestick data.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::primitives::{Price, Volume};

/// OHLCV (Open, High, Low, Close, Volume) candlestick data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candle {
    /// Candle open time.
    pub timestamp: DateTime<Utc>,
    /// Opening price.
    pub open: Price,
    /// Highest price during the period.
    pub high: Price,
    /// Lowest price during the period.
    pub low: Price,
    /// Closing price.
    pub close: Price,
    /// Trading volume during the period.
    pub volume: Volume,
}

impl Candle {
    /// Creates a new candle with the given OHLCV values.
    pub fn new(
        timestamp: DateTime<Utc>,
        open: Price,
        high: Price,
        low: Price,
        close: Price,
        volume: Volume,
    ) -> Self {
        Self {
            timestamp,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    /// Returns true if the candle is bullish (close > open).
    pub fn is_bullish(&self) -> bool {
        self.close > self.open
    }

    /// Returns true if the candle is bearish (close < open).
    pub fn is_bearish(&self) -> bool {
        self.close < self.open
    }

    /// Returns the body size (absolute difference between open and close).
    pub fn body_size(&self) -> Price {
        (self.close - self.open).abs()
    }

    /// Returns the full range (high - low).
    pub fn range(&self) -> Price {
        self.high - self.low
    }
}

/// Timeframe interval for candlestick data.
///
/// The strategy operates exclusively on the 1H timeframe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Interval {
    M15,
    H1,
    H4,
}

impl Interval {
    /// Returns the interval duration in minutes.
    pub fn as_minutes(&self) -> u32 {
        match self {
            Interval::M15 => 15,
            Interval::H1 => 60,
            Interval::H4 => 240,
        }
    }

    /// Returns the Binance API interval string.
    pub fn as_binance_str(&self) -> &'static str {
        match self {
            Interval::M15 => "15m",
            Interval::H1 => "1h",
            Interval::H4 => "4h",
        }
    }
}

/// Market data for the 1H timeframe.
///
/// This struct holds OHLCV data for the 1H interval used by the
/// Regime Classifier & Signal Module strategy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MarketData {
    /// Symbol pair (e.g., "BTCUSDT").
    pub symbol: String,
    pub candles_15m: Vec<Candle>,
    pub candles_1h: Vec<Candle>,
    pub candles_4h: Vec<Candle>,
}

impl MarketData {
    /// Creates a new MarketData instance for the given symbol.
    pub fn new(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            candles_15m: Vec::new(),
            candles_1h: Vec::new(),
            candles_4h: Vec::new(),
        }
    }

    /// Returns candles for the specified interval.
    pub fn candles(&self, interval: Interval) -> &[Candle] {
        match interval {
            Interval::M15 => &self.candles_15m,
            Interval::H1 => &self.candles_1h,
            Interval::H4 => &self.candles_4h,
        }
    }

    /// Returns a mutable reference to candles for the specified interval.
    pub fn candles_mut(&mut self, interval: Interval) -> &mut Vec<Candle> {
        match interval {
            Interval::M15 => &mut self.candles_15m,
            Interval::H1 => &mut self.candles_1h,
            Interval::H4 => &mut self.candles_4h,
        }
    }

    /// Returns the latest candle for the specified interval, if available.
    pub fn latest_candle(&self, interval: Interval) -> Option<&Candle> {
        self.candles(interval).last()
    }

    /// Returns the current price (close of the latest 1H candle).
    pub fn current_price(&self) -> Option<Price> {
        self.candles_1h.last().map(|c| c.close)
    }
}

/// Converts a `binance::model::KlineSummary` (from REST API) to our internal `Candle`.
///
/// Returns `None` if any OHLCV field fails to parse.
pub fn kline_summary_to_candle(ks: &binance::model::KlineSummary) -> Option<Candle> {
    let open: Decimal = ks.open.parse().ok()?;
    let high: Decimal = ks.high.parse().ok()?;
    let low: Decimal = ks.low.parse().ok()?;
    let close: Decimal = ks.close.parse().ok()?;
    let volume: Decimal = ks.volume.parse().ok()?;

    let timestamp = DateTime::from_timestamp_millis(ks.open_time)?;

    Some(Candle::new(timestamp, open, high, low, close, volume))
}

/// Converts a `crate::websocket::KlineEvent` (from WebSocket) to our internal `Candle`.
pub fn kline_event_to_candle(event: &crate::websocket::KlineEvent) -> Option<Candle> {
    let open: Decimal = event.kline.open.parse().ok()?;
    let high: Decimal = event.kline.high.parse().ok()?;
    let low: Decimal = event.kline.low.parse().ok()?;
    let close: Decimal = event.kline.close.parse().ok()?;
    let volume: Decimal = event.kline.volume.parse().ok()?;

    // Convert millisecond timestamp to DateTime<Utc>
    let timestamp = DateTime::from_timestamp_millis(event.kline.start_time as i64)?;

    Some(Candle::new(timestamp, open, high, low, close, volume))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_candle_is_bullish() {
        let candle = Candle::new(
            Utc::now(),
            dec!(100),
            dec!(110),
            dec!(95),
            dec!(105),
            dec!(1000),
        );
        assert!(candle.is_bullish());
        assert!(!candle.is_bearish());
    }

    #[test]
    fn test_candle_is_bearish() {
        let candle = Candle::new(
            Utc::now(),
            dec!(100),
            dec!(105),
            dec!(90),
            dec!(92),
            dec!(1000),
        );
        assert!(candle.is_bearish());
        assert!(!candle.is_bullish());
    }
}
