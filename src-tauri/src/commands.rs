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
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
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

    info!(
        "[IPC] start_mock_session — strategy: {}, live_mode: {}",
        config.name, is_live_mode
    );

    // ── Build the engine — load credentials for the selected environment ──
    // Testnet and Live Binance accounts use different API keys.
    // The UI writes them to separate env vars so the wrong key can never
    // accidentally hit the wrong endpoint.
    let (api_key, api_secret) = if is_live_mode {
        (
            std::env::var("BINANCE_API_KEY").unwrap_or_default(),
            std::env::var("BINANCE_API_SECRET").unwrap_or_default(),
        )
    } else {
        (
            std::env::var("BINANCE_TESTNET_API_KEY").unwrap_or_default(),
            std::env::var("BINANCE_TESTNET_API_SECRET").unwrap_or_default(),
        )
    };

    if api_key.is_empty() || api_secret.is_empty() {
        let env_hint = if is_live_mode {
            "BINANCE_API_KEY / BINANCE_API_SECRET"
        } else {
            "BINANCE_TESTNET_API_KEY / BINANCE_TESTNET_API_SECRET"
        };
        warn!(
            "[IPC] {} not set — open API Credentials and save {} keys first",
            env_hint,
            if is_live_mode { "Live" } else { "Testnet" }
        );
    }

    let engine_config = crate::exchange::EngineConfig {
        api_key,
        api_secret,
        testnet: !is_live_mode,
        symbols: vec![
            "BTCUSDT".to_string(),
            "ETHUSDT".to_string(),
            "SOLUSDT".to_string(),
        ],
        ..Default::default()
    };

    let engine = crate::exchange::ExecutionEngine::new(engine_config).await;

    // Fetch the real USDT balance from Binance Testnet and seed the risk manager.
    let testnet_balance = engine
        .fetch_account_balance()
        .await
        .map_err(|e| format!("Failed to fetch Testnet USDT balance: {:?}", e))?;

    info!("[IPC] Testnet USDT balance: {}", testnet_balance);

    // Request a user data stream listen key so the WebSocket layer can receive
    // live OrderUpdate events.  A failure here is non-fatal — we degrade
    // gracefully to market-data-only rather than aborting the session.
    let listen_key = match engine.create_listen_key().await {
        Ok(key) => {
            info!("[IPC] User data stream listen key obtained");
            Some(key)
        }
        Err(e) => {
            warn!(
                "[IPC] Failed to obtain listen key — OrderUpdate stream disabled: {:?}",
                e
            );
            None
        }
    };

    *state.engine.lock().await = Some(engine);

    // Reinitialise the risk manager with the live Testnet balance — replaces
    // any previous session state and removes the hardcoded $10 000 seed.
    *state.risk_manager.lock().await = crate::risk::RiskManager::new(testnet_balance);

    // ── Extract required intervals from config ────────────────────────────
    let mut required_intervals = std::collections::HashSet::new();
    for rule in &config.entry_rules {
        match rule {
            EntryRule::Rsi(r) => {
                required_intervals.insert(r.interval);
            }
            EntryRule::Ma(r) => {
                required_intervals.insert(r.interval);
            }
            EntryRule::Volume(r) => {
                required_intervals.insert(r.interval);
            }
        }
    }

    // ── Trading universe ─────────────────────────────────────────────────
    // All three symbols are subscribed, backfilled, and evaluated
    // independently. Each symbol runs its own signal check and can open a
    // position when its own indicators satisfy the entry rules.
    let trade_symbols: Vec<String> = vec![
        "BTCUSDT".to_string(),
        "ETHUSDT".to_string(),
        "SOLUSDT".to_string(),
    ];
    let symbols = trade_symbols.clone();
    let intervals = vec!["15m".to_string(), "1h".to_string(), "4h".to_string()];

    // ── Add Historical Backfill (per-symbol) ──────────────────────────────
    let mut market_data_map: std::collections::HashMap<String, MarketData> =
        std::collections::HashMap::new();
    for sym in &symbols {
        let mut md = MarketData::new(sym);
        for &interval in &required_intervals {
            info!(
                "[BACKFILL] Fetching historical data for {} {:?}",
                sym, interval
            );
            match crate::data::backfill::fetch_historical_data(sym, interval).await {
                Ok(candles) => {
                    md.candles_mut(interval).extend(candles);
                }
                Err(e) => {
                    let err_msg = format!("Failed to fetch historical data for {}: {}", sym, e);
                    error!("[BACKFILL] {}", err_msg);
                    return Err(err_msg);
                }
            }
        }
        market_data_map.insert(sym.clone(), md);
    }

    // ── Start the WebSocket stack ─────────────────────────────────────────
    let (mut price_rx, _order_rx, mut kline_rx) =
        state.build_ws_stack(symbols, intervals, listen_key).await;

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
        info!(
            "[SESSION] Restoring {} open trades from CSV...",
            open_trades.len()
        );
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
    // Clone the full trading universe into the closure so the loop can
    // evaluate every symbol independently on each kline event.
    let trade_symbols_inner = trade_symbols.clone();

    tokio::spawn(async move {
        info!("[SESSION] Execution loop started");
        let evaluator = RuleEvaluator::new();
        let mut initialized = false;
        // Per-symbol last price from aggTrade ticks.
        let mut last_prices: std::collections::HashMap<String, Decimal> =
            std::collections::HashMap::new();
        let mut last_traded_candle: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();

        // The loop runs until the shutdown flag is set.
        loop {
            if !session_flag.load(std::sync::atomic::Ordering::SeqCst) {
                info!("[SESSION] Shutdown flag detected — execution loop exiting");
                break;
            }

            // ── Drain price ticks (non-blocking, per-symbol) ─────────────
            while let Ok(tick) = price_rx.try_recv() {
                log::trace!("[SESSION] Price tick: {} @ {}", tick.symbol, tick.price);
                last_prices.insert(tick.symbol.clone(), tick.price);
            }

            // ── Drain klines (routed to the correct symbol's MarketData) ──
            while let Ok(kline) = kline_rx.try_recv() {
                let is_closed = kline.kline.is_closed;
                let kline_symbol = kline.symbol.clone();

                if let Some(candle) = crate::types::kline_event_to_candle(&kline) {
                    let interval = match kline.kline.interval.as_str() {
                        "15m" => Some(crate::types::Interval::M15),
                        "1h" => Some(crate::types::Interval::H1),
                        "4h" => Some(crate::types::Interval::H4),
                        _ => None,
                    };

                    if let Some(inv) = interval {
                        // Route candle to the correct symbol's MarketData.
                        let md = market_data_map
                            .entry(kline_symbol.clone())
                            .or_insert_with(|| MarketData::new(&kline_symbol));
                        let candles = md.candles_mut(inv);
                        if is_closed {
                            info!(
                                "[SESSION] Closed kline: {} {} close={}",
                                kline_symbol, kline.kline.interval, kline.kline.close
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

                // Initialise once any subscribed symbol has accumulated 15m candles.
                if !initialized {
                    if trade_symbols_inner.iter().any(|s| {
                        market_data_map
                            .get(s.as_str())
                            .map_or(false, |md| !md.candles_15m.is_empty())
                    }) {
                        initialized = true;
                        info!("[SESSION] Engine initialized — trading universe ready");
                    }
                }

                if initialized {
                    // Evaluate every symbol in the trading universe independently.
                    // Each symbol uses its own MarketData so signals fire per-asset.
                    'symbol_loop: for sym in &trade_symbols_inner {
                        let symbol = sym.as_str();

                        let trade_md = match market_data_map.get(symbol) {
                            Some(md) => md,
                            None => {
                                log::warn!(
                                    "[TRADE] No MarketData for {} yet — skipping",
                                    symbol
                                );
                                continue 'symbol_loop;
                            }
                        };

                        match evaluator.evaluate(trade_md, &strategy_config) {
                            Ok(true) => {
                                log::info!("[EVALUATOR] Signal on {}: conditions met!", symbol);

                                // Duplicate Trade Guard — keyed on the symbol's candle time.
                                let current_candle_time = trade_md
                                    .candles_15m
                                    .last()
                                    .map(|c| c.timestamp.timestamp_millis() as u64)
                                    .unwrap_or(0);
                                if let Some(&last_time) = last_traded_candle.get(symbol) {
                                    if last_time == current_candle_time {
                                        log::debug!(
                                            "[TRADE] Duplicate signal for {} candle {}, skipping.",
                                            symbol, current_candle_time
                                        );
                                        continue 'symbol_loop;
                                    }
                                }

                                // Entry price: prefer live aggTrade tick, fall back to kline close.
                                let entry_price =
                                    last_prices.get(symbol).copied().unwrap_or_else(|| {
                                        trade_md
                                            .candles_15m
                                            .last()
                                            .map(|c| c.close)
                                            .unwrap_or(dec!(0))
                                    });

                                if entry_price == dec!(0) {
                                    log::warn!(
                                        "[TRADE] Skipping — no price available for {}",
                                        symbol
                                    );
                                    continue 'symbol_loop;
                                }

                                let direction = TradeDirection::Long;
                                let slipped_entry_price = match direction {
                                    TradeDirection::Long => {
                                        entry_price
                                            * (Decimal::ONE
                                                + crate::exchange::config::MOCK_SLIPPAGE_PCT)
                                    }
                                    TradeDirection::Short => {
                                        entry_price
                                            * (Decimal::ONE
                                                - crate::exchange::config::MOCK_SLIPPAGE_PCT)
                                    }
                                };

                                let rm = risk_manager.lock().await;

                                if rm.is_trading_halted() {
                                    log::warn!("[TRADE] Circuit breaker active — skipping entry");
                                    continue 'symbol_loop;
                                }

                                if trade_manager.active_trades().len()
                                    >= crate::exchange::config::MAX_OPEN_POSITIONS
                                {
                                    log::warn!(
                                        "[TRADE] MAX_OPEN_POSITIONS guard hit — skipping entry"
                                    );
                                    continue 'symbol_loop;
                                }

                                // ── Compute indicators from the symbol's own candles ────
                                let mut atr_15m = dec!(0);
                                let mut rsi_val = dec!(0);
                                let mut adx_val = dec!(0);

                                let closes: Vec<f64> = trade_md
                                    .candles_15m
                                    .iter()
                                    .map(|c| c.close.to_f64().unwrap_or(0.0))
                                    .collect();
                                let highs: Vec<f64> = trade_md
                                    .candles_15m
                                    .iter()
                                    .map(|c| c.high.to_f64().unwrap_or(0.0))
                                    .collect();
                                let lows: Vec<f64> = trade_md
                                    .candles_15m
                                    .iter()
                                    .map(|c| c.low.to_f64().unwrap_or(0.0))
                                    .collect();

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
                                        if diff > 0.0 {
                                            gains += diff;
                                        } else {
                                            losses -= diff;
                                        }
                                    }
                                    if let Some(atr) =
                                        rust_decimal::Decimal::from_f64_retain(tr_sum / 14.0)
                                    {
                                        atr_15m = atr;
                                    }

                                    let avg_loss = losses / 14.0;
                                    if avg_loss == 0.0 {
                                        rsi_val = dec!(100);
                                    } else {
                                        let rs = (gains / 14.0) / avg_loss;
                                        if let Some(rsi) =
                                            rust_decimal::Decimal::from_f64_retain(
                                                100.0 - (100.0 / (1.0 + rs)),
                                            )
                                        {
                                            rsi_val = rsi;
                                        }
                                    }
                                    adx_val = dec!(25); // Simplified ADX proxy
                                }

                                // Skip if ATR is zero (insufficient data).
                                if atr_15m == dec!(0) {
                                    log::warn!(
                                        "[TRADE] ATR is zero for {} — insufficient candle data, skipping",
                                        symbol
                                    );
                                    continue 'symbol_loop;
                                }

                                let atr_4h = atr_15m; // Proxy: use 15m ATR until 4H accumulates.
                                let stop_loss = slipped_entry_price - dec!(2) * atr_15m;
                                let take_profit = slipped_entry_price + dec!(3) * atr_15m;

                                log::info!(
                                    "[TRADE] {} raw_entry={} slipped_entry={} | ATR={} | SL={} TP={}",
                                    symbol,
                                    entry_price,
                                    slipped_entry_price,
                                    atr_15m,
                                    stop_loss,
                                    take_profit
                                );

                                match rm.calculate_position_size(
                                    slipped_entry_price,
                                    stop_loss,
                                    atr_15m,
                                    atr_4h,
                                    None,
                                ) {
                                    Ok(sizing) => {
                                        let trade_id = format!("mock-{}", uuid::Uuid::new_v4());

                                        // Release the lock before CSV I/O.
                                        drop(rm);

                                        match trade_manager.open_trade(
                                            &trade_id,
                                            symbol,
                                            direction,
                                            slipped_entry_price,
                                            stop_loss,
                                            take_profit,
                                            sizing.size,
                                            atr_15m,
                                            rsi_val,
                                            adx_val,
                                            "LiveSignal".to_string(),
                                        ) {
                                            Ok(trade) => {
                                                last_traded_candle.insert(
                                                    symbol.to_string(),
                                                    current_candle_time,
                                                );
                                                log::info!(
                                                    "[TRADE] ✅ ENTRY: {} {} @ {} | SL={} TP={} | Size={}",
                                                    trade.direction,
                                                    trade.symbol,
                                                    slipped_entry_price,
                                                    stop_loss,
                                                    take_profit,
                                                    sizing.size
                                                );
                                            }
                                            Err(e) => {
                                                log::error!(
                                                    "[TRADE] Failed to open trade for {}: {}",
                                                    symbol,
                                                    e
                                                );
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        log::warn!(
                                            "[TRADE] Position sizing rejected for {}: {}",
                                            symbol,
                                            e
                                        );
                                    }
                                }
                            }
                            Ok(false) => {
                                // No signal — suppress log spam for open candles.
                            }
                            Err(EvaluatorError::InsufficientData {
                                required,
                                got,
                                interval,
                            }) => {
                                log::debug!(
                                    "[EVALUATOR] Insufficient data for {} {:?}: required {}, got {}",
                                    symbol,
                                    interval,
                                    required,
                                    got
                                );
                            }
                            Err(e) => {
                                log::warn!(
                                    "[EVALUATOR] Evaluation error on {}: {:?}",
                                    symbol,
                                    e
                                );
                            }
                        }
                    } // end 'symbol_loop
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

    let logger =
        CsvLogger::new(csv_path).map_err(|e| format!("Failed to open trades_log.csv: {}", e))?;

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

// ─────────────────────────────────────────────────────────────────────────────
// Credential Management
// ─────────────────────────────────────────────────────────────────────────────

/// **Save Binance API credentials to the local `.env` file.**
///
/// ## Security model
/// - The frontend passes keys only once via IPC — they are **never** stored in
///   the browser, localStorage, or any persistent React state.
/// - The backend writes the keys to the `.env` file on disk and immediately
///   reloads them into the process environment via `dotenvy`, so the session
///   starts with the new credentials without requiring an app restart.
/// - The `.env` file is git-ignored and lives only on the user's machine.
///
/// ## Environment model
/// Testnet and Live Binance accounts have **completely different** API keys.
/// `mode` selects which pair of env vars to write:
/// - `"testnet"` → `BINANCE_TESTNET_API_KEY` / `BINANCE_TESTNET_API_SECRET`
/// - `"live"`    → `BINANCE_API_KEY`          / `BINANCE_API_SECRET`
///
/// ## Behaviour
/// - Matching lines in `.env` are updated in-place; all other lines are kept.
/// - Missing lines are appended.
/// - An empty `api_key` or `api_secret` is rejected before any file I/O.
#[tauri::command]
pub async fn save_api_credentials(
    api_key: String,
    api_secret: String,
    mode: String,
) -> Result<(), String> {
    // ── Validate inputs ───────────────────────────────────────────────────────
    let api_key = api_key.trim().to_string();
    let api_secret = api_secret.trim().to_string();

    if api_key.is_empty() {
        return Err("API key must not be empty.".to_string());
    }
    if api_secret.is_empty() {
        return Err("API secret must not be empty.".to_string());
    }

    // Derive env-var names from the selected environment.
    let is_testnet = mode.trim() != "live";
    let (key_var, secret_var) = if is_testnet {
        ("BINANCE_TESTNET_API_KEY", "BINANCE_TESTNET_API_SECRET")
    } else {
        ("BINANCE_API_KEY", "BINANCE_API_SECRET")
    };

    info!(
        "[CREDENTIALS] Saving {} credentials → {}=… {}=…",
        if is_testnet { "Testnet" } else { "Live" },
        key_var,
        secret_var
    );

    // ── Resolve the .env path (same directory as the running binary) ──────────
    let env_path = std::env::current_exe()
        .map_err(|e| format!("Cannot locate binary directory: {}", e))?
        .parent()
        .ok_or_else(|| "Binary has no parent directory".to_string())?
        .join(".env");

    // Fallback: current working directory (matches dev builds).
    let env_path = if env_path.exists() {
        env_path
    } else {
        std::path::PathBuf::from(".env")
    };

    // ── Read the existing file (or start with an empty buffer) ────────────────
    let existing = if env_path.exists() {
        std::fs::read_to_string(&env_path).map_err(|e| format!("Failed to read .env: {}", e))?
    } else {
        String::new()
    };

    // ── Rewrite lines, updating matching keys in-place ────────────────────────
    let key_prefix = format!("{}=", key_var);
    let secret_prefix = format!("{}=", secret_var);
    let mut key_written = false;
    let mut secret_written = false;

    let mut new_lines: Vec<String> = existing
        .lines()
        .map(|line| {
            if line.starts_with(&key_prefix) {
                key_written = true;
                format!("{}={}", key_var, api_key)
            } else if line.starts_with(&secret_prefix) {
                secret_written = true;
                format!("{}={}", secret_var, api_secret)
            } else {
                line.to_string()
            }
        })
        .collect();

    // Append any keys that were not found.
    if !key_written {
        new_lines.push(format!("{}={}", key_var, api_key));
    }
    if !secret_written {
        new_lines.push(format!("{}={}", secret_var, api_secret));
    }

    // Ensure a trailing newline.
    let mut output = new_lines.join("\n");
    output.push('\n');

    // ── Write atomically (temp file + rename) ─────────────────────────────────
    let tmp_path = env_path.with_extension("env.tmp");
    std::fs::write(&tmp_path, &output).map_err(|e| format!("Failed to write temp .env: {}", e))?;
    std::fs::rename(&tmp_path, &env_path)
        .map_err(|e| format!("Failed to finalise .env: {}", e))?;

    // ── Reload into the live process so the new keys take effect immediately ──
    std::env::set_var(key_var, &api_key);
    std::env::set_var(secret_var, &api_secret);

    info!("[CREDENTIALS] .env updated and process environment refreshed.");
    Ok(())
}
