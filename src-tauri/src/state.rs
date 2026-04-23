//! Tauri managed application state.
//!
//! [`AppState`] is the single shared-state container held by Tauri's `.manage()`
//! system.  Every `#[tauri::command]` receives a `State<'_, AppState>` parameter
//! and interacts with the backend through it.
//!
//! ## Thread-safety model
//!
//! | Field                | Guard           | Reason                                                |
//! |----------------------|-----------------|-------------------------------------------------------|
//! | `engine`             | `Mutex`         | Mutation is rare (start/stop) — exclusive access OK   |
//! | `ws_manager`         | `Mutex`         | `start()` takes `&mut self` — needs exclusive access  |
//! | `risk_manager`       | `Mutex`         | Updated on every trade result                         |
//! | `session_active`     | `AtomicBool`    | Hot-path read — avoid lock overhead                   |
//!
//! All Mutexes are `tokio::sync::Mutex` so `await` inside a lock is safe and
//! the Tauri UI thread is never blocked.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tokio::sync::{Mutex, mpsc};

use crate::exchange::{EngineConfig, ExecutionEngine};
use crate::risk::RiskManager;
use crate::websocket::{KlineEvent, ManagerConfig, OrderEvent, PriceTick, SystemEvent, WebSocketManager};

/// Thread-safe application state managed by Tauri.
///
/// Constructed once in `lib.rs::run()` and handed to `.manage()`.
/// Cloned cheaply via `Arc` on every IPC call.
pub struct AppState {
    /// The Binance execution engine (mock or live).
    /// `None` before `start_mock_session` is called.
    pub engine: Mutex<Option<ExecutionEngine>>,

    /// The WebSocket supervisor (market + user data streams).
    /// `None` before `start_mock_session` is called.
    pub ws_manager: Mutex<Option<WebSocketManager>>,

    /// Risk manager — tracks equity, daily stats, circuit breaker.
    pub risk_manager: Mutex<RiskManager>,

    /// True while a trading session is running.
    pub session_active: Arc<AtomicBool>,

    /// Channel for receiving `SystemEvent::Reconnected` from the WebSocket layer.
    /// Held here so the Tauri runtime can poll it without needing the full manager lock.
    pub system_rx: Mutex<Option<mpsc::Receiver<SystemEvent>>>,
}

impl AppState {
    /// Creates the initial idle state with a RiskManager seeded at $10,000 paper equity.
    ///
    /// The engine and WebSocket manager are `None` until a session starts.
    pub fn new() -> Self {
        Self {
            engine: Mutex::new(None),
            ws_manager: Mutex::new(None),
            risk_manager: Mutex::new(RiskManager::new(dec!(10_000))),
            session_active: Arc::new(AtomicBool::new(false)),
            system_rx: Mutex::new(None),
        }
    }

    /// Returns `true` if a trading session is currently active.
    pub fn is_session_active(&self) -> bool {
        self.session_active.load(Ordering::SeqCst)
    }

    /// Builds a [`WebSocketManager`] + channel set for the given symbols/intervals.
    ///
    /// The `system_rx` end is stored in `self.system_rx` so IPC commands can poll
    /// reconnect signals without holding the full `ws_manager` lock.
    ///
    /// Returns the `(price_rx, order_rx, kline_rx)` receivers the engine loop needs.
    pub async fn build_ws_stack(
        &self,
        symbols: Vec<String>,
        intervals: Vec<String>,
    ) -> (
        mpsc::Receiver<PriceTick>,
        mpsc::Receiver<OrderEvent>,
        mpsc::Receiver<KlineEvent>,
    ) {
        let (price_tx, price_rx) = mpsc::channel(1_000);
        let (order_tx, order_rx) = mpsc::channel(100);
        let (kline_tx, kline_rx) = mpsc::channel(256);
        let (system_tx, system_rx) = mpsc::channel(32);

        let cfg = ManagerConfig {
            symbols,
            intervals,
            listen_key: None,
            enable_market_stream: true,
            enable_user_stream: false,
        };

        let manager = WebSocketManager::new(cfg, price_tx, order_tx, kline_tx, system_tx);

        *self.ws_manager.lock().await = Some(manager);
        *self.system_rx.lock().await = Some(system_rx);

        (price_rx, order_rx, kline_rx)
    }

    /// Builds a mock [`ExecutionEngine`] (testnet: true, no real HTTP calls).
    pub async fn build_mock_engine(&self) {
        let config = EngineConfig {
            api_key: std::env::var("BINANCE_API_KEY").unwrap_or_default(),
            api_secret: std::env::var("BINANCE_API_SECRET").unwrap_or_default(),
            testnet: true, // ← Always testnet for mock sessions.
            ..Default::default()
        };

        *self.engine.lock().await = Some(ExecutionEngine::new_mock());
        // config is kept for reference; the mock engine ignores exchange calls.
        let _ = config;
    }

    /// Sends the WebSocket shutdown signal and marks session inactive.
    pub async fn stop_session(&self) {
        if let Some(manager) = self.ws_manager.lock().await.as_ref() {
            manager.shutdown();
        }
        self.session_active.store(false, Ordering::SeqCst);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
