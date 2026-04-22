//! Primitive financial types using rust_decimal for precision.

use rust_decimal::Decimal;

/// Price in quote currency (e.g., USDT for BTC/USDT pair).
pub type Price = Decimal;

/// Volume/quantity of the base asset.
pub type Volume = Decimal;

/// Account balance in quote currency.
pub type Balance = Decimal;
