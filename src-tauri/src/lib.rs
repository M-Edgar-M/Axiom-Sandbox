//! Axiom Sandbox — Tauri backend library root.
//!
//! ## Module Layout
//! - `types`    — Core financial data types (Candle, Position, Trade, …)
//! - `websocket`— Dual-stream WebSocket manager (market + user data)
//! - `exchange` — ExecutionEngine for Binance Spot/Futures (testnet or live)
//! - `data`     — CsvLogger & TradeManager for cloud-free local state
//! - `risk`     — RiskManager with 5% ceiling & daily circuit-breaker
//! - `strategy` — Placeholder: JSON Rule Evaluator (Phase 2)

// ── Phase 1: Infrastructure scaffold — all ported modules are called by Phase 2.
// Suppress expected dead-code noise until the IPC command surface is built.
#![allow(dead_code)]
#![allow(unused_imports)]

pub mod data;
pub mod exchange;
pub mod risk;
pub mod strategy;
pub mod types;
pub mod websocket;

// ── Tauri IPC Commands ─────────────────────────────────────────────────────

/// Greet command — placeholder until the full command surface is built in Phase 2.
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Axiom Sandbox — Hello, {}! Engine is online.", name)
}

/// Tauri application entry point called by `main.rs`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Load .env credentials before anything else.
    // `dotenvy::dotenv()` is a no-op (Ok) if the file does not exist, so
    // this is safe to call unconditionally.
    let _ = dotenvy::dotenv();

    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running Axiom Sandbox");
}
