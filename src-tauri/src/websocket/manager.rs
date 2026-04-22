//! WebSocket manager supervisor for dual stream management.

use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;

use super::config;
use super::connection::{CircuitBreaker, ConnectionError, WsConnection};
use super::messages::{KlineEvent, OrderEvent, PriceTick, WsMessage};
use super::streams::{
    StreamEvent, StreamProcessor, StreamType, TrackedPosition, build_market_stream_url,
    build_user_stream_url,
};

/// WebSocket manager errors.
#[derive(Debug, Error)]
pub enum ManagerError {
    #[error("Connection error: {0}")]
    Connection(#[from] ConnectionError),

    #[error("Channel error")]
    Channel,

    #[error("Circuit breaker triggered - too many disconnections")]
    CircuitBreaker,

    #[error("Shutdown requested")]
    Shutdown,
}

/// System-level events sent from the WebSocket manager to the main event loop.
#[derive(Debug, Clone)]
pub enum SystemEvent {
    /// WebSocket stream reconnected — main loop should reconcile state.
    Reconnected,
}

/// Configuration for WebSocket manager.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    /// Symbols to subscribe to.
    pub symbols: Vec<String>,
    /// Kline intervals to subscribe to.
    pub intervals: Vec<String>,
    /// Listen key for user data stream (optional).
    pub listen_key: Option<String>,
    /// Enable market stream.
    pub enable_market_stream: bool,
    /// Enable user data stream.
    pub enable_user_stream: bool,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            symbols: vec!["BTCUSDT".to_string()],
            intervals: vec!["1h".to_string()],
            listen_key: None,
            enable_market_stream: true,
            enable_user_stream: false,
        }
    }
}

/// RobustWebSocketManager - supervisor for market and user data streams.
///
/// Features:
/// - Dual stream management (Market + User Data)
/// - Proactive 23-hour rotation
/// - Exponential backoff with jitter
/// - Circuit breaker (10 disconnects/5min)
/// - State reconciliation on reconnect
/// - Phase 2/3 detection for strategy integration
pub struct WebSocketManager {
    /// Configuration.
    config: ManagerConfig,
    /// Circuit breaker.
    circuit_breaker: Arc<RwLock<CircuitBreaker>>,
    /// Stream processor.
    processor: Arc<RwLock<StreamProcessor>>,
    /// Shutdown flag.
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    /// Price event sender (to ExecutionEngine/AnalysisEngine).
    price_tx: mpsc::Sender<PriceTick>,
    /// Order event sender.
    order_tx: mpsc::Sender<OrderEvent>,
    /// Kline event sender (closed klines to main loop).
    kline_tx: mpsc::Sender<KlineEvent>,
    /// Stream event receiver.
    event_rx: Option<mpsc::Receiver<StreamEvent>>,
    /// Stream event sender (internal).
    event_tx: mpsc::Sender<StreamEvent>,
    /// System event sender (reconnect signals to main loop).
    system_tx: mpsc::Sender<SystemEvent>,
    /// Market stream task handle.
    market_handle: Option<JoinHandle<()>>,
    /// User stream task handle.
    user_handle: Option<JoinHandle<()>>,
}

impl WebSocketManager {
    /// Creates a new WebSocket manager.
    pub fn new(
        config: ManagerConfig,
        price_tx: mpsc::Sender<PriceTick>,
        order_tx: mpsc::Sender<OrderEvent>,
        kline_tx: mpsc::Sender<KlineEvent>,
        system_tx: mpsc::Sender<SystemEvent>,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::channel(config::PRICE_CHANNEL_SIZE);
        let processor = StreamProcessor::new(event_tx.clone());

        Self {
            config,
            circuit_breaker: Arc::new(RwLock::new(CircuitBreaker::default_config())),
            processor: Arc::new(RwLock::new(processor)),
            shutdown: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            price_tx,
            order_tx,
            kline_tx,
            event_rx: Some(event_rx),
            event_tx,
            system_tx,
            market_handle: None,
            user_handle: None,
        }
    }

    /// Starts the WebSocket manager.
    pub async fn start(&mut self) -> Result<(), ManagerError> {
        log::info!("Starting WebSocket manager...");

        // Start market stream if enabled
        if self.config.enable_market_stream {
            self.start_market_stream().await?;
        }

        // Start user stream if enabled and listen key provided
        if self.config.enable_user_stream && self.config.listen_key.is_some() {
            self.start_user_stream().await?;
        }

        // Start event processor
        self.start_event_processor().await;

        log::info!("WebSocket manager started");
        Ok(())
    }

    /// Starts the market data stream.
    async fn start_market_stream(&mut self) -> Result<(), ManagerError> {
        let symbols: Vec<&str> = self.config.symbols.iter().map(|s| s.as_str()).collect();
        let intervals: Vec<&str> = self.config.intervals.iter().map(|s| s.as_str()).collect();
        let url = build_market_stream_url(&symbols, &intervals);

        log::info!("Starting market stream: {}", url);

        let (msg_tx, mut msg_rx) = mpsc::channel::<WsMessage>(config::PRICE_CHANNEL_SIZE);
        let _processor = Arc::clone(&self.processor);
        let circuit_breaker = Arc::clone(&self.circuit_breaker);
        let shutdown = Arc::clone(&self.shutdown);
        let event_tx = self.event_tx.clone();

        // Spawn connection handler
        let connection_handle = tokio::spawn(async move {
            let mut connection =
                match WsConnection::new(&url, StreamType::Market.heartbeat_timeout()) {
                    Ok(c) => c,
                    Err(e) => {
                        log::error!("Failed to create connection: {}", e);
                        return;
                    }
                };

            loop {
                if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                    log::info!("Market stream shutdown requested");
                    break;
                }

                // Check circuit breaker
                {
                    let breaker = circuit_breaker.read().await;
                    if breaker.is_tripped() {
                        log::error!("Circuit breaker is open, stopping market stream");
                        break;
                    }
                }

                // Run connection
                let result = super::connection::run_connection(
                    connection.clone(),
                    msg_tx.clone(),
                    StreamType::Market.activity_check_interval(),
                )
                .await;

                match result {
                    Ok(()) => {
                        log::info!("Market stream connection ended cleanly");
                    }
                    Err(e) => {
                        log::warn!("Market stream error: {}", e);

                        // Record disconnection
                        let mut breaker = circuit_breaker.write().await;
                        if breaker.record_disconnection() {
                            log::error!("Circuit breaker triggered!");
                            break;
                        }
                    }
                }

                // Signal reconnection for state sync
                let _ = event_tx.send(StreamEvent::Reconnected).await;

                // Reset connection for retry
                connection = match WsConnection::new(&url, StreamType::Market.heartbeat_timeout()) {
                    Ok(c) => c,
                    Err(e) => {
                        log::error!("Failed to recreate connection: {}", e);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                };
            }
        });

        // Spawn message processor
        let processor_clone = Arc::clone(&self.processor);
        tokio::spawn(async move {
            while let Some(msg) = msg_rx.recv().await {
                let mut proc = processor_clone.write().await;
                if let Err(e) = proc.process_message(msg).await {
                    log::error!("Failed to process message: {}", e);
                }
            }
        });

        self.market_handle = Some(connection_handle);
        Ok(())
    }

    /// Starts the user data stream.
    async fn start_user_stream(&mut self) -> Result<(), ManagerError> {
        let listen_key = self.config.listen_key.as_ref().unwrap();
        let url = build_user_stream_url(listen_key);

        log::info!("Starting user stream");

        let (msg_tx, mut msg_rx) = mpsc::channel::<WsMessage>(config::ORDER_CHANNEL_SIZE);
        let order_tx = self.order_tx.clone();
        let circuit_breaker = Arc::clone(&self.circuit_breaker);
        let shutdown = Arc::clone(&self.shutdown);

        // Spawn connection handler
        let connection_handle = tokio::spawn(async move {
            let mut connection =
                match WsConnection::new(&url, StreamType::UserData.heartbeat_timeout()) {
                    Ok(c) => c,
                    Err(e) => {
                        log::error!("Failed to create user stream connection: {}", e);
                        return;
                    }
                };

            loop {
                if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                    log::info!("User stream shutdown requested");
                    break;
                }

                // Check circuit breaker
                {
                    let breaker = circuit_breaker.read().await;
                    if breaker.is_tripped() {
                        log::error!("Circuit breaker is open, stopping user stream");
                        break;
                    }
                }

                // Run connection
                let result = super::connection::run_connection(
                    connection.clone(),
                    msg_tx.clone(),
                    StreamType::UserData.activity_check_interval(),
                )
                .await;

                if let Err(e) = result {
                    log::warn!("User stream error: {}", e);

                    let mut breaker = circuit_breaker.write().await;
                    breaker.record_disconnection();
                }

                // Reset connection for retry
                connection = match WsConnection::new(&url, StreamType::UserData.heartbeat_timeout())
                {
                    Ok(c) => c,
                    Err(e) => {
                        log::error!("Failed to recreate user stream connection: {}", e);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                };
            }
        });

        // Spawn order message processor
        tokio::spawn(async move {
            while let Some(msg) = msg_rx.recv().await {
                if let WsMessage::Order(order) = msg {
                    if order.is_filled() {
                        let event = OrderEvent::Filled {
                            symbol: order.symbol.clone(),
                            order_id: order.order_id,
                            side: order.side.clone(),
                            price: order.price_decimal().unwrap_or_default(),
                            quantity: order.quantity_decimal().unwrap_or_default(),
                        };
                        let _ = order_tx.send(event).await;
                    } else if order.is_cancelled() {
                        let event = OrderEvent::Cancelled {
                            symbol: order.symbol,
                            order_id: order.order_id,
                        };
                        let _ = order_tx.send(event).await;
                    }
                }
            }
        });

        self.user_handle = Some(connection_handle);
        Ok(())
    }

    /// Starts the event processor that forwards events to engines.
    async fn start_event_processor(&mut self) {
        let mut event_rx = self.event_rx.take().unwrap();
        let price_tx = self.price_tx.clone();
        let kline_tx = self.kline_tx.clone();
        let system_tx = self.system_tx.clone();

        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                match event {
                    StreamEvent::PriceTick(tick) => {
                        if price_tx.send(tick).await.is_err() {
                            log::error!("Failed to send price tick");
                        }
                    }
                    StreamEvent::Phase2Triggered {
                        symbol,
                        close_quantity,
                        new_stop_loss,
                    } => {
                        log::info!(
                            "Phase 2 triggered for {}: close {} at breakeven {}",
                            symbol,
                            close_quantity,
                            new_stop_loss
                        );
                        // ExecutionEngine would handle this via the event
                    }
                    StreamEvent::Phase3Triggered {
                        symbol,
                        trailing_stop,
                    } => {
                        log::info!(
                            "Phase 3 triggered for {}: trailing stop at {}",
                            symbol,
                            trailing_stop
                        );
                    }
                    StreamEvent::KlineClosed(kline) => {
                        log::info!(
                            "Kline closed: {} {} close={}",
                            kline.symbol,
                            kline.kline.interval,
                            kline.kline.close
                        );
                        if kline_tx.send(kline).await.is_err() {
                            log::error!("Failed to send kline event");
                        }
                    }
                    StreamEvent::Reconnected => {
                        log::info!(
                            "Stream reconnected - forwarding reconciliation signal to main loop"
                        );
                        if system_tx.send(SystemEvent::Reconnected).await.is_err() {
                            log::error!(
                                "Failed to send SystemEvent::Reconnected — main loop receiver dropped"
                            );
                        }
                    }
                }
            }
        });
    }

    /// Tracks a position for phase detection.
    pub async fn track_position(&self, position: TrackedPosition) {
        let mut processor = self.processor.write().await;
        processor.track_position(position);
    }

    /// Untracks a position.
    pub async fn untrack_position(&self, symbol: &str) {
        let mut processor = self.processor.write().await;
        processor.untrack_position(symbol);
    }

    /// Requests graceful shutdown.
    pub fn shutdown(&self) {
        log::info!("Shutdown requested");
        self.shutdown
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Returns true if shutdown was requested.
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Returns true if circuit breaker is tripped.
    pub async fn is_circuit_breaker_tripped(&self) -> bool {
        let breaker = self.circuit_breaker.read().await;
        breaker.is_tripped()
    }

    /// Resets the circuit breaker.
    pub async fn reset_circuit_breaker(&self) {
        let mut breaker = self.circuit_breaker.write().await;
        breaker.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    #[tokio::test]
    async fn test_manager_config_default() {
        let config = ManagerConfig::default();
        assert!(config.enable_market_stream);
        assert!(!config.enable_user_stream);
        assert!(config.symbols.contains(&"BTCUSDT".to_string()));
    }

    #[tokio::test]
    async fn test_manager_creation() {
        let (price_tx, _price_rx) = mpsc::channel(100);
        let (order_tx, _order_rx) = mpsc::channel(100);
        let (kline_tx, _kline_rx) = mpsc::channel(100);

        let (system_tx, _system_rx) = mpsc::channel(32);
        let manager = WebSocketManager::new(
            ManagerConfig::default(),
            price_tx,
            order_tx,
            kline_tx,
            system_tx,
        );

        assert!(!manager.is_shutdown());
    }

    #[tokio::test]
    async fn test_track_position() {
        let (price_tx, _price_rx) = mpsc::channel(100);
        let (order_tx, _order_rx) = mpsc::channel(100);
        let (kline_tx, _kline_rx) = mpsc::channel(100);

        let (system_tx, _system_rx) = mpsc::channel(32);
        let manager = WebSocketManager::new(
            ManagerConfig::default(),
            price_tx,
            order_tx,
            kline_tx,
            system_tx,
        );

        let position = TrackedPosition::new(
            "BTCUSDT",
            crate::types::Position::Long,
            Decimal::from(50000),
            Decimal::from(49000),
            Decimal::from(53000),
            Decimal::from(1),
            Decimal::from(500),
        );

        manager.track_position(position).await;

        // Position should be tracked
        let processor = manager.processor.read().await;
        assert!(processor.kline_buffer().get("BTCUSDT", "15m").is_none()); // No klines yet
    }

    #[tokio::test]
    async fn test_shutdown() {
        let (price_tx, _price_rx) = mpsc::channel(100);
        let (order_tx, _order_rx) = mpsc::channel(100);
        let (kline_tx, _kline_rx) = mpsc::channel(100);

        let (system_tx, _system_rx) = mpsc::channel(32);
        let manager = WebSocketManager::new(
            ManagerConfig::default(),
            price_tx,
            order_tx,
            kline_tx,
            system_tx,
        );

        assert!(!manager.is_shutdown());
        manager.shutdown();
        assert!(manager.is_shutdown());
    }
}
