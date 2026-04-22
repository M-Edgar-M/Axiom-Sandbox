//! Risk management module.
//!
//! This module provides strict risk management for the trading bot:
//!
//! ## Position Sizing
//! - Base formula: `(Equity × 0.8%) / |Entry - StopLoss|`
//! - Volatility scalar: 50% reduction if 15M_ATR / 4H_ATR > 2.0
//! - Market cap scalar: 50% reduction for coins < $1B
//! - Liquidity cap: Maximum 3% of equity per position
//!
//! ## Circuit Breaker
//! - Daily loss limit: 2.5% of starting equity
//! - When triggered, all trading halts until `reset_daily()`
//!
//! ## Usage
//!
//! ```ignore
//! use crate::risk::RiskManager;
//!
//! let mut manager = RiskManager::new(dec!(100_000));
//!
//! let result = manager.calculate_position_size(
//!     dec!(50_000),  // entry
//!     dec!(49_000),  // stop loss
//!     dec!(100),     // 15M ATR
//!     dec!(100),     // 4H ATR
//!     Some(dec!(5_000_000_000)), // market cap
//! )?;
//! ```

pub mod config;
mod manager;
mod sizing;

pub use manager::RiskManager;
