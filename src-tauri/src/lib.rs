//! Axiom Sandbox — Tauri backend library root.
//!
//! ## Module Layout
//! - `types`    — Core financial data types (Candle, Position, Trade, …)
//! - `websocket`— Dual-stream WebSocket manager (market + user data)
//! - `exchange` — ExecutionEngine for Binance Spot/Futures (testnet or live)
//! - `data`     — CsvLogger & TradeManager for cloud-free local state
//! - `risk`     — RiskManager with 5% ceiling & daily circuit-breaker
//! - `strategy` — JSON Rule Evaluator (Phase 2)
//! - `state`    — Thread-safe Tauri managed state (Phase 3)
//! - `commands` — Tauri IPC command surface (Phase 3)

// ── Phase 1/2/3: Infrastructure scaffold.
// Suppress dead-code noise for modules not yet called from the UI.
#![allow(dead_code)]
#![allow(unused_imports)]

pub mod data;
pub mod exchange;
pub mod risk;
pub mod state;
pub mod strategy;
pub mod types;
pub mod websocket;

mod commands;

// ── Tauri Application Entry Point ──────────────────────────────────────────

/// Tauri application entry point called by `main.rs`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Load .env credentials before anything else.
    // `dotenvy::dotenv()` is a no-op (Ok) if the file does not exist, so
    // this is safe to call unconditionally.
    let _ = dotenvy::dotenv();

    // Initialise the shared application state.
    // `AppState::new()` seeds the RiskManager at $10,000 paper equity.
    let app_state = state::AppState::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        // Register the managed state — accessible in every command via
        // `State<'_, AppState>`.
        .manage(app_state)
        // Register all Phase 3 IPC commands.
        .invoke_handler(tauri::generate_handler![
            commands::start_mock_session,
            commands::stop_session,
            commands::get_trade_history,
            commands::get_system_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Axiom Sandbox");
}
