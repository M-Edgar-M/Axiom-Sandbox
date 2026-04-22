//! Core types for the Axiom Sandbox trading engine.
//!
//! This module uses `rust_decimal` for all financial values to avoid
//! floating-point precision errors common in financial calculations.

mod market;
mod position;
mod primitives;
mod trade;

// Re-export all public types for convenient access
pub use market::{Candle, Interval, MarketData, kline_event_to_candle, kline_summary_to_candle};
pub use position::Position;
pub use primitives::{Balance, Price, Volume};
