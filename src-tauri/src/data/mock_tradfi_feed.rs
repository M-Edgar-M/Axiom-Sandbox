//! Mock TradFi data feed — CSV replay for dry-run simulation.
//!
//! Reads a standard OHLCV CSV file row-by-row and drips candles over an
//! `mpsc::Sender<Candle>` at a configurable pace, simulating the tick-by-tick
//! flow of a live market data connection.
//!
//! ## Expected CSV format
//! Standard 1H OHLCV with a header row (column order is flexible — header
//! names are matched case-insensitively):
//!
//! ```csv
//! datetime,open,high,low,close,volume
//! 2020-01-01 00:00:00,60.00,62.50,59.10,61.80,45123
//! ```
//!
//! Supported datetime formats (tried in order):
//! - `%Y-%m-%d %H:%M:%S`
//! - `%Y-%m-%dT%H:%M:%S`
//! - `%Y-%m-%d` (date-only → midnight UTC)
//! - Unix timestamp (integer seconds)
//!
//! ## Usage
//! ```ignore
//! let feed = MockCsvFeed::new("data/WTI_Crude_Oil_1h_historical.csv", "CL=F");
//! let rx = feed.start_dripping(Duration::from_millis(10)).await;
//!
//! while let Some(candle) = rx.recv().await {
//!     // feed into AnalysisEngine …
//! }
//! ```

use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use log::{error, info, warn};
use rust_decimal::Decimal;
use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::types::Candle;

/// Drip-feed pace: default delay between consecutive simulated candles.
pub const DEFAULT_DRIP_DELAY_MS: u64 = 10;

/// Maximum channel buffer before the producer applies back-pressure.
const CHANNEL_CAPACITY: usize = 1024;

// ─── Column index map ─────────────────────────────────────────────────────────

/// Resolved column indices after parsing the CSV header.
#[derive(Debug)]
struct ColumnMap {
    timestamp: usize,
    open: usize,
    high: usize,
    low: usize,
    close: usize,
    volume: usize,
}

impl ColumnMap {
    /// Build a `ColumnMap` by matching header names case-insensitively.
    fn from_header(record: &csv::StringRecord) -> Result<Self, String> {
        let find = |names: &[&str]| -> Result<usize, String> {
            for (i, field) in record.iter().enumerate() {
                let f = field.trim().to_lowercase();
                if names.iter().any(|n| *n == f) {
                    return Ok(i);
                }
            }
            Err(format!(
                "Could not find a column matching {:?} in header: {:?}",
                names,
                record.iter().collect::<Vec<_>>()
            ))
        };

        Ok(Self {
            // Accept "datetime", "date", "time", "timestamp", "open_time"
            timestamp: find(&["datetime", "date", "time", "timestamp", "open_time"])?,
            open: find(&["open", "o"])?,
            high: find(&["high", "h"])?,
            low: find(&["low", "l"])?,
            close: find(&["close", "c"])?,
            volume: find(&["volume", "vol", "v"])?,
        })
    }
}

// ─── Timestamp parsing ────────────────────────────────────────────────────────

/// Attempt to parse a timestamp string into `DateTime<Utc>` using multiple
/// common formats.  Falls back to Unix-second integer parsing as a last resort.
fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();

    // 0. Timezone-aware datetime (e.g. "2021-03-04 17:00:00+00:00")
    if let Ok(dt) = DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%:z") {
        return Some(dt.with_timezone(&Utc));
    }

    // 1. ISO-like datetime with space separator
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(Utc.from_utc_datetime(&ndt));
    }

    // 2. ISO 8601 with T separator (with or without trailing Z)
    let iso = s.trim_end_matches('Z');
    if let Ok(ndt) = NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S") {
        return Some(Utc.from_utc_datetime(&ndt));
    }

    // 3. Date-only → midnight UTC
    if let Ok(nd) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(Utc.from_utc_datetime(&nd.and_hms_opt(0, 0, 0)?));
    }

    // 4. Unix timestamp (seconds)
    if let Ok(secs) = s.parse::<i64>() {
        return DateTime::from_timestamp(secs, 0);
    }

    // 5. Unix timestamp (milliseconds — larger values)
    if let Ok(ms) = s.parse::<i64>() {
        return DateTime::from_timestamp_millis(ms);
    }

    None
}

// ─── MockCsvFeed ─────────────────────────────────────────────────────────────

/// Reads a historical OHLCV CSV and replays candles into an async channel.
///
/// Each row is converted to a `Candle` and sent with a configurable delay
/// so downstream engines (AnalysisEngine + IbkrExecutionEngine) can process
/// the data at a realistic pace during dry-run simulations.
pub struct MockCsvFeed {
    /// Path to the CSV file.
    csv_path: PathBuf,
    /// Symbol name to stamp on every generated candle.
    symbol: String,
}

impl MockCsvFeed {
    /// Creates a new `MockCsvFeed`.
    ///
    /// # Arguments
    /// * `csv_path`  — Path to the OHLCV CSV file (relative or absolute).
    /// * `symbol`    — Symbol name to embed in log messages (e.g. `"CL=F"`).
    pub fn new(csv_path: impl Into<PathBuf>, symbol: impl Into<String>) -> Self {
        Self {
            csv_path: csv_path.into(),
            symbol: symbol.into(),
        }
    }

    /// Spawns a background Tokio task that reads the CSV and sends candles.
    ///
    /// Returns the receiving end of the channel. When all rows have been sent
    /// (or an error occurs), the sender is dropped and the channel closes,
    /// causing the receiver to return `None` on the next `.recv()`.
    ///
    /// # Arguments
    /// * `delay` — Sleep duration between consecutive candles (default: 10 ms).
    pub async fn start_dripping(self, delay: Duration) -> mpsc::Receiver<Candle> {
        let (tx, rx) = mpsc::channel::<Candle>(CHANNEL_CAPACITY);
        let symbol = self.symbol.clone();
        let path = self.csv_path.clone();

        tokio::spawn(async move {
            info!(
                "[MOCK FEED] Starting CSV replay: path={:?} symbol={} delay={:?}",
                path, symbol, delay
            );

            // ── Open and parse the CSV ────────────────────────────────────
            let mut reader = match csv::Reader::from_path(&path) {
                Ok(r) => r,
                Err(e) => {
                    error!("[MOCK FEED] Failed to open CSV {:?}: {}", path, e);
                    return;
                }
            };

            // Build column map from the header record.
            let col_map = {
                let headers = match reader.headers() {
                    Ok(h) => h.clone(),
                    Err(e) => {
                        error!("[MOCK FEED] Failed to read CSV headers: {}", e);
                        return;
                    }
                };
                match ColumnMap::from_header(&headers) {
                    Ok(m) => m,
                    Err(e) => {
                        error!("[MOCK FEED] {}", e);
                        return;
                    }
                }
            };

            info!("[MOCK FEED] Header mapped: {:?}", col_map);

            // ── Drip rows ─────────────────────────────────────────────────
            let mut total: u64 = 0;
            let mut skipped: u64 = 0;

            for result in reader.records() {
                let record = match result {
                    Ok(r) => r,
                    Err(e) => {
                        warn!("[MOCK FEED] Skipping malformed row: {}", e);
                        skipped += 1;
                        continue;
                    }
                };

                // Parse each OHLCV field; skip the row on any parse failure.
                let candle = (|| -> Option<Candle> {
                    let ts_str = record.get(col_map.timestamp)?;
                    let timestamp = parse_timestamp(ts_str)?;

                    let open: Decimal = record.get(col_map.open)?.trim().parse().ok()?;
                    let high: Decimal = record.get(col_map.high)?.trim().parse().ok()?;
                    let low: Decimal = record.get(col_map.low)?.trim().parse().ok()?;
                    let close: Decimal = record.get(col_map.close)?.trim().parse().ok()?;
                    let volume: Decimal = record.get(col_map.volume)?.trim().parse().ok()?;

                    Some(Candle::new(timestamp, open, high, low, close, volume))
                })();

                match candle {
                    Some(c) => {
                        // Back-pressure: if the receiver is full, the send will
                        // block here until space is available.
                        if tx.send(c).await.is_err() {
                            info!("[MOCK FEED] Receiver dropped — stopping replay.");
                            return;
                        }
                        total += 1;

                        // Inter-candle delay — simulates live market pacing.
                        if !delay.is_zero() {
                            sleep(delay).await;
                        }
                    }
                    None => {
                        warn!(
                            "[MOCK FEED] Skipping unparseable row #{}: {:?}",
                            total + skipped + 1,
                            record
                        );
                        skipped += 1;
                    }
                }
            }

            info!(
                "[MOCK FEED] Replay complete: {} candles sent, {} rows skipped.",
                total, skipped
            );
        });

        rx
    }
}
