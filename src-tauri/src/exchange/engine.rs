//! ExecutionEngine for Binance USD(S)-M Futures live trading.
//!
//! Handles order placement, position monitoring, and in-trade management.
//! All Binance API calls are wrapped in `tokio::task::spawn_blocking` because
//! the `binance` crate performs synchronous HTTP requests.

use std::collections::HashMap;
use std::sync::Arc;

use binance::account::OrderSide as BinanceOrderSide;
use binance::api::Binance as BinanceApi;
use binance::futures::account::{CustomOrderRequest, FuturesAccount, OrderType as FutOrderType};
use binance::futures::general::FuturesGeneral;
use binance::model::Filters;
use hmac::{Hmac, Mac};
use log::{error, info, warn};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::sync::RwLock;

use crate::types::{Position, Price, Volume};

use super::config;
use super::orders::ExecutionError;
pub use super::orders::OrderSide;
use super::trade_manager::{ManagedPosition, ManagementAction, TradeState, evaluate_position};

// ─── Re-export EngineConfig ──────────────────────────────────────────────────

/// Funding rate information for a symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingRate {
    /// Trading symbol.
    pub symbol: String,
    /// Current funding rate.
    pub rate: Decimal,
    /// Next funding time (timestamp).
    pub next_funding_time: i64,
}

/// Price ticker for a symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceTicker {
    /// Trading symbol.
    pub symbol: String,
    /// Current price.
    pub price: Price,
    /// Best bid price.
    pub bid: Price,
    /// Best ask price.
    pub ask: Price,
}

/// Account balance information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountBalance {
    /// Asset name (e.g., "USDT").
    pub asset: String,
    /// Free balance.
    pub free: Decimal,
    /// Locked balance.
    pub locked: Decimal,
}

impl AccountBalance {
    /// Returns total balance (free + locked).
    pub fn total(&self) -> Decimal {
        self.free + self.locked
    }
}

/// Per-symbol exchange filters fetched from Binance `/fapi/v1/exchangeInfo`.
#[derive(Debug, Clone)]
pub struct SymbolFilters {
    /// LOT_SIZE stepSize — quantity must satisfy `(qty - minQty) % stepSize == 0`.
    pub step_size: Decimal,
    /// PRICE_FILTER tickSize — price must satisfy `(price - minPrice) % tickSize == 0`.
    pub tick_size: Decimal,
    /// LOT_SIZE minQty — minimum order quantity.
    pub min_qty: Decimal,
}

/// Configuration for the ExecutionEngine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    /// API key for exchange.
    pub api_key: String,
    /// API secret for exchange.
    pub api_secret: String,
    /// Whether to use testnet.
    pub testnet: bool,
    /// Maximum slippage allowed.
    pub max_slippage: Decimal,
    /// Funding rate limit.
    pub funding_rate_limit: Decimal,
    /// Symbols to fetch exchange filters for on startup.
    pub symbols: Vec<String>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            api_secret: String::new(),
            testnet: true,
            max_slippage: config::MAX_SLIPPAGE_PCT,
            funding_rate_limit: config::FUNDING_RATE_LIMIT,
            symbols: Vec::new(),
        }
    }
}

// ─── Price / Quantity Rounding ───────────────────────────────────────────────

/// Round quantity **down** to the nearest valid LOT_SIZE step.
///
/// `(qty / step_size).floor() * step_size`
///
/// Rounding down ensures we never exceed the risk budget or available balance.
fn round_qty_to_step(qty: Decimal, step_size: Decimal) -> f64 {
    let stepped = (qty / step_size).floor() * step_size;
    stepped.to_f64().unwrap_or(0.0)
}

/// Round price to the nearest valid PRICE_FILTER tick.
///
/// `(price / tick_size).round() * tick_size`
fn round_price_to_tick(price: Decimal, tick_size: Decimal) -> f64 {
    let ticked = (price / tick_size).round() * tick_size;
    ticked.to_f64().unwrap_or(0.0)
}

/// Build a blank `CustomOrderRequest` with safe defaults so callers only need
/// to fill in the fields that differ.
fn blank_order(
    symbol: &str,
    side: BinanceOrderSide,
    order_type: FutOrderType,
) -> CustomOrderRequest {
    CustomOrderRequest {
        symbol: symbol.to_string(),
        side,
        position_side: None,
        order_type,
        time_in_force: None,
        qty: None,
        reduce_only: None,
        price: None,
        stop_price: None,
        close_position: None,
        activation_price: None,
        callback_rate: None,
        working_type: None,
        price_protect: None,
        new_client_order_id: None,
    }
}

// ─── Binance Client Factory ──────────────────────────────────────────────────

/// Construct a `FuturesAccount` client from the engine's config.
///
/// Instantiated fresh inside each `spawn_blocking` closure to avoid
/// `Arc<Mutex<>>` complexity; the client is lightweight.
fn make_client(config: &EngineConfig) -> FuturesAccount {
    if config.testnet {
        FuturesAccount::new_with_config(
            Some(config.api_key.clone()),
            Some(config.api_secret.clone()),
            &binance::config::Config::testnet(),
        )
    } else {
        let mainnet_cfg =
            binance::config::Config::default().set_rest_api_endpoint("https://fapi.binance.com");
        FuturesAccount::new_with_config(
            Some(config.api_key.clone()),
            Some(config.api_secret.clone()),
            &mainnet_cfg,
        )
    }
}

// ─── Side Helpers ────────────────────────────────────────────────────────────

/// Returns the Binance order side needed to open an entry position.
fn entry_side(direction: Position) -> BinanceOrderSide {
    match direction {
        Position::Long => BinanceOrderSide::Buy,
        _ => BinanceOrderSide::Sell,
    }
}

/// Returns the Binance order side needed to close a position (opposite of entry).
fn close_side(direction: Position) -> BinanceOrderSide {
    match direction {
        Position::Long => BinanceOrderSide::Sell,
        _ => BinanceOrderSide::Buy,
    }
}

// ─── Algo Order Helpers ──────────────────────────────────────────────────

type HmacSha256 = Hmac<Sha256>;

/// Compute HMAC-SHA256 hex signature for a Binance query string.
fn hmac_sign(query: &str, secret: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
    mac.update(query.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Returns the correct Binance Futures REST base URL for algo orders.
fn algo_base_url(testnet: bool) -> &'static str {
    if testnet {
        "https://testnet.binancefuture.com"
    } else {
        "https://fapi.binance.com"
    }
}

/// Place a conditional order via Binance's Algo Service endpoint.
///
/// Since 2025-12-09 Binance routes STOP_MARKET, TAKE_PROFIT_MARKET, and
/// TRAILING_STOP_MARKET through `/fapi/v1/algoOrder` instead of the
/// standard `/fapi/v1/order` endpoint.  The `binance` crate hardcodes the
/// old endpoint, so we bypass it with a raw signed `reqwest` POST.
#[allow(clippy::too_many_arguments)]
async fn place_algo_order(
    api_key: &str,
    api_secret: &str,
    base_url: &str,
    symbol: &str,
    side: &str,
    order_type: &str,
    stop_price: Option<f64>,
    callback_rate: Option<f64>,
) -> Result<u64, ExecutionError> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_millis();

    let mut query = format!(
        "symbol={}&side={}&type={}&closePosition=true&algoType=CONDITIONAL",
        symbol, side, order_type
    );
    if let Some(sp) = stop_price {
        query.push_str(&format!("&triggerPrice={}", sp));
    }
    if let Some(cr) = callback_rate {
        query.push_str(&format!("&callbackRate={}", cr));
    }
    query.push_str(&format!("&timestamp={}", timestamp));

    let signature = hmac_sign(&query, api_secret);

    let url = format!(
        "{}/fapi/v1/algoOrder?{}&signature={}",
        base_url, query, signature
    );

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("X-MBX-APIKEY", api_key)
        .send()
        .await
        .map_err(|e| ExecutionError::ExchangeError {
            message: format!("Algo order request failed: {}", e),
        })?;

    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ExecutionError::ExchangeError {
            message: format!("Algo order JSON parse error: {}", e),
        })?;

    if !status.is_success() {
        let code = body.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let msg = body
            .get("msg")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        return Err(ExecutionError::ExchangeError {
            message: format!("Binance algoOrder HTTP {} — code {}: {}", status, code, msg),
        });
    }

    let order_id = body
        .get("orderId")
        .or_else(|| body.get("algoId"))
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ExecutionError::ExchangeError {
            message: format!("No orderId in algoOrder response: {}", body),
        })?;

    Ok(order_id)
}

/// Cancel a single conditional (algo) order via `DELETE /fapi/v1/algoOrder`.
///
/// Best-effort: callers should log failures rather than abort, because the
/// order may already have been filled or expired.
async fn cancel_algo_order(
    api_key: &str,
    api_secret: &str,
    base_url: &str,
    symbol: &str,
    order_id: u64,
) -> Result<(), ExecutionError> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_millis();

    let query = format!(
        "symbol={}&orderId={}&timestamp={}",
        symbol, order_id, timestamp
    );
    let signature = hmac_sign(&query, api_secret);

    let url = format!(
        "{}/fapi/v1/algoOrder?{}&signature={}",
        base_url, query, signature
    );

    let client = reqwest::Client::new();
    let resp = client
        .delete(&url)
        .header("X-MBX-APIKEY", api_key)
        .send()
        .await
        .map_err(|e| ExecutionError::ExchangeError {
            message: format!("Cancel algo order request failed: {}", e),
        })?;

    let status = resp.status();
    if !status.is_success() {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let code = body.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let msg = body
            .get("msg")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        return Err(ExecutionError::ExchangeError {
            message: format!(
                "Cancel algoOrder {} HTTP {} — code {}: {}",
                order_id, status, code, msg
            ),
        });
    }

    Ok(())
}

/// Cancel **all** open conditional (algo) orders for a symbol via
/// `DELETE /fapi/v1/algoOpenOrders`.
async fn cancel_all_algo_orders(
    api_key: &str,
    api_secret: &str,
    base_url: &str,
    symbol: &str,
) -> Result<(), ExecutionError> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_millis();

    let query = format!("symbol={}&timestamp={}", symbol, timestamp);
    let signature = hmac_sign(&query, api_secret);

    let url = format!(
        "{}/fapi/v1/algoOpenOrders?{}&signature={}",
        base_url, query, signature
    );

    let client = reqwest::Client::new();
    let resp = client
        .delete(&url)
        .header("X-MBX-APIKEY", api_key)
        .send()
        .await
        .map_err(|e| ExecutionError::ExchangeError {
            message: format!("Cancel all algo orders request failed: {}", e),
        })?;

    let status = resp.status();
    if !status.is_success() {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let code = body.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let msg = body
            .get("msg")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        return Err(ExecutionError::ExchangeError {
            message: format!(
                "Cancel all algoOpenOrders HTTP {} — code {}: {}",
                status, code, msg
            ),
        });
    }

    Ok(())
}

/// Fetch open conditional (algo) orders via `GET /fapi/v1/openAlgoOrders`.
pub async fn get_open_algo_orders(
    api_key: &str,
    api_secret: &str,
    base_url: &str,
    symbol: &str,
) -> Result<String, ExecutionError> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_millis();

    // 1. Build the query string
    let query = format!("symbol={}&timestamp={}", symbol, timestamp);

    // 2. Sign the query
    let signature = hmac_sign(&query, api_secret);

    // 3. Construct the full URL
    let url = format!(
        "{}/fapi/v1/openAlgoOrders?{}&signature={}",
        base_url, query, signature
    );

    // 4. Send the GET request
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("X-MBX-APIKEY", api_key)
        .send()
        .await
        .map_err(|e| ExecutionError::ExchangeError {
            message: format!("Failed to fetch open algo orders: {}", e),
        })?;

    // 5. Return the raw JSON text so we can print it
    resp.text()
        .await
        .map_err(|e| ExecutionError::ExchangeError {
            message: format!("Failed to read response text: {}", e),
        })
}

// ─── ExecutionEngine ─────────────────────────────────────────────────────────

/// ExecutionEngine for managing live trades on Binance USD(S)-M Futures.
///
/// Trade lifecycle:
/// 1. **open_position** — MARKET entry + `STOP_MARKET` + `TAKE_PROFIT_MARKET` (closePosition=true)
/// 2. **execute_first_tp** — cancel SL/TP, partial MARKET close (reduce_only), new breakeven SL + TP
/// 3. **activate_trailing_stop** — cancel SL/TP, place `TRAILING_STOP_MARKET` (closePosition=true)
/// 4. **close_position** — cancel all known orders, MARKET reduce_only close
pub struct ExecutionEngine {
    /// Engine configuration (credentials + flags).
    config: EngineConfig,
    /// Active managed positions, keyed by symbol.
    positions: Arc<RwLock<HashMap<String, ManagedPosition>>>,
    /// Per-symbol LOT_SIZE / PRICE_FILTER rules from exchange info.
    symbol_filters: HashMap<String, SymbolFilters>,
    /// Mock mode: store positions locally without hitting the exchange.
    mock_mode: bool,
}

impl ExecutionEngine {
    /// Creates a new live `ExecutionEngine`.
    ///
    /// Fetches `/fapi/v1/exchangeInfo` to populate per-symbol LOT_SIZE and
    /// PRICE_FILTER rules.  Panics if the exchange info cannot be retrieved
    /// (fail-fast, same philosophy as the historical-data backfill).
    pub async fn new(config: EngineConfig) -> Self {
        let symbol_filters = if config.symbols.is_empty() {
            HashMap::new()
        } else {
            let cfg = config.clone();
            tokio::task::spawn_blocking(move || Self::fetch_symbol_filters(&cfg))
                .await
                .expect("[ENGINE] spawn_blocking panicked while fetching exchange info")
        };

        Self {
            config,
            positions: Arc::new(RwLock::new(HashMap::new())),
            symbol_filters,
            mock_mode: false,
        }
    }

    /// Creates a mock `ExecutionEngine` (no exchange calls).
    ///
    /// Populates `symbol_filters` with safe defaults so tests work without
    /// hitting the network.
    pub fn new_mock() -> Self {
        use rust_decimal_macros::dec;

        let mut symbol_filters = HashMap::new();
        for sym in &config::TOP_COINS {
            symbol_filters.insert(
                sym.to_string(),
                SymbolFilters {
                    step_size: dec!(0.001),
                    tick_size: dec!(0.01),
                    min_qty: dec!(0.001),
                },
            );
        }

        Self {
            config: EngineConfig::default(),
            positions: Arc::new(RwLock::new(HashMap::new())),
            symbol_filters,
            mock_mode: true,
        }
    }

    /// Queries Binance exchange info and extracts LOT_SIZE / PRICE_FILTER for
    /// every symbol listed in `config.symbols`.
    fn fetch_symbol_filters(config: &EngineConfig) -> HashMap<String, SymbolFilters> {
        let general: FuturesGeneral = if config.testnet {
            BinanceApi::new_with_config(None, None, &binance::config::Config::testnet())
        } else {
            let mainnet_cfg = binance::config::Config::default()
                .set_rest_api_endpoint("https://fapi.binance.com");
            BinanceApi::new_with_config(None, None, &mainnet_cfg)
        };

        let exchange_info = general
            .exchange_info()
            .unwrap_or_else(|e| panic!("[ENGINE] Failed to fetch exchange info: {}", e));

        let wanted: std::collections::HashSet<&str> =
            config.symbols.iter().map(|s| s.as_str()).collect();

        let mut filters_map = HashMap::new();

        for sym_info in &exchange_info.symbols {
            if !wanted.contains(sym_info.symbol.as_str()) {
                continue;
            }

            let mut step_size: Option<Decimal> = None;
            let mut tick_size: Option<Decimal> = None;
            let mut min_qty: Option<Decimal> = None;

            for f in &sym_info.filters {
                match f {
                    Filters::LotSize {
                        step_size: ss,
                        min_qty: mq,
                        ..
                    } => {
                        step_size = ss.parse::<Decimal>().ok();
                        min_qty = mq.parse::<Decimal>().ok();
                    }
                    Filters::PriceFilter { tick_size: ts, .. } => {
                        tick_size = ts.parse::<Decimal>().ok();
                    }
                    _ => {}
                }
            }

            let sf = SymbolFilters {
                step_size: step_size.unwrap_or_else(|| {
                    panic!("[ENGINE] LOT_SIZE.stepSize missing for {}", sym_info.symbol)
                }),
                tick_size: tick_size.unwrap_or_else(|| {
                    panic!(
                        "[ENGINE] PRICE_FILTER.tickSize missing for {}",
                        sym_info.symbol
                    )
                }),
                min_qty: min_qty.unwrap_or(Decimal::ZERO),
            };

            info!(
                "[ENGINE] Loaded filters for {}: step_size={} tick_size={} min_qty={}",
                sym_info.symbol, sf.step_size, sf.tick_size, sf.min_qty
            );

            filters_map.insert(sym_info.symbol.clone(), sf);
        }

        for sym in &config.symbols {
            if !filters_map.contains_key(sym) {
                error!(
                    "[ENGINE] Symbol {} not found in exchange info — orders will panic",
                    sym
                );
            }
        }

        filters_map
    }

    /// Returns the cached exchange filters for `symbol`.
    fn filters_for(&self, symbol: &str) -> &SymbolFilters {
        self.symbol_filters
            .get(symbol)
            .unwrap_or_else(|| panic!("[ENGINE] No symbol filters loaded for {}", symbol))
    }

    // ── Safety Guards ────────────────────────────────────────────────────────

    /// Rejects entry if the funding rate exceeds the configured limit.
    pub fn check_funding_rate(&self, funding: &FundingRate) -> Result<(), ExecutionError> {
        if funding.rate.abs() > self.config.funding_rate_limit {
            return Err(ExecutionError::FundingRateTooHigh {
                rate: funding.rate,
                limit: self.config.funding_rate_limit,
            });
        }
        Ok(())
    }

    /// Calculates a limit price with slippage applied.
    pub fn calculate_limit_price(&self, ticker: &PriceTicker, side: OrderSide) -> Price {
        match side {
            OrderSide::Buy => ticker.ask * (Decimal::ONE + self.config.max_slippage),
            OrderSide::Sell => ticker.bid * (Decimal::ONE - self.config.max_slippage),
        }
    }

    /// Returns `true` if the symbol is in the top-10 coins list.
    pub fn is_top_coin(symbol: &str) -> bool {
        config::TOP_COINS.contains(&symbol)
    }

    // ── Task 3: Entry ────────────────────────────────────────────────────────

    /// Opens a new Futures position:
    /// 1. MARKET order to enter
    /// 2. `STOP_MARKET` at `stop_loss` (closePosition=true)
    /// 3. `TAKE_PROFIT_MARKET` at `take_profit` (closePosition=true)
    pub async fn open_position(
        &self,
        symbol: &str,
        direction: Position,
        entry_price: Price,
        stop_loss: Price,
        take_profit: Price,
        quantity: Volume,
        atr_value: Decimal,
        funding_rate: Option<&FundingRate>,
    ) -> Result<ManagedPosition, ExecutionError> {
        // Safety: reject on excessive funding rate
        if let Some(funding) = funding_rate {
            self.check_funding_rate(funding)?;
        }

        // Build position state machine
        let mut position = ManagedPosition::new(
            symbol,
            direction,
            entry_price,
            stop_loss,
            take_profit,
            quantity,
            atr_value,
        );

        if self.mock_mode {
            let mut positions = self.positions.write().await;
            positions.insert(symbol.to_string(), position.clone());
            return Ok(position);
        }

        // ── Live path ────────────────────────────────────────────────────────
        let filters = self.filters_for(symbol);
        let sym = symbol.to_string();
        let qty_f64 = round_qty_to_step(quantity, filters.step_size);
        let sl_f64 = round_price_to_tick(stop_loss, filters.tick_size);
        let tp_f64 = round_price_to_tick(take_profit, filters.tick_size);
        let is_long = matches!(direction, Position::Long);

        // 1. Self-heal: clear both standard and conditional zombie orders
        {
            let cfg = self.config.clone();
            let sym = sym.clone();
            tokio::task::spawn_blocking(move || {
                let client = make_client(&cfg);
                match client.cancel_all_open_orders(sym.clone()) {
                    Ok(()) => info!(
                        "[ENGINE] Cleared standard open orders for {} (zombie prevention)",
                        sym
                    ),
                    Err(e) => warn!(
                        "[ENGINE] cancel_all_open_orders for {} failed (may have none): {}",
                        sym, e
                    ),
                }
            })
            .await
            .map_err(|e| ExecutionError::ExchangeError {
                message: format!("{:?}", e),
            })?;
        }

        let base_url = algo_base_url(self.config.testnet);
        match cancel_all_algo_orders(
            &self.config.api_key,
            &self.config.api_secret,
            base_url,
            &sym,
        )
        .await
        {
            Ok(()) => info!(
                "[ENGINE] Cleared algo orders for {} (zombie prevention)",
                sym
            ),
            Err(e) => warn!(
                "[ENGINE] cancel_all_algo_orders for {} failed (may have none): {}",
                sym, e
            ),
        }

        // 2. MARKET entry (sync binance crate → spawn_blocking)
        let entry_id = {
            let cfg = self.config.clone();
            let sym = sym.clone();
            tokio::task::spawn_blocking(move || {
                let client = make_client(&cfg);

                let entry_tx = if is_long {
                    client.market_buy(sym.clone(), qty_f64)
                } else {
                    client.market_sell(sym.clone(), qty_f64)
                }
                .map_err(|e| ExecutionError::ExchangeError {
                    message: format!("{:?}", e),
                })?;

                info!(
                    "[ENGINE] Entry filled for {}: order_id={} qty={}",
                    sym, entry_tx.order_id, qty_f64
                );

                Ok::<u64, ExecutionError>(entry_tx.order_id)
            })
            .await
            .map_err(|e| ExecutionError::ExchangeError {
                message: format!("{:?}", e),
            })??
        };

        // 3. STOP_MARKET + TAKE_PROFIT_MARKET via Algo Service (async reqwest)
        let close_side_str = if is_long { "SELL" } else { "BUY" };

        let sl_id = place_algo_order(
            &self.config.api_key,
            &self.config.api_secret,
            base_url,
            &sym,
            close_side_str,
            "STOP_MARKET",
            Some(sl_f64),
            None,
        )
        .await?;

        info!(
            "[ENGINE] Stop loss placed for {}: order_id={} stop_price={}",
            sym, sl_id, sl_f64
        );

        let tp_id = place_algo_order(
            &self.config.api_key,
            &self.config.api_secret,
            base_url,
            &sym,
            close_side_str,
            "TAKE_PROFIT_MARKET",
            Some(tp_f64),
            None,
        )
        .await?;

        info!(
            "[ENGINE] Take profit placed for {}: order_id={} stop_price={}",
            sym, tp_id, tp_f64
        );

        // Store Binance order IDs in the position for later modification
        position.stop_loss_order_id = Some(sl_id);
        position.take_profit_order_id = Some(tp_id);
        let _ = entry_id;

        let mut positions = self.positions.write().await;
        positions.insert(symbol.to_string(), position.clone());

        Ok(position)
    }

    // ── Monitor ──────────────────────────────────────────────────────────────

    /// Evaluates current position state and executes the appropriate action.
    pub async fn monitor_position(
        &self,
        symbol: &str,
        current_price: Price,
    ) -> Result<ManagementAction, ExecutionError> {
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
                self.execute_first_tp(symbol, *quantity_to_close, *new_stop_loss)
                    .await?;
            }
            ManagementAction::ActivateTrailingStop { offset } => {
                self.activate_trailing_stop(symbol, *offset).await?;
            }
            ManagementAction::ClosePosition { reason: _ } => {
                self.close_position(symbol).await?;
            }
            ManagementAction::None => {}
        }

        Ok(action)
    }

    // ── Task 4: Phase 2 — First TP ───────────────────────────────────────────

    /// Phase 2: 1.5R hit.
    /// - Cancels existing SL & TP
    /// - MARKET close 33% of position (`reduce_only=true`)
    /// - Places new `STOP_MARKET` at breakeven (`close_position=true`)
    /// - Places new `TAKE_PROFIT_MARKET` for remainder (`close_position=true`)
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

        // Local snapshot for the blocking closure
        let sl_id = position.stop_loss_order_id;
        let tp_id = position.take_profit_order_id;
        let direction = position.direction;
        let take_profit = position.take_profit;

        if !self.mock_mode {
            let filters = self.filters_for(symbol);
            let sym = symbol.to_string();
            let qty_f64 = round_qty_to_step(quantity_to_close, filters.step_size);
            let new_sl_f64 = round_price_to_tick(new_stop_loss, filters.tick_size);
            let new_tp_f64 = round_price_to_tick(take_profit, filters.tick_size);
            let is_long = matches!(direction, Position::Long);
            let base_url = algo_base_url(self.config.testnet);

            // 1. Cancel existing SL & TP (algo orders — async)
            if let Some(id) = sl_id {
                match cancel_algo_order(
                    &self.config.api_key,
                    &self.config.api_secret,
                    base_url,
                    &sym,
                    id,
                )
                .await
                {
                    Ok(()) => info!("[ENGINE] Cancelled SL algo order {} for {}", id, sym),
                    Err(e) => warn!("[ENGINE] Cancel SL {} failed (may be filled): {}", id, e),
                }
            }
            if let Some(id) = tp_id {
                match cancel_algo_order(
                    &self.config.api_key,
                    &self.config.api_secret,
                    base_url,
                    &sym,
                    id,
                )
                .await
                {
                    Ok(()) => info!("[ENGINE] Cancelled TP algo order {} for {}", id, sym),
                    Err(e) => warn!("[ENGINE] Cancel TP {} failed (may be filled): {}", id, e),
                }
            }

            // 2. Partial MARKET close — reduce_only=true (standard order → spawn_blocking)
            {
                let cfg = self.config.clone();
                let sym = sym.clone();
                tokio::task::spawn_blocking(move || {
                    let client = make_client(&cfg);
                    let close1 = if is_long {
                        BinanceOrderSide::Sell
                    } else {
                        BinanceOrderSide::Buy
                    };
                    let mut partial_req = blank_order(&sym, close1, FutOrderType::Market);
                    partial_req.qty = Some(qty_f64);
                    partial_req.reduce_only = Some(true);

                    let partial_tx = client.custom_order(partial_req).map_err(|e| {
                        ExecutionError::ExchangeError {
                            message: format!("{:?}", e),
                        }
                    })?;

                    info!(
                        "[ENGINE] Partial close {} for {}: order_id={}",
                        qty_f64, sym, partial_tx.order_id
                    );

                    Ok::<(), ExecutionError>(())
                })
                .await
                .map_err(|e| ExecutionError::ExchangeError {
                    message: format!("{:?}", e),
                })??;
            }

            // 3. New breakeven STOP_MARKET + TAKE_PROFIT_MARKET (algo orders — async)
            let close_side_str = if is_long { "SELL" } else { "BUY" };

            let new_sl_id = place_algo_order(
                &self.config.api_key,
                &self.config.api_secret,
                base_url,
                &sym,
                close_side_str,
                "STOP_MARKET",
                Some(new_sl_f64),
                None,
            )
            .await?;

            info!(
                "[ENGINE] Breakeven SL for {}: order_id={} stop={}",
                sym, new_sl_id, new_sl_f64
            );

            let new_tp_id = place_algo_order(
                &self.config.api_key,
                &self.config.api_secret,
                base_url,
                &sym,
                close_side_str,
                "TAKE_PROFIT_MARKET",
                Some(new_tp_f64),
                None,
            )
            .await?;

            info!(
                "[ENGINE] New TP (Phase 2) for {}: order_id={} stop={}",
                sym, new_tp_id, new_tp_f64
            );

            // Update tracked order IDs
            position.stop_loss_order_id = Some(new_sl_id);
            position.take_profit_order_id = Some(new_tp_id);
        }

        // Calculate realized P&L for the closed portion (approximation via new_stop_loss)
        let pnl_per_unit = match direction {
            Position::Long => new_stop_loss - position.entry_price,
            Position::Short => position.entry_price - new_stop_loss,
            Position::None => Decimal::ZERO,
        };
        let realized_pnl = pnl_per_unit * quantity_to_close * config::FIRST_TP_R_MULTIPLE;

        position.transition_to_first_tp(quantity_to_close, realized_pnl);

        Ok(())
    }

    // ── Task 5a: Phase 3 — Trailing Stop ─────────────────────────────────────

    /// Phase 3: 2.5R hit.
    /// - Cancels breakeven SL (and residual TP if still open)
    /// - Places `TRAILING_STOP_MARKET` with `callback_rate` derived from ATR offset
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

        if position.state == TradeState::TrailingActive {
            return Ok(()); // Already active
        }

        let sl_id = position.stop_loss_order_id;
        let tp_id = position.take_profit_order_id;
        let direction = position.direction;
        let entry_price = position.entry_price;

        let trailing_order_id = if self.mock_mode {
            12345u64 // Sentinel for tests
        } else {
            let sym = symbol.to_string();
            let is_long = matches!(direction, Position::Long);
            let base_url = algo_base_url(self.config.testnet);

            // Convert ATR-offset (price units) to Binance callback_rate percentage.
            // callback_rate % = (offset / entry_price) * 100, clamped to [0.1, 5.0].
            let cb_rate = (offset / entry_price * Decimal::from(100))
                .to_f64()
                .unwrap_or(1.0)
                .clamp(0.1, 5.0);

            info!(
                "[ENGINE] Trailing stop for {}: offset={} entry={} callback_rate={:.2}%",
                symbol, offset, entry_price, cb_rate
            );

            // 1. Cancel breakeven SL (algo order — async)
            if let Some(id) = sl_id {
                match cancel_algo_order(
                    &self.config.api_key,
                    &self.config.api_secret,
                    base_url,
                    &sym,
                    id,
                )
                .await
                {
                    Ok(()) => info!("[ENGINE] Cancelled breakeven SL {} for {}", id, sym),
                    Err(e) => warn!("[ENGINE] Cancel SL {} failed: {}", id, e),
                }
            }

            // 2. Cancel residual TP if any (algo order — async)
            if let Some(id) = tp_id {
                match cancel_algo_order(
                    &self.config.api_key,
                    &self.config.api_secret,
                    base_url,
                    &sym,
                    id,
                )
                .await
                {
                    Ok(()) => info!("[ENGINE] Cancelled TP {} for {}", id, sym),
                    Err(e) => warn!("[ENGINE] Cancel TP {} failed: {}", id, e),
                }
            }

            // 3. Place TRAILING_STOP_MARKET via Algo Service (async)
            let close_side_str = if is_long { "SELL" } else { "BUY" };

            let trail_id = place_algo_order(
                &self.config.api_key,
                &self.config.api_secret,
                base_url,
                &sym,
                close_side_str,
                "TRAILING_STOP_MARKET",
                None,
                Some(cb_rate),
            )
            .await?;

            info!(
                "[ENGINE] Trailing stop placed for {}: order_id={} callback={}%",
                sym, trail_id, cb_rate
            );

            trail_id
        };

        position.transition_to_trailing(trailing_order_id);

        Ok(())
    }

    // ── Task 5b: Emergency Close ──────────────────────────────────────────────

    /// Closes the position immediately:
    /// - Cancels all tracked orders (best-effort, ignores errors)
    /// - MARKET order with `reduce_only=true` for the remaining quantity
    pub async fn close_position(&self, symbol: &str) -> Result<(), ExecutionError> {
        let mut positions = self.positions.write().await;

        let position = positions
            .get_mut(symbol)
            .ok_or(ExecutionError::PositionNotFound {
                symbol: symbol.to_string(),
            })?;

        if !position.is_active() {
            return Ok(()); // Already closed
        }

        if !self.mock_mode {
            let filters = self.filters_for(symbol);
            let sym = symbol.to_string();
            let sl_id = position.stop_loss_order_id;
            let tp_id = position.take_profit_order_id;
            let trail_id = position.trailing_stop_order_id;
            let is_long = matches!(position.direction, Position::Long);
            let remaining_qty = round_qty_to_step(position.current_quantity, filters.step_size);
            let base_url = algo_base_url(self.config.testnet);

            // 1. Cancel all known conditional orders (best-effort, algo API)
            for id in [sl_id, tp_id, trail_id].into_iter().flatten() {
                match cancel_algo_order(
                    &self.config.api_key,
                    &self.config.api_secret,
                    base_url,
                    &sym,
                    id,
                )
                .await
                {
                    Ok(()) => info!("[ENGINE] Cancelled algo order {} for {}", id, sym),
                    Err(e) => warn!(
                        "[ENGINE] Cancel {} failed (possibly already closed): {}",
                        id, e
                    ),
                }
            }

            // 2. MARKET close with reduce_only=true (standard order → spawn_blocking)
            {
                let cfg = self.config.clone();
                let sym = sym.clone();
                let c_is_long = is_long;
                tokio::task::spawn_blocking(move || {
                    let client = make_client(&cfg);

                    let c_side = if c_is_long {
                        BinanceOrderSide::Sell
                    } else {
                        BinanceOrderSide::Buy
                    };
                    let mut close_req = blank_order(&sym, c_side, FutOrderType::Market);
                    close_req.qty = Some(remaining_qty);
                    close_req.reduce_only = Some(true);

                    let close_tx = client.custom_order(close_req).map_err(|e| {
                        ExecutionError::ExchangeError {
                            message: format!("{:?}", e),
                        }
                    })?;

                    info!(
                        "[ENGINE] Emergency close for {}: order_id={} qty={}",
                        sym, close_tx.order_id, remaining_qty
                    );

                    Ok::<(), ExecutionError>(())
                })
                .await
                .map_err(|e| ExecutionError::ExchangeError {
                    message: format!("{:?}", e),
                })??;
            }
        }

        position.close(Decimal::ZERO);

        Ok(())
    }

    // ── Query Helpers ─────────────────────────────────────────────────────────

    /// Returns a snapshot of the position for the given symbol, if any.
    pub async fn get_position(&self, symbol: &str) -> Option<ManagedPosition> {
        let positions = self.positions.read().await;
        positions.get(symbol).cloned()
    }

    /// Returns all currently active positions.
    pub async fn get_all_positions(&self) -> Vec<ManagedPosition> {
        let positions = self.positions.read().await;
        positions
            .values()
            .filter(|p| p.is_active())
            .cloned()
            .collect()
    }

    pub async fn print_active_algo_orders(&self, symbol: &str) {
        if self.mock_mode {
            return;
        }

        let base_url = if self.config.testnet {
            "https://testnet.binancefuture.com"
        } else {
            "https://fapi.binance.com"
        };

        match get_open_algo_orders(
            &self.config.api_key,
            &self.config.api_secret,
            base_url,
            symbol,
        )
        .await
        {
            Ok(json) => log::info!("[VERIFICATION] Open Algo Orders for {}: {}", symbol, json),
            Err(e) => log::error!("[VERIFICATION] Failed to fetch algo orders: {}", e),
        }
    }

    /// Removes a position from the tracker (call after it is confirmed closed).
    pub async fn remove_position(&self, symbol: &str) -> Option<ManagedPosition> {
        let mut positions = self.positions.write().await;
        positions.remove(symbol)
    }
}

// ─── Tests (all use mock mode) ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[tokio::test]
    async fn test_open_position_mock() {
        let engine = ExecutionEngine::new_mock();

        let position = engine
            .open_position(
                "BTCUSDT",
                Position::Long,
                dec!(50000),
                dec!(49000),
                dec!(53000),
                dec!(1),
                dec!(500),
                None,
            )
            .await
            .unwrap();

        assert_eq!(position.symbol, "BTCUSDT");
        assert_eq!(position.state, TradeState::Open);
    }

    #[tokio::test]
    async fn test_funding_rate_check() {
        let engine = ExecutionEngine::new_mock();

        let acceptable = FundingRate {
            symbol: "BTCUSDT".to_string(),
            rate: dec!(0.0002),
            next_funding_time: 0,
        };

        assert!(engine.check_funding_rate(&acceptable).is_ok());

        let too_high = FundingRate {
            symbol: "BTCUSDT".to_string(),
            rate: dec!(0.0005),
            next_funding_time: 0,
        };

        assert!(matches!(
            engine.check_funding_rate(&too_high),
            Err(ExecutionError::FundingRateTooHigh { .. })
        ));
    }

    #[tokio::test]
    async fn test_calculate_limit_price() {
        let engine = ExecutionEngine::new_mock();

        let ticker = PriceTicker {
            symbol: "BTCUSDT".to_string(),
            price: dec!(50000),
            bid: dec!(49990),
            ask: dec!(50010),
        };

        // Buy: ask + 0.2% = 50010 * 1.002 = 50110.02
        let buy_price = engine.calculate_limit_price(&ticker, OrderSide::Buy);
        assert_eq!(buy_price, dec!(50110.02));

        // Sell: bid - 0.2% = 49990 * 0.998 = 49890.02
        let sell_price = engine.calculate_limit_price(&ticker, OrderSide::Sell);
        assert_eq!(sell_price, dec!(49890.02));
    }

    #[tokio::test]
    async fn test_monitor_position_first_tp() {
        let engine = ExecutionEngine::new_mock();

        engine
            .open_position(
                "BTCUSDT",
                Position::Long,
                dec!(50000),
                dec!(49000),
                dec!(53000),
                dec!(1),
                dec!(500),
                None,
            )
            .await
            .unwrap();

        let action = engine
            .monitor_position("BTCUSDT", dec!(51500))
            .await
            .unwrap();

        match action {
            ManagementAction::ExecuteFirstTp { .. } => {}
            _ => panic!("Expected ExecuteFirstTp action"),
        }

        let position = engine.get_position("BTCUSDT").await.unwrap();
        assert_eq!(position.state, TradeState::FirstTpHit);
    }

    #[tokio::test]
    async fn test_monitor_position_trailing() {
        let engine = ExecutionEngine::new_mock();

        engine
            .open_position(
                "BTCUSDT",
                Position::Long,
                dec!(50000),
                dec!(49000),
                dec!(53000),
                dec!(1),
                dec!(500),
                None,
            )
            .await
            .unwrap();

        let action = engine
            .monitor_position("BTCUSDT", dec!(52500))
            .await
            .unwrap();

        match action {
            ManagementAction::ActivateTrailingStop { offset } => {
                assert_eq!(offset, dec!(1000)); // 500 * 2.0
            }
            _ => panic!("Expected ActivateTrailingStop action"),
        }

        let position = engine.get_position("BTCUSDT").await.unwrap();
        assert_eq!(position.state, TradeState::TrailingActive);
    }

    #[tokio::test]
    async fn test_close_position() {
        let engine = ExecutionEngine::new_mock();

        engine
            .open_position(
                "BTCUSDT",
                Position::Long,
                dec!(50000),
                dec!(49000),
                dec!(53000),
                dec!(1),
                dec!(500),
                None,
            )
            .await
            .unwrap();

        engine.close_position("BTCUSDT").await.unwrap();

        let position = engine.get_position("BTCUSDT").await.unwrap();
        assert_eq!(position.state, TradeState::Closed);
        assert!(!position.is_active());
    }

    #[test]
    fn test_is_top_coin() {
        assert!(ExecutionEngine::is_top_coin("BTCUSDT"));
        assert!(ExecutionEngine::is_top_coin("ETHUSDT"));
        assert!(!ExecutionEngine::is_top_coin("OBSCUREUSDT"));
    }

    // ── Short position tests ──

    #[tokio::test]
    async fn test_short_open_position_mock() {
        let engine = ExecutionEngine::new_mock();

        let position = engine
            .open_position(
                "BTCUSDT",
                Position::Short,
                dec!(50000),
                dec!(51000),
                dec!(47000),
                dec!(1),
                dec!(500),
                None,
            )
            .await
            .unwrap();

        assert_eq!(position.symbol, "BTCUSDT");
        assert_eq!(position.direction, Position::Short);
        assert_eq!(position.state, TradeState::Open);
    }

    #[tokio::test]
    async fn test_short_monitor_position_first_tp() {
        let engine = ExecutionEngine::new_mock();

        engine
            .open_position(
                "BTCUSDT",
                Position::Short,
                dec!(50000),
                dec!(51000),
                dec!(47000),
                dec!(1),
                dec!(500),
                None,
            )
            .await
            .unwrap();

        let action = engine
            .monitor_position("BTCUSDT", dec!(48500))
            .await
            .unwrap();

        match action {
            ManagementAction::ExecuteFirstTp { .. } => {}
            _ => panic!("Expected ExecuteFirstTp action for Short"),
        }

        let position = engine.get_position("BTCUSDT").await.unwrap();
        assert_eq!(position.state, TradeState::FirstTpHit);
    }

    #[tokio::test]
    async fn test_short_monitor_position_trailing() {
        let engine = ExecutionEngine::new_mock();

        engine
            .open_position(
                "BTCUSDT",
                Position::Short,
                dec!(50000),
                dec!(51000),
                dec!(47000),
                dec!(1),
                dec!(500),
                None,
            )
            .await
            .unwrap();

        let action = engine
            .monitor_position("BTCUSDT", dec!(47500))
            .await
            .unwrap();

        match action {
            ManagementAction::ActivateTrailingStop { offset } => {
                assert_eq!(offset, dec!(1000)); // 500 * 2.0
            }
            _ => panic!("Expected ActivateTrailingStop action for Short"),
        }

        let position = engine.get_position("BTCUSDT").await.unwrap();
        assert_eq!(position.state, TradeState::TrailingActive);
    }
}
