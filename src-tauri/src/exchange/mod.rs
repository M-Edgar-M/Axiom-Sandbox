//! Exchange integration module.
//!
//! This module provides Binance exchange integration with:
//!
//! ## Order Management
//! - OCO (One-Cancels-Other) orders for simultaneous SL and TP
//! - Limit orders only to minimize slippage (max 0.2%)
//!
//! ## In-Trade Management
//! - **1.5R Hit**: Move SL to breakeven, close 33% of position
//! - **2.5R Hit**: Activate trailing stop (2.0× ATR offset)
//!
//! ## Safety Guards
//! - Skip trades if funding rate > 0.03%
//! - Slippage protection for top 10 coins
//!
//! ## Usage
//!
//! ```ignore
//! use crate::exchange::{ExecutionEngine, EngineConfig};
//!
//! let config = EngineConfig {
//!     api_key: "your_key".to_string(),
//!     api_secret: "your_secret".to_string(),
//!     testnet: true,
//!     ..Default::default()
//! };
//!
//! let engine = ExecutionEngine::new(config);
//! ```

pub mod config;
mod engine;
pub mod ibkr_engine;
mod orders;
mod trade_manager;

pub use engine::{EngineConfig, ExecutionEngine};
pub use ibkr_engine::{IbkrEngineConfig, IbkrExecutionEngine};
pub use orders::ExecutionError;
pub use trade_manager::ManagementAction;
