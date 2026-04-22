//! Risk management configuration constants.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// Risk per trade as a fraction of equity (5% = 0.05).
pub const RISK_PER_TRADE: Decimal = dec!(0.050);

/// Maximum position size as a fraction of equity (500% = 5.0).
pub const MAX_POSITION_EQUITY_PCT: Decimal = dec!(5.0);

/// ATR ratio threshold for 50% position size reduction.
/// If 15M_ATR / 4H_ATR > 2.0, reduce by 50%.
pub const VOLATILITY_REDUCE_THRESHOLD: Decimal = dec!(2.0);

/// ATR ratio threshold to skip trade entirely.
/// If 15M_ATR / 4H_ATR > 3.0, return error.
pub const VOLATILITY_SKIP_THRESHOLD: Decimal = dec!(3.0);

/// Daily loss limit as a fraction of equity (2.5% = 0.025).
/// If daily loss exceeds this, halt trading for the day.
pub const DAILY_LOSS_LIMIT: Decimal = dec!(0.025);

/// Market cap threshold for small cap coins (< $1B).
/// Positions in small cap coins are reduced by 50%.
pub const SMALL_CAP_THRESHOLD: Decimal = dec!(1_000_000_000);

/// Position size reduction factor for high volatility (50%).
pub const VOLATILITY_REDUCTION_FACTOR: Decimal = dec!(0.5);

/// Position size reduction factor for small cap coins (50%).
pub const SMALL_CAP_REDUCTION_FACTOR: Decimal = dec!(0.5);
