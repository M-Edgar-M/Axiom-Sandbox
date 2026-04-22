//! Market and User data stream handlers.

use std::collections::HashMap;

use rust_decimal::Decimal;
use tokio::sync::mpsc;

use super::config;
use crate::types::Position;

use super::messages::{KlineEvent, PriceTick, WsMessage};

/// Stream type for different WebSocket connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamType {
    /// Market data stream (AggTrade, Klines).
    Market,
    /// User data stream (Account, Orders).
    UserData,
}

impl StreamType {
    /// Returns the heartbeat timeout for this stream type.
    pub fn heartbeat_timeout(&self) -> std::time::Duration {
        match self {
            StreamType::Market => config::MARKET_HEARTBEAT_TIMEOUT,
            StreamType::UserData => config::USER_HEARTBEAT_TIMEOUT,
        }
    }

    /// Returns the activity check interval for this stream type.
    pub fn activity_check_interval(&self) -> std::time::Duration {
        match self {
            StreamType::Market => config::MARKET_ACTIVITY_CHECK,
            StreamType::UserData => std::time::Duration::from_secs(60), // 1 minute for user
        }
    }
}

/// Builds WebSocket URL for combined market streams (USD-M Futures).
pub fn build_market_stream_url(symbols: &[&str], intervals: &[&str]) -> String {
    let mut streams = Vec::new();

    for symbol in symbols {
        let lower = symbol.to_lowercase();

        // Add aggTrade stream
        streams.push(format!("{}@aggTrade", lower));

        // Add kline streams for each interval
        for interval in intervals {
            streams.push(format!("{}@kline_{}", lower, interval));
        }
    }

    format!(
        "{}/stream?streams={}",
        config::BINANCE_FUTURES_WS_BASE,
        streams.join("/")
    )
}

/// Builds WebSocket URL for user data stream (USD-M Futures).
pub fn build_user_stream_url(listen_key: &str) -> String {
    format!("{}/ws/{}", config::BINANCE_FUTURES_WS_BASE, listen_key)
}

/// Kline buffer for indicator backfill.
#[derive(Debug, Clone)]
pub struct KlineBuffer {
    /// Buffered klines by symbol and interval.
    klines: HashMap<(String, String), Vec<KlineEvent>>,
    /// Maximum klines to keep per symbol/interval.
    max_size: usize,
}

impl KlineBuffer {
    /// Creates a new kline buffer.
    pub fn new(max_size: usize) -> Self {
        Self {
            klines: HashMap::new(),
            max_size,
        }
    }

    /// Adds a kline to the buffer.
    pub fn push(&mut self, kline: KlineEvent) {
        let key = (kline.symbol.clone(), kline.kline.interval.clone());
        let buffer = self.klines.entry(key).or_insert_with(Vec::new);

        buffer.push(kline);

        // Keep only most recent
        if buffer.len() > self.max_size {
            buffer.remove(0);
        }
    }

    /// Gets klines for a symbol and interval.
    pub fn get(&self, symbol: &str, interval: &str) -> Option<&Vec<KlineEvent>> {
        self.klines.get(&(symbol.to_string(), interval.to_string()))
    }

    /// Clears all buffered klines.
    pub fn clear(&mut self) {
        self.klines.clear();
    }
}

/// Position tracking for phase detection.
#[derive(Debug, Clone)]
pub struct TrackedPosition {
    /// Symbol.
    pub symbol: String,
    /// Position direction (Long or Short).
    pub direction: Position,
    /// Entry price.
    pub entry_price: Decimal,
    /// Stop loss price.
    pub stop_loss: Decimal,
    /// Take profit price.
    pub take_profit: Decimal,
    /// Position size.
    pub size: Decimal,
    /// Risk per unit (|entry - stop loss|).
    pub risk_per_unit: Decimal,
    /// ATR value for trailing stop.
    pub atr_value: Decimal,
    /// Current phase.
    pub phase: TradePhase,
}

/// Trade phase for in-trade management.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradePhase {
    /// Initial phase.
    Phase1,
    /// After 1.5R: SL at breakeven.
    Phase2,
    /// After 2.5R: Trailing stop active.
    Phase3,
}

impl TrackedPosition {
    /// Creates a new tracked position.
    pub fn new(
        symbol: impl Into<String>,
        direction: Position,
        entry_price: Decimal,
        stop_loss: Decimal,
        take_profit: Decimal,
        size: Decimal,
        atr_value: Decimal,
    ) -> Self {
        let risk_per_unit = (entry_price - stop_loss).abs();

        Self {
            symbol: symbol.into(),
            direction,
            entry_price,
            stop_loss,
            take_profit,
            size,
            risk_per_unit,
            atr_value,
            phase: TradePhase::Phase1,
        }
    }

    /// Calculates current R-multiple based on price.
    pub fn current_r_multiple(&self, current_price: Decimal) -> Decimal {
        if self.risk_per_unit == Decimal::ZERO {
            return Decimal::ZERO;
        }

        let pnl = match self.direction {
            Position::Long => current_price - self.entry_price,
            Position::Short => self.entry_price - current_price,
            Position::None => Decimal::ZERO,
        };

        pnl / self.risk_per_unit
    }

    /// Checks if price has reached 1.5R (Phase 2 trigger).
    pub fn should_trigger_phase2(&self, current_price: Decimal) -> bool {
        self.phase == TradePhase::Phase1
            && self.current_r_multiple(current_price) >= Decimal::from_str_exact("1.5").unwrap()
    }

    /// Checks if price has reached 2.5R (Phase 3 trigger).
    pub fn should_trigger_phase3(&self, current_price: Decimal) -> bool {
        self.phase != TradePhase::Phase3
            && self.current_r_multiple(current_price) >= Decimal::from_str_exact("2.5").unwrap()
    }

    /// Transitions to Phase 2: SL to breakeven.
    pub fn transition_phase2(&mut self) {
        self.phase = TradePhase::Phase2;
        self.stop_loss = self.entry_price;
    }

    /// Transitions to Phase 3: trailing stop active.
    pub fn transition_phase3(&mut self) {
        self.phase = TradePhase::Phase3;
    }

    /// Calculates trailing stop price.
    pub fn trailing_stop_price(&self, current_price: Decimal) -> Decimal {
        let offset = self.atr_value * Decimal::from(2); // 2.0x ATR
        match self.direction {
            Position::Long => current_price - offset,
            Position::Short => current_price + offset,
            Position::None => current_price,
        }
    }

    /// Calculates quantity to close at Phase 2 (33%).
    pub fn phase2_close_quantity(&self) -> Decimal {
        self.size * Decimal::from_str_exact("0.33").unwrap()
    }
}

/// Events emitted by stream processor.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Price tick received.
    PriceTick(PriceTick),
    /// Phase 2 triggered: move SL to breakeven, close 33%.
    Phase2Triggered {
        symbol: String,
        close_quantity: Decimal,
        new_stop_loss: Decimal,
    },
    /// Phase 3 triggered: activate trailing stop.
    Phase3Triggered {
        symbol: String,
        trailing_stop: Decimal,
    },
    /// Kline closed (for indicator updates).
    KlineClosed(KlineEvent),
    /// Reconnection occurred (trigger state sync).
    Reconnected,
}

/// Stream processor that monitors prices and triggers phase transitions.
pub struct StreamProcessor {
    /// Tracked positions.
    positions: HashMap<String, TrackedPosition>,
    /// Event sender.
    event_tx: mpsc::Sender<StreamEvent>,
    /// Kline buffer.
    kline_buffer: KlineBuffer,
}

impl StreamProcessor {
    /// Creates a new stream processor.
    pub fn new(event_tx: mpsc::Sender<StreamEvent>) -> Self {
        Self {
            positions: HashMap::new(),
            event_tx,
            kline_buffer: KlineBuffer::new(config::BACKFILL_KLINE_COUNT as usize),
        }
    }

    /// Adds a position to track.
    pub fn track_position(&mut self, position: TrackedPosition) {
        self.positions.insert(position.symbol.clone(), position);
    }

    /// Removes a position from tracking.
    pub fn untrack_position(&mut self, symbol: &str) {
        self.positions.remove(symbol);
    }

    /// Processes a WebSocket message.
    pub async fn process_message(
        &mut self,
        msg: WsMessage,
    ) -> Result<(), mpsc::error::SendError<StreamEvent>> {
        match msg {
            WsMessage::AggTrade(trade) => {
                if let Some(price) = trade.price_decimal() {
                    // Emit price tick
                    let tick = PriceTick {
                        symbol: trade.symbol.clone(),
                        price,
                        timestamp: trade.trade_time,
                    };
                    self.event_tx.send(StreamEvent::PriceTick(tick)).await?;

                    // Check phase transitions
                    self.check_phase_transitions(&trade.symbol, price).await?;
                }
            }
            WsMessage::Kline(kline) => {
                // Buffer kline
                self.kline_buffer.push(kline.clone());

                // If closed, emit event
                if kline.kline.is_closed {
                    self.event_tx.send(StreamEvent::KlineClosed(kline)).await?;
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Checks and triggers phase transitions for a symbol.
    async fn check_phase_transitions(
        &mut self,
        symbol: &str,
        current_price: Decimal,
    ) -> Result<(), mpsc::error::SendError<StreamEvent>> {
        if let Some(position) = self.positions.get_mut(symbol) {
            // Check Phase 3 first (higher priority)
            if position.should_trigger_phase3(current_price) {
                let trailing_stop = position.trailing_stop_price(current_price);
                position.transition_phase3();

                self.event_tx
                    .send(StreamEvent::Phase3Triggered {
                        symbol: symbol.to_string(),
                        trailing_stop,
                    })
                    .await?;
            }
            // Check Phase 2
            else if position.should_trigger_phase2(current_price) {
                let close_quantity = position.phase2_close_quantity();
                position.transition_phase2();

                self.event_tx
                    .send(StreamEvent::Phase2Triggered {
                        symbol: symbol.to_string(),
                        close_quantity,
                        new_stop_loss: position.entry_price,
                    })
                    .await?;
            }
        }

        Ok(())
    }

    /// Returns kline buffer for backfill.
    pub fn kline_buffer(&self) -> &KlineBuffer {
        &self.kline_buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_build_market_stream_url() {
        let url = build_market_stream_url(&["BTCUSDT"], &["15m", "1h"]);
        assert!(url.starts_with("wss://fstream.binance.com"));
        assert!(url.contains("btcusdt@aggTrade"));
        assert!(url.contains("btcusdt@kline_15m"));
        assert!(url.contains("btcusdt@kline_1h"));
    }

    #[test]
    fn test_tracked_position_r_multiple() {
        let pos = TrackedPosition::new(
            "BTCUSDT",
            Position::Long,
            dec!(50000),
            dec!(49000),
            dec!(53000),
            dec!(1),
            dec!(500),
        );

        // At entry: 0R
        assert_eq!(pos.current_r_multiple(dec!(50000)), dec!(0));

        // At 1.5R
        assert_eq!(pos.current_r_multiple(dec!(51500)), dec!(1.5));

        // At 2.5R
        assert_eq!(pos.current_r_multiple(dec!(52500)), dec!(2.5));
    }

    #[test]
    fn test_phase_triggers() {
        let pos = TrackedPosition::new(
            "BTCUSDT",
            Position::Long,
            dec!(50000),
            dec!(49000),
            dec!(53000),
            dec!(1),
            dec!(500),
        );

        assert!(!pos.should_trigger_phase2(dec!(50000)));
        assert!(pos.should_trigger_phase2(dec!(51500)));

        assert!(!pos.should_trigger_phase3(dec!(51500)));
        assert!(pos.should_trigger_phase3(dec!(52500)));
    }

    #[test]
    fn test_phase_transitions() {
        let mut pos = TrackedPosition::new(
            "BTCUSDT",
            Position::Long,
            dec!(50000),
            dec!(49000),
            dec!(53000),
            dec!(1),
            dec!(500),
        );

        assert_eq!(pos.phase, TradePhase::Phase1);

        pos.transition_phase2();
        assert_eq!(pos.phase, TradePhase::Phase2);
        assert_eq!(pos.stop_loss, dec!(50000)); // Breakeven

        pos.transition_phase3();
        assert_eq!(pos.phase, TradePhase::Phase3);
    }

    #[test]
    fn test_trailing_stop_calculation() {
        let pos = TrackedPosition::new(
            "BTCUSDT",
            Position::Long,
            dec!(50000),
            dec!(49000),
            dec!(53000),
            dec!(1),
            dec!(500),
        );

        // Trailing stop = current - (2.0 * ATR) = 52000 - 1000 = 51000
        assert_eq!(pos.trailing_stop_price(dec!(52000)), dec!(51000));
    }

    #[test]
    fn test_kline_buffer() {
        let mut buffer = KlineBuffer::new(3);

        // Create mock kline events
        let kline1 = create_mock_kline("BTCUSDT", "15m", 1);
        let kline2 = create_mock_kline("BTCUSDT", "15m", 2);
        let kline3 = create_mock_kline("BTCUSDT", "15m", 3);
        let kline4 = create_mock_kline("BTCUSDT", "15m", 4);

        buffer.push(kline1);
        buffer.push(kline2);
        buffer.push(kline3);

        let klines = buffer.get("BTCUSDT", "15m").unwrap();
        assert_eq!(klines.len(), 3);

        // Adding 4th should remove oldest
        buffer.push(kline4);
        let klines = buffer.get("BTCUSDT", "15m").unwrap();
        assert_eq!(klines.len(), 3);
    }

    fn create_mock_kline(symbol: &str, interval: &str, idx: u64) -> KlineEvent {
        KlineEvent {
            event_type: "kline".to_string(),
            event_time: idx,
            symbol: symbol.to_string(),
            kline: super::super::messages::KlineData {
                start_time: idx,
                close_time: idx + 1,
                symbol: symbol.to_string(),
                interval: interval.to_string(),
                open: "50000".to_string(),
                close: "50100".to_string(),
                high: "50200".to_string(),
                low: "49900".to_string(),
                volume: "100".to_string(),
                num_trades: 1000,
                is_closed: true,
                quote_volume: "5000000".to_string(),
            },
        }
    }

    // ── Short position tests ──

    fn create_short_position() -> TrackedPosition {
        // Short BTCUSDT: entry=50000, SL=51000 (above), TP=47000 (below)
        // risk_per_unit = |50000 - 51000| = 1000
        TrackedPosition::new(
            "BTCUSDT",
            Position::Short,
            dec!(50000),
            dec!(51000),
            dec!(47000),
            dec!(1),
            dec!(500),
        )
    }

    #[test]
    fn test_short_r_multiple_calculation() {
        let pos = create_short_position();

        // At entry: 0R
        assert_eq!(pos.current_r_multiple(dec!(50000)), dec!(0));

        // Price drops 1500 (favorable) => +1.5R
        assert_eq!(pos.current_r_multiple(dec!(48500)), dec!(1.5));

        // Price drops 2500 (favorable) => +2.5R
        assert_eq!(pos.current_r_multiple(dec!(47500)), dec!(2.5));

        // Price rises 1000 (adverse) => -1R
        assert_eq!(pos.current_r_multiple(dec!(51000)), dec!(-1));
    }

    #[test]
    fn test_short_phase_triggers() {
        let pos = create_short_position();

        // At entry — no trigger
        assert!(!pos.should_trigger_phase2(dec!(50000)));

        // Price drops to 1.5R (48500) — Phase 2 should trigger
        assert!(pos.should_trigger_phase2(dec!(48500)));

        // Price drops to 2.5R (47500) — Phase 3 should trigger
        assert!(pos.should_trigger_phase3(dec!(47500)));

        // Price rises — should NOT trigger
        assert!(!pos.should_trigger_phase2(dec!(51000)));
        assert!(!pos.should_trigger_phase3(dec!(51000)));
    }

    #[test]
    fn test_short_phase_transitions() {
        let mut pos = create_short_position();

        assert_eq!(pos.phase, TradePhase::Phase1);

        pos.transition_phase2();
        assert_eq!(pos.phase, TradePhase::Phase2);
        assert_eq!(pos.stop_loss, dec!(50000)); // Breakeven = entry price

        pos.transition_phase3();
        assert_eq!(pos.phase, TradePhase::Phase3);
    }

    #[test]
    fn test_short_trailing_stop_calculation() {
        let pos = create_short_position();

        // Short trailing stop = current_price + (2.0 * ATR)
        // = 47500 + (2 * 500) = 47500 + 1000 = 48500
        assert_eq!(pos.trailing_stop_price(dec!(47500)), dec!(48500));
    }
}
