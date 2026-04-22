//! Exchange configuration constants.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// Maximum acceptable slippage for limit orders (0.2% for top 10 coins).
pub const MAX_SLIPPAGE_PCT: Decimal = dec!(0.002);

/// Maximum funding rate before skipping trade (0.03%).
pub const FUNDING_RATE_LIMIT: Decimal = dec!(0.0003);

/// R-multiple at which to move stop loss to breakeven.
pub const FIRST_TP_R_MULTIPLE: Decimal = dec!(1.5);

/// Percentage of position to close at first take profit (33%).
pub const FIRST_TP_CLOSE_PCT: Decimal = dec!(0.33);

/// R-multiple at which to activate trailing stop.
pub const TRAILING_ACTIVATION_R: Decimal = dec!(2.5);

/// ATR multiplier for trailing stop offset.
pub const TRAILING_ATR_MULTIPLIER: Decimal = dec!(2.0);

/// Top 10 coins by market cap (for slippage rules).
pub const TOP_COINS: [&str; 10] = [
    "BTCUSDT",
    "ETHUSDT",
    "BNBUSDT",
    "XRPUSDT",
    "ADAUSDT",
    "SOLUSDT",
    "DOGEUSDT",
    "DOTUSDT",
    "MATICUSDT",
    "LTCUSDT",
];

/// Order retry attempts before giving up.
pub const ORDER_RETRY_ATTEMPTS: u32 = 3;

/// Delay between order retries in milliseconds.
pub const ORDER_RETRY_DELAY_MS: u64 = 500;

/// Maximum allowed simultaneous open trades per symbol.
pub const MAX_OPEN_POSITIONS: usize = 3;

/// Minimum minutes between new entries for the same symbol.
pub const TRADE_COOLDOWN_MINUTES: i64 = 15;
