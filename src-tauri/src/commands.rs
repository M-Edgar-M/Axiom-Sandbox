//! Tauri IPC commands — the React ↔ Rust bridge.
//!
//! Every function here is decorated with `#[tauri::command]` and registered
//! in `lib.rs::run()`.  The React frontend calls them via:
//! ```ts
//! import { invoke } from "@tauri-apps/api/core";
//! const result = await invoke("start_mock_session", { config: { ... } });
//! ```
//!
//! ## Security contract
//! - API keys are **never** returned to the frontend — they are loaded from
//!   the `.env` file inside the Rust process only.
//! - All commands return `Result<T, String>` so JS errors surface cleanly.
//! - The UI thread is never blocked: heavy work runs on `tokio::spawn`.

use log::{error, info, warn};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::data::TradeDirection;
use crate::data::{CsvLogger, TradeRecord};
use crate::risk::DailyStats;
use crate::state::AppState;
use crate::strategy::{EntryRule, EvaluatorError, RuleEvaluator, UserStrategyConfig};
use crate::types::MarketData;

// ─────────────────────────────────────────────────────────────────────────────
// Response DTOs (frontend-safe — no raw credentials)
// ─────────────────────────────────────────────────────────────────────────────

/// Lightweight summary of a trade record safe for IPC serialisation.
///
/// `rust_decimal::Decimal` serialises to a JSON string, which React's
/// `Number()` can parse. We keep it as `String` here for explicitness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRecordDto {
    pub id: String,
    pub symbol: String,
    pub direction: String,
    pub entry_time: String,
    pub entry_price: String,
    pub stop_loss: String,
    pub take_profit: String,
    pub position_size: String,
    pub status: String,
    pub exit_time: Option<String>,
    pub exit_price: Option<String>,
    pub realized_pnl: String,
    pub phase: String,
    pub risk_per_unit: String,
    /// ATR value at entry — used for trailing stop sizing context.
    pub atr_value: String,
    /// RSI value at entry — signal quality context.
    pub entry_rsi: String,
    /// ADX value at entry — trend strength context.
    pub entry_adx: String,
    /// Trend condition label at entry (e.g. "LiveSignal", "StrongUptrend").
    pub trend_condition: String,
}

impl From<TradeRecord> for TradeRecordDto {
    fn from(r: TradeRecord) -> Self {
        Self {
            id: r.id,
            symbol: r.symbol,
            direction: format!("{:?}", r.direction),
            entry_time: r.entry_time.to_rfc3339(),
            entry_price: r.entry_price.to_string(),
            stop_loss: r.stop_loss.to_string(),
            take_profit: r.take_profit.to_string(),
            position_size: r.position_size.to_string(),
            status: format!("{:?}", r.status),
            exit_time: r.exit_time.map(|t| t.to_rfc3339()),
            exit_price: r.exit_price.map(|p| p.to_string()),
            realized_pnl: r.realized_pnl.to_string(),
            phase: format!("{:?}", r.phase),
            risk_per_unit: r.risk_per_unit.to_string(),
            atr_value: r.atr_value.to_string(),
            entry_rsi: r.entry_rsi.to_string(),
            entry_adx: r.entry_adx.to_string(),
            trend_condition: r.trend_condition,
        }
    }
}

/// System health status returned to the UI health dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStatus {
    /// Whether a trading session is active.
    pub session_active: bool,
    /// Whether the circuit breaker is active (all entries halted).
    pub circuit_breaker_active: bool,
    /// Daily loss as a percentage of starting equity (0.0 – 1.0).
    pub daily_loss_pct: String,
    /// Current account equity (paper money in mock mode).
    pub current_equity: String,
    /// Daily statistics snapshot.
    pub daily_stats: DailyStatsDto,
}

/// Serialisable subset of [`DailyStats`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyStatsDto {
    pub realized_pnl: String,
    pub trade_count: u32,
    pub wins: u32,
    pub losses: u32,
    pub win_rate: String,
}

impl From<&DailyStats> for DailyStatsDto {
    fn from(s: &DailyStats) -> Self {
        Self {
            realized_pnl: s.realized_pnl.to_string(),
            trade_count: s.trade_count,
            wins: s.wins,
            losses: s.losses,
            win_rate: s.win_rate().to_string(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// IPC Commands
// ─────────────────────────────────────────────────────────────────────────────

/// **Start a mock (paper) trading session.**
///
/// - Validates the user's JSON strategy config.
/// - Loads `BINANCE_API_KEY` / `BINANCE_API_SECRET` from the `.env` file on
///   the Rust side — these are **never** forwarded to the frontend.
/// - Initialises the `ExecutionEngine` in mock (testnet) mode.
/// - Starts the WebSocket data pipeline.
/// - Spawns the execution loop on a separate `tokio` task so the Tauri UI
///   thread is never blocked.
///
/// Returns `Err` if a session is already running or the config is invalid.
#[tauri::command]
pub async fn start_mock_session(
    config: UserStrategyConfig,
    is_live_mode: bool,
    state: State<'_, AppState>,
) -> Result<String, String> {
    // ── Guard: reject if a session is already running ─────────────────────
    if state.is_session_active() {
        return Err("A session is already running. Call stop_session first.".to_string());
    }

    // ── Validate config before doing any I/O ─────────────────────────────
    let errors = config.validate();
    if !errors.is_empty() {
        return Err(format!("Invalid strategy config:\n{}", errors.join("\n")));
    }

    info!("[IPC] start_mock_session — strategy: {}, live_mode: {}", config.name, is_live_mode);

    // ── Build the engine (mock or live) ───────────────────────────────────
    let api_key = std::env::var("BINANCE_API_KEY").unwrap_or_default();
    let api_secret = std::env::var("BINANCE_API_SECRET").unwrap_or_default();
    
    let engine_config = crate::exchange::EngineConfig {
        api_key,
        api_secret,
        testnet: !is_live_mode,
        symbols: vec!["BTCUSDT".to_string()],
        ..Default::default()
    };
    
    let engine = if is_live_mode {
        crate::exchange::ExecutionEngine::new(engine_config).await
    } else {
        crate::exchange::ExecutionEngine::new_mock()
    };
    *state.engine.lock().await = Some(engine);

    // ── Extract required intervals from config ────────────────────────────
    let mut required_intervals = std::collections::HashSet::new();
    for rule in &config.entry_rules {
        match rule {
            EntryRule::Rsi(r) => { required_intervals.insert(r.interval); },
            EntryRule::Ma(r) => { required_intervals.insert(r.interval); },
            EntryRule::Volume(r) => { required_intervals.insert(r.interval); },
        }
    }

    // ── Extract symbols from config entry rules for WS subscriptions ──────
    let symbol = "BTCUSDT".to_string(); // Phase 4: derive from config rules.
    let symbols = vec![symbol.clone()];
    let intervals = vec!["15m".to_string(), "1h".to_string(), "4h".to_string()];

    // ── Add Historical Backfill ───────────────────────────────────────────
    let mut market_data = MarketData::new(&symbol);
    for &interval in &required_intervals {
        info!("[BACKFILL] Fetching historical data for {} {:?}", symbol, interval);
        match crate::data::backfill::fetch_historical_data(&symbol, interval).await {
            Ok(candles) => {
                market_data.candles_mut(interval).extend(candles);
            }
            Err(e) => {
                let err_msg = format!("Failed to fetch historical data: {}", e);
                error!("[BACKFILL] {}", err_msg);
                return Err(err_msg);
            }
        }
    }

    // ── Start the WebSocket stack ─────────────────────────────────────────
    let (mut price_rx, _order_rx, mut kline_rx) =
        state.build_ws_stack(symbols, intervals).await;

    // Start the WS manager (takes &mut self — must hold the lock briefly).
    {
        let mut ws_guard = state.ws_manager.lock().await;
        if let Some(manager) = ws_guard.as_mut() {
            manager.start().await.map_err(|e| {
                error!("[IPC] WS manager start failed: {}", e);
                format!("WebSocket start failed: {}", e)
            })?;
        }
    } // lock released here

    // ── Restore Positions via TradeManager ────────────────────────────────
    let mut trade_manager = match crate::data::TradeManager::default_path() {
        Ok(tm) => tm,
        Err(e) => {
            let msg = format!("Failed to initialize TradeManager for recovery: {}", e);
            error!("[SESSION] {}", msg);
            return Err(msg);
        }
    };
    
    let open_trades = trade_manager.active_trades().to_vec();
    if !open_trades.is_empty() {
        info!("[SESSION] Restoring {} open trades from CSV...", open_trades.len());
        if let Some(engine) = state.engine.lock().await.as_mut() {
            engine.restore_positions(open_trades).await;
        }
    }

    // ── Mark session active ───────────────────────────────────────────────
    state
        .session_active
        .store(true, std::sync::atomic::Ordering::SeqCst);

    // ── Spawn the execution loop on a dedicated tokio task ────────────────
    // We clone the Arc<AtomicBool> so the task can self-terminate on shutdown.
    let session_flag = state.session_active.clone();
    let strategy_config = config.clone();
    let risk_manager = state.risk_manager.clone();

    tokio::spawn(async move {
        info!("[SESSION] Execution loop started");
        let evaluator = RuleEvaluator::new();
        let mut initialized = false;
        let mut last_price: Option<Decimal> = None;
        let mut last_traded_candle: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

        // The loop runs until the shutdown flag is set.
        loop {
            if !session_flag.load(std::sync::atomic::Ordering::SeqCst) {
                info!("[SESSION] Shutdown flag detected — execution loop exiting");
                break;
            }

            // ── Drain price ticks (non-blocking) ─────────────────────────
            while let Ok(tick) = price_rx.try_recv() {
                log::trace!("[SESSION] Price tick: {} @ {}", tick.symbol, tick.price);
                last_price = Some(tick.price);
            }

            // ── Drain klines ───────────────────────────────────────
            while let Ok(kline) = kline_rx.try_recv() {
                let is_closed = kline.kline.is_closed;

                if let Some(candle) = crate::types::kline_event_to_candle(&kline) {
                    let interval = match kline.kline.interval.as_str() {
                        "15m" => Some(crate::types::Interval::M15),
                        "1h" => Some(crate::types::Interval::H1),
                        "4h" => Some(crate::types::Interval::H4),
                        _ => None,
                    };
                    
                    if let Some(inv) = interval {
                        let candles = market_data.candles_mut(inv);
                        if is_closed {
                            info!(
                                "[SESSION] Closed kline: {} {} close={}",
                                kline.symbol, kline.kline.interval, kline.kline.close
                            );
                            candles.push(candle);
                        } else {
                            if let Some(last) = candles.last_mut() {
                                if last.timestamp == candle.timestamp {
                                    *last = candle;
                                } else {
                                    candles.push(candle);
                                }
                            } else {
                                candles.push(candle);
                            }
                        }
                    }
                }

                if !initialized && !market_data.candles_15m.is_empty() {
                    initialized = true;
                    info!("[SESSION] Engine initialized with 15m candles");
                }

                if initialized {
                    match evaluator.evaluate(&market_data, &strategy_config) {
                        Ok(true) => {
                            log::info!("[EVALUATOR] Signal generated: conditions met!");

                            let symbol = "BTCUSDT";

                            // Task 2: Duplicate Trade Guard
                            let current_candle_time = market_data.candles_15m.last().map(|c| c.timestamp.timestamp_millis() as u64).unwrap_or(0);
                            if let Some(&last_time) = last_traded_candle.get(symbol) {
                                if last_time == current_candle_time {
                                    log::debug!("[TRADE] Duplicate signal for candle {}, skipping.", current_candle_time);
                                    continue;
                                }
                            }

                            // ── Execute mock trade ───────────────────────
                            let entry_price = last_price.unwrap_or_else(|| {
                                // Fallback: use the last closed candle's close price
                                market_data.candles_15m.last()
                                    .map(|c| c.close)
                                    .unwrap_or(dec!(0))
                            });

                            if entry_price == dec!(0) {
                                log::warn!("[TRADE] Skipping — no price available");
                                continue;
                            }

                            // Use 1% of entry as a simple stop-loss distance for mock
                            let stop_distance = entry_price * dec!(0.01);
                            let stop_loss = entry_price - stop_distance;
                            let minimum_rr = Decimal::try_from(strategy_config.risk.minimum_rr).unwrap_or(dec!(2));
                            let take_profit = entry_price + stop_distance * minimum_rr;

                            // Calculate position size via risk manager
                            let rm = risk_manager.lock().await;

                            if rm.is_trading_halted() {
                                log::warn!("[TRADE] Circuit breaker active — skipping entry");
                                continue;
                            }

                            // Task 1: Check MAX_OPEN_POSITIONS using the properly updated active_trades
                            if trade_manager.active_trades().len() >= crate::exchange::config::MAX_OPEN_POSITIONS {
                                log::warn!("[TRADE] MAX_OPEN_POSITIONS guard hit — skipping entry");
                                continue;
                            }

                            // Task 3: Calculate indicators dynamically for logging
                            let mut atr_15m = stop_distance;
                            let atr_4h = stop_distance;
                            let mut rsi_val = dec!(0);
                            let mut adx_val = dec!(0);

                            let closes: Vec<f64> = market_data.candles_15m.iter().map(|c| c.close.to_f64().unwrap_or(0.0)).collect();
                            let highs: Vec<f64> = market_data.candles_15m.iter().map(|c| c.high.to_f64().unwrap_or(0.0)).collect();
                            let lows: Vec<f64> = market_data.candles_15m.iter().map(|c| c.low.to_f64().unwrap_or(0.0)).collect();

                            if closes.len() > 14 {
                                let mut tr_sum = 0.0;
                                let mut gains = 0.0;
                                let mut losses = 0.0;
                                for i in 1..15 {
                                    let idx = closes.len() - i;
                                    let tr1 = highs[idx] - lows[idx];
                                    let tr2 = (highs[idx] - closes[idx - 1]).abs();
                                    let tr3 = (lows[idx] - closes[idx - 1]).abs();
                                    tr_sum += tr1.max(tr2).max(tr3);

                                    let diff = closes[idx] - closes[idx - 1];
                                    if diff > 0.0 { gains += diff; } else { losses -= diff; }
                                }
                                if let Some(atr) = rust_decimal::Decimal::from_f64_retain(tr_sum / 14.0) {
                                    atr_15m = atr;
                                }

                                let avg_loss = losses / 14.0;
                                if avg_loss == 0.0 {
                                    rsi_val = dec!(100);
                                } else {
                                    let rs = (gains / 14.0) / avg_loss;
                                    if let Some(rsi) = rust_decimal::Decimal::from_f64_retain(100.0 - (100.0 / (1.0 + rs))) {
                                        rsi_val = rsi;
                                    }
                                }
                                adx_val = dec!(25); // Simplified ADX proxy
                            }

                            match rm.calculate_position_size(
                                entry_price,
                                stop_loss,
                                atr_15m,
                                atr_4h,
                                None,
                            ) {
                                Ok(sizing) => {
                                    let trade_id = format!("mock-{}", uuid::Uuid::new_v4());
                                    
                                    // Drop the risk manager lock before performing I/O via trade_manager
                                    drop(rm);

                                    // Use TradeManager to open trade, log to CSV, and update active_trades
                                    match trade_manager.open_trade(
                                        &trade_id,
                                        symbol,
                                        TradeDirection::Long,
                                        entry_price,
                                        stop_loss,
                                        take_profit,
                                        sizing.size,
                                        atr_15m,
                                        rsi_val,
                                        adx_val,
                                        "LiveSignal".to_string(),
                                    ) {
                                        Ok(trade) => {
                                            // Update last_traded_candle since trade successfully opened
                                            last_traded_candle.insert(symbol.to_string(), current_candle_time);

                                            log::info!(
                                                "[TRADE] ✅ ENTRY: {} {} @ {} | SL={} TP={} | Size={}",
                                                trade.direction, trade.symbol, entry_price,
                                                stop_loss, take_profit, sizing.size
                                            );
                                        }
                                        Err(e) => {
                                            log::error!("[TRADE] Failed to open trade in TradeManager: {}", e);
                                        }
                                    }
                                }
                                Err(e) => {
                                    log::warn!("[TRADE] Position sizing rejected: {}", e);
                                }
                            }
                        }
                        Ok(false) => {
                            // Suppress spam for open candles
                        }
                        Err(EvaluatorError::InsufficientData { required, got, interval }) => {
                            log::debug!("[EVALUATOR] Insufficient data for {:?}: required {}, available {}", interval, required, got);
                        }
                        Err(e) => {
                            log::warn!("[EVALUATOR] Evaluation error: {:?}", e);
                        }
                    }
                }
            }

            // Yield to the tokio runtime — avoids a busy-spin.
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        info!("[SESSION] Execution loop terminated");
    });

    Ok(format!(
        "Mock session started — strategy '{}' is active on Binance testnet.",
        config.name
    ))
}

/// **Stop the current trading session.**
///
/// Sends a shutdown signal to the WebSocket manager, marks the session
/// inactive, and returns a summary of open positions (to be closed manually
/// or flattened in Phase 4).
///
/// Safe to call even if no session is running.
#[tauri::command]
pub async fn stop_session(state: State<'_, AppState>) -> Result<String, String> {
    if !state.is_session_active() {
        warn!("[IPC] stop_session called with no active session");
        return Ok("No active session to stop.".to_string());
    }

    info!("[IPC] stop_session — sending shutdown signal");
    state.stop_session().await;

    Ok("Session stopped. WebSocket streams are shutting down.".to_string())
}

/// **Retrieve the full trade history from `trades_log.csv`.**
///
/// Reads the local CSV file via [`CsvLogger`] and returns all records as a
/// JSON array.  Returns an empty array if the file does not yet exist.
///
/// The file path defaults to `trades_log.csv` relative to the Tauri app data
/// directory.  In Phase 4 this will be resolved via `app_handle.path()`.
#[tauri::command]
pub async fn get_trade_history() -> Result<Vec<TradeRecordDto>, String> {
    log::trace!("[IPC] get_trade_history");

    // Resolve to a sensible default. Phase 4 will use app_handle.path().
    let csv_path = "trades_log.csv";

    // If the file doesn't exist yet, return an empty list — not an error.
    if !std::path::Path::new(csv_path).exists() {
        return Ok(Vec::new());
    }

    let logger = CsvLogger::new(csv_path)
        .map_err(|e| format!("Failed to open trades_log.csv: {}", e))?;

    let records = logger
        .read_all_records()
        .map_err(|e| format!("Failed to read trade history: {}", e))?;

    log::trace!("[IPC] Returning {} trade records", records.len());

    Ok(records.into_iter().map(TradeRecordDto::from).collect())
}

/// **Get the current system health status.**
///
/// Returns risk metrics, circuit-breaker state, and daily statistics so the
/// React dashboard can render a live health panel.
///
/// This command holds the `risk_manager` lock only briefly and returns a
/// fully-owned DTO — the UI thread receives a snapshot, not a live reference.
#[tauri::command]
pub async fn get_system_status(state: State<'_, AppState>) -> Result<SystemStatus, String> {
    let rm = state.risk_manager.lock().await;

    let status = SystemStatus {
        session_active: state.is_session_active(),
        circuit_breaker_active: rm.is_trading_halted(),
        daily_loss_pct: rm.daily_loss_percentage().to_string(),
        current_equity: rm.current_equity().to_string(),
        daily_stats: DailyStatsDto::from(rm.daily_stats()),
    };

    Ok(status)
}
