//! WebSocket configuration constants.

use std::time::Duration;

/// Binance WebSocket base URL (Spot).
pub const BINANCE_WS_BASE: &str = "wss://stream.binance.com:9443";

/// Binance WebSocket base URL for Futures.
pub const BINANCE_FUTURES_WS_BASE: &str = "wss://stream.binancefuture.com";

/// Binance REST API base URL (for backfill).
pub const BINANCE_REST_BASE: &str = "https://api.binance.com";

/// Proactive rotation interval (23 hours, before 24h limit).
pub const ROTATION_INTERVAL: Duration = Duration::from_secs(23 * 60 * 60);

/// Market stream heartbeat timeout.
/// Reconnect if no message received within this duration.
pub const MARKET_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(200);

/// Market stream activity check interval.
pub const MARKET_ACTIVITY_CHECK: Duration = Duration::from_secs(20);

/// User data stream heartbeat timeout.
pub const USER_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(3 * 60);

/// Initial reconnection delay.
pub const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Maximum reconnection delay.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Backoff multiplier.
pub const BACKOFF_MULTIPLIER: u32 = 2;

/// Maximum jitter to add to backoff (in milliseconds).
pub const MAX_JITTER_MS: u64 = 1000;

/// Circuit breaker: max disconnections within window.
pub const CIRCUIT_BREAKER_LIMIT: u32 = 10;

/// Circuit breaker: time window for counting disconnections.
pub const CIRCUIT_BREAKER_WINDOW: Duration = Duration::from_secs(5 * 60);

/// Number of klines to fetch for backfill on reconnect.
pub const BACKFILL_KLINE_COUNT: u32 = 100;

/// Channel buffer size for price events.
pub const PRICE_CHANNEL_SIZE: usize = 1000;

/// Channel buffer size for order events.
pub const ORDER_CHANNEL_SIZE: usize = 100;

/// R-multiple for Phase 2 trigger (move SL to breakeven, close 33%).
pub const PHASE2_R_MULTIPLE: f64 = 1.5;

/// R-multiple for Phase 3 trigger (activate trailing stop).
pub const PHASE3_R_MULTIPLE: f64 = 2.5;

/// ATR multiplier for trailing stop.
pub const TRAILING_ATR_MULT: f64 = 2.0;

/// Percentage of position to close at Phase 2.
pub const PHASE2_CLOSE_PERCENT: f64 = 0.33;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_values() {
        assert_eq!(ROTATION_INTERVAL.as_secs(), 23 * 60 * 60);
        assert_eq!(MARKET_HEARTBEAT_TIMEOUT.as_secs(), 200);
        assert_eq!(USER_HEARTBEAT_TIMEOUT.as_secs(), 180);
        assert_eq!(CIRCUIT_BREAKER_LIMIT, 10);
    }
}
