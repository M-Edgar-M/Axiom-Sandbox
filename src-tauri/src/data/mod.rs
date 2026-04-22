//! Data management module.
//!
//! This module provides trade logging and management:
//!
//! ## TradeRecord
//! Complete trade data structure with:
//! - 8 decimal places for Binance compatibility
//! - Direction (Long/Short), Status (Open/Closed), Phase (1/2/3)
//! - Entry/exit prices, P&L, ATR for trailing stops
//!
//! ## CsvLogger
//! Thread-safe CSV persistence:
//! - `append_record()` - Add new trades
//! - `update_record()` - Modify existing trades
//! - Auto-creates `trades_log.csv` with headers
//!
//! ## TradeManager
//! Trade lifecycle orchestration:
//! - **Phase 2 (1.5R)**: SL to breakeven, close 33%
//! - **Phase 3 (2.5R)**: Trailing stop (2.0× ATR)
//! - Automatic exit on SL/TP hit

pub mod backfill;
mod csv_logger;
pub mod mock_tradfi_feed;
mod trade_manager;
mod trade_record;

pub use mock_tradfi_feed::MockCsvFeed;
pub use trade_manager::TradeManager;
pub use trade_record::{TradeDirection, TradeRecord, TradeStatus};
