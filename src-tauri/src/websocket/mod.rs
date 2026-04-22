//! WebSocket manager module for Binance streams.
//!
//! ## Features
//! - **Dual Streams**: Market data (AggTrade, Klines) + User data (Orders)
//! - **Proactive Rotation**: 23-hour reconnect before Binance 24h limit
//! - **Exponential Backoff**: 1s→60s with jitter
//! - **Circuit Breaker**: 10 disconnects/5min triggers shutdown
//! - **Heartbeat Protocol**: Reactive Ping/Pong, 60s market, 3min user
//! - **Phase Detection**: 1.5R/2.5R triggers for strategy integration
//!
//! ## Usage
//!
//! ```ignore
//! use crate::websocket::{WebSocketManager, ManagerConfig};
//!
//! let (price_tx, price_rx) = mpsc::channel(1000);
//! let (order_tx, order_rx) = mpsc::channel(100);
//!
//! let mut manager = WebSocketManager::new(
//!     ManagerConfig::default(),
//!     price_tx,
//!     order_tx,
//! );
//!
//! manager.start().await?;
//! ```

pub mod config;
mod connection;
mod manager;
mod messages;
mod streams;

pub use manager::{ManagerConfig, SystemEvent, WebSocketManager};
pub use messages::KlineEvent;
