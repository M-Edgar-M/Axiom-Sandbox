//! IbkrExecutionEngine — scaffold for Interactive Brokers TradFi execution.
//!
//! This engine will connect to the IBKR TWS/Gateway API to trade regulated
//! instruments such as Crude Oil (WTI) futures.  Unlike the 24/7 Binance
//! engine, TradFi markets close on weekends, so this engine MUST flatten all
//! open positions before Friday's market close to avoid weekend gap risk.
//!
//! ## Current status
//! This is a **mock scaffold**.  All methods log their intent and manipulate
//! in-memory state only — no real IBKR API calls are issued yet.
//!
//! ## Planned integration points
//! - TWS / IB Gateway connection via the `ibapi` crate (or raw EClient socket)
//! - Contract details lookup (conid resolution for CL futures)
//! - Order placement: MKT, STP, LMT order types
//! - `check_market_hours()` → schedule-aware position flattening every Friday

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Datelike, Timelike, Utc, Weekday};
use log::{info, warn};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tokio::sync::RwLock;

use crate::types::{Position, Price, Volume};

use super::orders::ExecutionError;
use super::trade_manager::{ManagedPosition, ManagementAction, TradeState, evaluate_position};

// ─── Configuration ────────────────────────────────────────────────────────────

/// Configuration for the IbkrExecutionEngine.
#[derive(Debug, Clone)]
pub struct IbkrEngineConfig {
    /// TWS / IB Gateway host (e.g., "127.0.0.1").
    pub host: String,
    /// TWS / IB Gateway port (7497 = paper, 7496 = live).
    pub port: u16,
    /// Client ID used to tag this connection inside TWS.
    pub client_id: i32,
    /// Whether to operate in paper-trading mode.
    pub paper_trading: bool,
    /// Instruments this engine is permitted to trade (e.g., ["CL", "BRN"]).
    pub symbols: Vec<String>,
    /// Maximum slippage tolerated on entry/exit (fractional, e.g. 0.001 = 0.1%).
    pub max_slippage: Decimal,
    /// Local time (UTC hour) at which to check for end-of-week flattening.
    /// NYMEX WTI (CL) closes at 17:00 ET on Friday ≈ 22:00 UTC.
    pub friday_close_utc_hour: u32,
}

impl Default for IbkrEngineConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 7497, // paper trading
            client_id: 1,
            paper_trading: true,
            symbols: vec!["CL".to_string()], // WTI Crude Oil front-month
            max_slippage: dec!(0.001),
            friday_close_utc_hour: 22, // 17:00 ET ≈ 22:00 UTC
        }
    }
}

// ─── IbkrExecutionEngine ─────────────────────────────────────────────────────

/// Execution engine for Interactive Brokers TradFi instruments.
///
/// Trade lifecycle mirrors the Binance `ExecutionEngine`:
/// 1. **open_position**   — MKT entry + protective STP + take-profit LMT
/// 2. **monitor_position** — evaluate R-multiple milestones, manage in-trade
/// 3. **close_position**  — flatten the position with a MKT order
///
/// Additionally, **check_market_hours()** must be called on every candle close
/// to detect the Friday pre-close window and flatten any remaining positions
/// before the weekend gap.
pub struct IbkrExecutionEngine {
    /// Engine configuration.
    config: IbkrEngineConfig,
    /// Active managed positions, keyed by symbol.
    positions: Arc<RwLock<HashMap<String, ManagedPosition>>>,
    /// Mock mode: all operations are logged but not sent to IBKR.
    mock_mode: bool,
}

impl IbkrExecutionEngine {
    // ── Constructors ─────────────────────────────────────────────────────────

    /// Creates a live `IbkrExecutionEngine`.
    ///
    /// In the future this will open a socket connection to TWS / IB Gateway
    /// and verify that the requested instruments are tradable.
    pub async fn new(config: IbkrEngineConfig) -> Self {
        // TODO: establish EClient socket connection to config.host:config.port
        // TODO: call reqContractDetails for each symbol in config.symbols
        info!(
            "[IBKR] Engine initialised (host={}:{} paper={})",
            config.host, config.port, config.paper_trading
        );

        Self {
            config,
            positions: Arc::new(RwLock::new(HashMap::new())),
            mock_mode: false,
        }
    }

    /// Creates a **mock** `IbkrExecutionEngine` — no network calls, safe for
    /// unit tests and dry-run simulations.
    pub fn new_mock() -> Self {
        info!("[IBKR] Mock engine initialised");
        Self {
            config: IbkrEngineConfig::default(),
            positions: Arc::new(RwLock::new(HashMap::new())),
            mock_mode: true,
        }
    }

    // ── Market-Hours Guard ───────────────────────────────────────────────────

    /// Returns `true` if it is currently safe to hold or open new positions.
    ///
    /// Returns `false` when the engine detects the Friday pre-close window,
    /// signalling that all open positions should be flattened immediately to
    /// avoid weekend gap risk.
    ///
    /// # Friday-close logic (TODO — implement fully)
    /// NYMEX WTI Crude Oil (CL) trades Sunday–Friday with a daily maintenance
    /// break.  The last tradeable moment each week is **Friday 17:00 ET**
    /// (≈ 22:00 UTC in winter, 21:00 UTC in summer due to DST).
    ///
    /// Full implementation should:
    /// 1. Convert UTC now → US/Eastern using a DST-aware timezone library.
    /// 2. Return `false` if `weekday == Friday && time >= 16:45 ET` to give a
    ///    15-minute buffer to execute the flatten order before hard close.
    /// 3. Return `false` all day Saturday and until Sunday 18:00 ET (re-open).
    /// 4. Optionally query the IBKR `reqMarketDataType` / `isMarketOpen` API
    ///    to get the authoritative exchange status rather than relying on a
    ///    hardcoded schedule.
    pub fn check_market_hours(&self) -> bool {
        let now = Utc::now();
        let weekday = now.weekday();
        let hour = now.hour();

        // Weekend: Saturday is always closed.
        if weekday == Weekday::Sat {
            warn!("[IBKR] Market closed — Saturday. No new positions allowed.");
            return false;
        }

        // Sunday: closed until ~18:00 ET (23:00 UTC winter).
        // TODO: use a proper DST-aware conversion instead of a fixed UTC offset.
        if weekday == Weekday::Sun && hour < 23 {
            warn!(
                "[IBKR] Market closed — Sunday pre-open (UTC hour={}).",
                hour
            );
            return false;
        }

        // Friday close — flatten all positions before `friday_close_utc_hour`.
        // TODO: replace fixed UTC offset with US/Eastern DST-aware conversion.
        if weekday == Weekday::Fri && hour >= self.config.friday_close_utc_hour {
            warn!(
                "[IBKR] Friday pre-close window reached (UTC hour={}). \
                 Positions must be flattened to avoid weekend gap risk.",
                hour
            );
            return false;
        }

        true
    }

    // ── Entry ────────────────────────────────────────────────────────────────

    /// Opens a new position on the configured IBKR instrument.
    ///
    /// Signature mirrors `ExecutionEngine::open_position` so that strategy
    /// logic can call either engine through a shared trait in the future.
    ///
    /// Steps (mock — real implementation pending):
    /// 1. Verify `check_market_hours()` → refuse entry outside trading hours.
    /// 2. Place MKT order for `quantity` in `direction`.
    /// 3. Place protective STP at `stop_loss`.
    /// 4. Place take-profit LMT at `take_profit`.
    pub async fn open_position(
        &self,
        symbol: &str,
        direction: Position,
        entry_price: Price,
        stop_loss: Price,
        take_profit: Price,
        quantity: Volume,
        atr_value: Decimal,
    ) -> Result<ManagedPosition, ExecutionError> {
        // Guard: never open a position outside market hours.
        if !self.check_market_hours() {
            return Err(ExecutionError::ExchangeError {
                message: format!(
                    "[IBKR] Refused to open {} position on {} — market is closed or \
                     in Friday pre-close window.",
                    symbol,
                    match direction {
                        Position::Long => "LONG",
                        Position::Short => "SHORT",
                        Position::None => "NONE",
                    }
                ),
            });
        }

        let position = ManagedPosition::new(
            symbol,
            direction,
            entry_price,
            stop_loss,
            take_profit,
            quantity,
            atr_value,
        );

        if self.mock_mode {
            info!(
                "[IBKR MOCK] open_position: symbol={} direction={:?} \
                 entry={} sl={} tp={} qty={}",
                symbol, direction, entry_price, stop_loss, take_profit, quantity
            );
            let mut positions = self.positions.write().await;
            positions.insert(symbol.to_string(), position.clone());
            return Ok(position);
        }

        // TODO: translate `direction` → IBKR Action::Buy / Action::Sell
        // TODO: place MKT order via EClient::place_order
        // TODO: place STP and LMT bracket orders
        // TODO: store IBKR orderId on the ManagedPosition

        let mut positions = self.positions.write().await;
        positions.insert(symbol.to_string(), position.clone());
        Ok(position)
    }

    // ── Monitor ──────────────────────────────────────────────────────────────

    /// Evaluates the active position for `symbol` at `current_price` and
    /// executes the appropriate in-trade management action.
    ///
    /// Also checks `check_market_hours()` — if the Friday close window is
    /// reached, it overrides the normal state-machine and forces a full
    /// position close to avoid weekend gap risk.
    pub async fn monitor_position(
        &self,
        symbol: &str,
        current_price: Price,
    ) -> Result<ManagementAction, ExecutionError> {
        // Friday close override: flatten everything.
        if !self.check_market_hours() {
            warn!(
                "[IBKR] Friday close override — forcing close of {} position.",
                symbol
            );
            self.close_position(symbol).await?;
            return Ok(ManagementAction::ClosePosition {
                reason: "Friday pre-close: weekend gap risk mitigation".to_string(),
            });
        }

        let positions = self.positions.read().await;
        let position = positions
            .get(symbol)
            .ok_or(ExecutionError::PositionNotFound {
                symbol: symbol.to_string(),
            })?;

        let action = evaluate_position(position, current_price);
        drop(positions);

        match &action {
            ManagementAction::ExecuteFirstTp {
                quantity_to_close,
                new_stop_loss,
            } => {
                info!(
                    "[IBKR] First TP hit for {} — closing {:.4} qty, moving SL to {}",
                    symbol, quantity_to_close, new_stop_loss
                );
                self.execute_first_tp(symbol, *quantity_to_close, *new_stop_loss)
                    .await?;
            }
            ManagementAction::ActivateTrailingStop { offset } => {
                info!(
                    "[IBKR] Trailing stop activation for {} — offset={}",
                    symbol, offset
                );
                self.activate_trailing_stop(symbol, *offset).await?;
            }
            ManagementAction::ClosePosition { reason } => {
                info!("[IBKR] Closing {} — reason: {}", symbol, reason);
                self.close_position(symbol).await?;
            }
            ManagementAction::None => {}
        }

        Ok(action)
    }

    // ── Phase 2: First TP ────────────────────────────────────────────────────

    /// Executes the first take-profit milestone:
    /// - Cancels existing STP / LMT bracket orders.
    /// - MKT-closes `quantity_to_close` (reduce-only).
    /// - Places new STP at breakeven (`new_stop_loss`).
    /// - Places new take-profit LMT for the remainder.
    async fn execute_first_tp(
        &self,
        symbol: &str,
        quantity_to_close: Volume,
        new_stop_loss: Price,
    ) -> Result<(), ExecutionError> {
        let mut positions = self.positions.write().await;
        let position = positions
            .get_mut(symbol)
            .ok_or(ExecutionError::PositionNotFound {
                symbol: symbol.to_string(),
            })?;

        if position.state != TradeState::Open {
            return Ok(()); // Already processed
        }

        if self.mock_mode {
            info!(
                "[IBKR MOCK] execute_first_tp: symbol={} close_qty={} new_sl={}",
                symbol, quantity_to_close, new_stop_loss
            );
        }

        // TODO: cancel bracket orders via EClient::cancel_order
        // TODO: place reduce-only MKT order for quantity_to_close
        // TODO: place new STP at new_stop_loss and new LMT at position.take_profit

        position.transition_to_first_tp(quantity_to_close, Decimal::ZERO);
        position.current_stop_loss = new_stop_loss;

        Ok(())
    }

    // ── Phase 3: Trailing Stop ───────────────────────────────────────────────

    /// Activates a trailing stop at `offset` price units from the current
    /// market price.
    ///
    /// IBKR supports native TRAIL / TRAIL LIMIT orders; this mock simply
    /// records the state transition.
    async fn activate_trailing_stop(
        &self,
        symbol: &str,
        offset: Decimal,
    ) -> Result<(), ExecutionError> {
        let mut positions = self.positions.write().await;
        let position = positions
            .get_mut(symbol)
            .ok_or(ExecutionError::PositionNotFound {
                symbol: symbol.to_string(),
            })?;

        if self.mock_mode {
            info!(
                "[IBKR MOCK] activate_trailing_stop: symbol={} offset={}",
                symbol, offset
            );
        }

        // TODO: cancel existing bracket orders
        // TODO: place IBKR TRAIL order with auxPrice = offset

        // Use a placeholder order ID of 0 in mock mode
        position.transition_to_trailing(0);

        Ok(())
    }

    // ── Close ────────────────────────────────────────────────────────────────

    /// Flattens the position for `symbol` with a MKT order.
    ///
    /// Called both by the normal trade state machine and by the Friday
    /// pre-close override in `monitor_position`.
    pub async fn close_position(&self, symbol: &str) -> Result<(), ExecutionError> {
        let mut positions = self.positions.write().await;
        let position = positions
            .get_mut(symbol)
            .ok_or(ExecutionError::PositionNotFound {
                symbol: symbol.to_string(),
            })?;

        if self.mock_mode {
            info!(
                "[IBKR MOCK] close_position: symbol={} direction={:?} qty={}",
                symbol, position.direction, position.current_quantity
            );
        }

        // TODO: cancel all open orders for symbol via EClient::req_global_cancel
        // TODO: place MKT order in opposite direction with TIF=DAY

        position.close(Decimal::ZERO);

        Ok(())
    }

    // ── Accessors ────────────────────────────────────────────────────────────

    /// Returns a snapshot of all currently active positions.
    pub async fn active_positions(&self) -> HashMap<String, ManagedPosition> {
        self.positions
            .read()
            .await
            .iter()
            .filter(|(_, p)| p.is_active())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Returns `true` if there is an active position for the given symbol.
    pub async fn has_position(&self, symbol: &str) -> bool {
        self.positions
            .read()
            .await
            .get(symbol)
            .map(|p| p.is_active())
            .unwrap_or(false)
    }
}
