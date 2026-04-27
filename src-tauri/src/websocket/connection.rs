//! WebSocket connection handler with lifecycle management.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use rand::RngExt;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use url::Url;

use super::config;
use super::messages::WsMessage;

/// WebSocket connection errors.
#[derive(Debug, Error)]
pub enum ConnectionError {
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("Connection timeout")]
    Timeout,

    #[error("Heartbeat timeout")]
    HeartbeatTimeout,

    #[error("Channel send error")]
    ChannelSend,

    #[error("Circuit breaker triggered")]
    CircuitBreaker,

    #[error("Connection closed")]
    Closed,
}

/// Connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Rotating,
}

#[derive(Clone)]
pub struct WsConnection {
    /// Connection URL.
    url: Url,
    /// Current state.
    state: ConnectionState,
    /// Last message received time.
    last_message_time: Instant,
    /// Connection start time (for rotation).
    connection_start: Option<Instant>,
    /// Current backoff attempt.
    backoff_attempt: u32,
    /// Heartbeat timeout duration.
    heartbeat_timeout: Duration,
    /// Shutdown signal.
    shutdown: Arc<AtomicBool>,
}

impl WsConnection {
    /// Creates a new connection handler.
    pub fn new(url: &str, heartbeat_timeout: Duration) -> Result<Self, ConnectionError> {
        Ok(Self {
            url: Url::parse(url)?,
            state: ConnectionState::Disconnected,
            last_message_time: Instant::now(),
            connection_start: None,
            backoff_attempt: 0,
            heartbeat_timeout,
            shutdown: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Returns current connection state.
    pub fn state(&self) -> ConnectionState {
        self.state
    }

    /// Returns a shutdown signal handle.
    pub fn shutdown_signal(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown)
    }

    /// Triggers shutdown.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Returns true if shutdown was requested.
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// Connects to the WebSocket server.
    pub async fn connect(
        &mut self,
    ) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, ConnectionError> {
        self.state = ConnectionState::Connecting;

        let (ws_stream, _response) = connect_async(self.url.as_str()).await?;

        self.state = ConnectionState::Connected;
        self.connection_start = Some(Instant::now());
        self.last_message_time = Instant::now();
        self.backoff_attempt = 0;

        Ok(ws_stream)
    }

    /// Calculates backoff delay with jitter.
    pub fn calculate_backoff(&self) -> Duration {
        let base = config::INITIAL_BACKOFF.as_millis() as u64;
        let multiplier = config::BACKOFF_MULTIPLIER.pow(self.backoff_attempt);
        let delay_ms = base.saturating_mul(multiplier as u64);
        let capped_delay_ms = delay_ms.min(config::MAX_BACKOFF.as_millis() as u64);

        // Add jitter
        let jitter: u64 = rand::rng().random_range(0..config::MAX_JITTER_MS);
        Duration::from_millis(capped_delay_ms + jitter)
    }

    /// Increments backoff attempt counter.
    pub fn increment_backoff(&mut self) {
        self.backoff_attempt = self.backoff_attempt.saturating_add(1);
    }

    /// Resets backoff counter (after successful connection).
    pub fn reset_backoff(&mut self) {
        self.backoff_attempt = 0;
    }

    /// Checks if proactive rotation is needed.
    pub fn needs_rotation(&self) -> bool {
        if let Some(start) = self.connection_start {
            start.elapsed() >= config::ROTATION_INTERVAL
        } else {
            false
        }
    }

    /// Checks if heartbeat timeout has occurred.
    pub fn heartbeat_timeout(&self) -> bool {
        self.last_message_time.elapsed() >= self.heartbeat_timeout
    }

    /// Updates last message time.
    pub fn touch(&mut self) {
        self.last_message_time = Instant::now();
    }

    /// Sets state to reconnecting.
    pub fn set_reconnecting(&mut self) {
        self.state = ConnectionState::Reconnecting;
    }

    /// Sets state to rotating.
    pub fn set_rotating(&mut self) {
        self.state = ConnectionState::Rotating;
    }

    /// Sets state to disconnected.
    pub fn set_disconnected(&mut self) {
        self.state = ConnectionState::Disconnected;
        self.connection_start = None;
    }
}

/// Handles a single WebSocket connection with message processing.
pub async fn run_connection(
    mut connection: WsConnection,
    message_tx: mpsc::Sender<WsMessage>,
    activity_check_interval: Duration,
) -> Result<(), ConnectionError> {
    loop {
        if connection.is_shutdown() {
            return Ok(());
        }

        // Connect with backoff
        let ws_stream = match connection.connect().await {
            Ok(stream) => stream,
            Err(e) => {
                log::warn!("Connection failed: {}, retrying...", e);
                connection.increment_backoff();
                let delay = connection.calculate_backoff();
                tokio::time::sleep(delay).await;
                continue;
            }
        };

        log::info!("WebSocket connected to {}", connection.url);

        // Split stream
        let (write, read) = ws_stream.split();

        // Run message loop
        let result = message_loop(
            &mut connection,
            write,
            read,
            message_tx.clone(),
            activity_check_interval,
        )
        .await;

        match result {
            Ok(()) => {
                // Clean disconnect (rotation)
                log::info!("Connection closed cleanly");
                connection.set_disconnected();
            }
            Err(ConnectionError::HeartbeatTimeout) => {
                log::warn!("Heartbeat timeout, reconnecting...");
                connection.set_reconnecting();
            }
            Err(e) => {
                log::error!("Connection error: {}", e);
                connection.set_reconnecting();
                connection.increment_backoff();
            }
        }

        // Check for rotation
        if connection.needs_rotation() {
            log::info!("Proactive rotation triggered");
            connection.set_rotating();
        }

        // Apply backoff delay
        if connection.state() == ConnectionState::Reconnecting {
            let delay = connection.calculate_backoff();
            log::info!("Reconnecting in {:?}...", delay);
            tokio::time::sleep(delay).await;
        }
    }
}

/// Message processing loop.
async fn message_loop(
    connection: &mut WsConnection,
    mut write: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    mut read: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    message_tx: mpsc::Sender<WsMessage>,
    activity_check_interval: Duration,
) -> Result<(), ConnectionError> {
    let mut activity_check = interval(activity_check_interval);

    loop {
        if connection.is_shutdown() {
            // Send close frame
            let _ = write.send(Message::Close(None)).await;
            return Ok(());
        }

        // Check for rotation
        if connection.needs_rotation() {
            log::info!("Rotation needed, closing connection");
            let _ = write.send(Message::Close(None)).await;
            return Ok(());
        }

        tokio::select! {
            // Receive message
            msg_result = read.next() => {
                match msg_result {
                    Some(Ok(msg)) => {
                        connection.touch();

                        match msg {
                            Message::Text(text) => {
                                let ws_msg = parse_message(&text);
                                if message_tx.send(ws_msg).await.is_err() {
                                    return Err(ConnectionError::ChannelSend);
                                }
                            }
                            Message::Binary(data) => {
                                // Parse binary as text
                                if let Ok(text) = String::from_utf8(data.to_vec()) {
                                    let ws_msg = parse_message(&text);
                                    if message_tx.send(ws_msg).await.is_err() {
                                        return Err(ConnectionError::ChannelSend);
                                    }
                                }
                            }
                            Message::Ping(payload) => {
                                // Respond with Pong using identical payload
                                write.send(Message::Pong(payload.clone())).await?;
                                if message_tx.send(WsMessage::Ping(payload.to_vec())).await.is_err() {
                                    return Err(ConnectionError::ChannelSend);
                                }
                            }
                            Message::Pong(_) => {
                                // Ignore pong frames
                            }
                            Message::Close(_) => {
                                log::info!("Received close frame");
                                if message_tx.send(WsMessage::Closed).await.is_err() {
                                    return Err(ConnectionError::ChannelSend);
                                }
                                return Err(ConnectionError::Closed);
                            }
                            Message::Frame(_) => {
                                // Raw frame, ignore
                            }
                        }
                    }
                    Some(Err(e)) => {
                        return Err(ConnectionError::WebSocket(e));
                    }
                    None => {
                        // Stream ended
                        return Err(ConnectionError::Closed);
                    }
                }
            }

            // Activity check
            _ = activity_check.tick() => {
                if connection.heartbeat_timeout() {
                    return Err(ConnectionError::HeartbeatTimeout);
                }
            }
        }
    }
}

#[derive(serde::Deserialize)]
struct CombinedEvent {
    data: serde_json::Value,
}

/// Parse raw message into WsMessage type.
///
/// Binance combined streams wrap all events in an envelope:
/// `{"stream":"btcusdt@aggTrade","data":{...actual event...}}`
fn parse_message(text: &str) -> WsMessage {
    // Try to unwrap the combined stream envelope first.
    if let Ok(envelope) = serde_json::from_str::<CombinedEvent>(text) {
        return parse_inner_value(envelope.data, text);
    }

    // If not a combined stream envelope, try parsing directly as Value
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        return parse_inner_value(value, text);
    }

    WsMessage::Unknown(text.to_string())
}

/// Parse the inner event JSON into a WsMessage directly from a Value.
fn parse_inner_value(value: serde_json::Value, original: &str) -> WsMessage {
    if let Ok(agg_trade) = serde_json::from_value::<super::messages::AggTrade>(value.clone()) {
        if agg_trade.event_type == "aggTrade" {
            return WsMessage::AggTrade(agg_trade);
        }
    }

    if let Ok(kline) = serde_json::from_value::<super::messages::KlineEvent>(value.clone()) {
        if kline.event_type == "kline" {
            return WsMessage::Kline(kline);
        }
    }

    if let Ok(account) = serde_json::from_value::<super::messages::AccountUpdate>(value.clone()) {
        if account.event_type == "outboundAccountPosition" {
            return WsMessage::Account(account);
        }
    }

    if let Ok(order) = serde_json::from_value::<super::messages::OrderUpdate>(value.clone()) {
        if order.event_type == "executionReport" {
            return WsMessage::Order(order);
        }
    }

    WsMessage::Unknown(original.to_string())
}

/// Circuit breaker for connection failures.
pub struct CircuitBreaker {
    /// Disconnection timestamps within window.
    disconnections: Vec<Instant>,
    /// Maximum disconnections allowed.
    limit: u32,
    /// Time window.
    window: Duration,
    /// Whether circuit is open (tripped).
    is_open: bool,
}

impl CircuitBreaker {
    /// Creates a new circuit breaker.
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            disconnections: Vec::new(),
            limit,
            window,
            is_open: false,
        }
    }

    /// Creates with default configuration.
    pub fn default_config() -> Self {
        Self::new(
            config::CIRCUIT_BREAKER_LIMIT,
            config::CIRCUIT_BREAKER_WINDOW,
        )
    }

    /// Records a disconnection and checks if circuit should trip.
    pub fn record_disconnection(&mut self) -> bool {
        let now = Instant::now();

        // Remove old disconnections outside window
        self.disconnections
            .retain(|&t| now.duration_since(t) < self.window);

        // Add new disconnection
        self.disconnections.push(now);

        // Check if limit exceeded
        if self.disconnections.len() as u32 >= self.limit {
            self.is_open = true;
            log::error!(
                "Circuit breaker triggered: {} disconnections in {:?}",
                self.disconnections.len(),
                self.window
            );
        }

        self.is_open
    }

    /// Returns true if circuit is open (bot should shutdown).
    pub fn is_tripped(&self) -> bool {
        self.is_open
    }

    /// Resets the circuit breaker.
    pub fn reset(&mut self) {
        self.disconnections.clear();
        self.is_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_calculation() {
        let mut conn = WsConnection::new("wss://example.com", Duration::from_secs(60)).unwrap();

        // First attempt: 1s + jitter
        let delay1 = conn.calculate_backoff();
        assert!(delay1 >= Duration::from_secs(1));
        assert!(delay1 < Duration::from_secs(3));

        // Second attempt: 2s + jitter
        conn.increment_backoff();
        let delay2 = conn.calculate_backoff();
        assert!(delay2 >= Duration::from_secs(2));

        // Third attempt: 4s + jitter
        conn.increment_backoff();
        let delay3 = conn.calculate_backoff();
        assert!(delay3 >= Duration::from_secs(4));
    }

    #[test]
    fn test_backoff_max_cap() {
        let mut conn = WsConnection::new("wss://example.com", Duration::from_secs(60)).unwrap();

        // Many attempts should cap at MAX_BACKOFF
        for _ in 0..20 {
            conn.increment_backoff();
        }

        let delay = conn.calculate_backoff();
        assert!(delay <= config::MAX_BACKOFF + Duration::from_millis(config::MAX_JITTER_MS));
    }

    #[test]
    fn test_circuit_breaker() {
        let mut breaker = CircuitBreaker::new(3, Duration::from_secs(60));

        assert!(!breaker.record_disconnection());
        assert!(!breaker.record_disconnection());
        assert!(breaker.record_disconnection()); // 3rd triggers

        assert!(breaker.is_tripped());

        breaker.reset();
        assert!(!breaker.is_tripped());
    }

    #[test]
    fn test_parse_agg_trade_message() {
        let json = r#"{"e":"aggTrade","E":123,"s":"BTCUSDT","a":1,"p":"50000","q":"1","f":1,"l":1,"T":123,"m":true}"#;
        let msg = parse_message(json);

        match msg {
            WsMessage::AggTrade(t) => assert_eq!(t.symbol, "BTCUSDT"),
            _ => panic!("Expected AggTrade"),
        }
    }

    #[test]
    fn test_needs_rotation() {
        let mut conn = WsConnection::new("wss://example.com", Duration::from_secs(60)).unwrap();

        // No connection yet
        assert!(!conn.needs_rotation());

        // Simulate connection
        conn.connection_start = Some(Instant::now() - Duration::from_secs(24 * 60 * 60));
        assert!(conn.needs_rotation());
    }
}
