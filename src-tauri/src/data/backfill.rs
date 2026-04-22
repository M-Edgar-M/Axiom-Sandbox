//! Historical data backfill using Binance Futures REST API.
//!
//! Fetches recent candles to pre-populate MarketData buffers,
//! solving the cold-start problem for EMA/indicator calculation.

use log::{info, warn};

use binance::api::Binance;
use binance::futures::market::FuturesMarket;
use binance::model::KlineSummaries;

use crate::types::kline_summary_to_candle;
use crate::types::{Candle, Interval};

/// Total number of historical candles to fetch per symbol/interval.
const BACKFILL_TARGET: usize = 10_000;

/// Maximum candles Binance allows per single klines request.
const BINANCE_KLINES_LIMIT: u16 = 1500;

/// Fetches historical kline data from the Binance Futures REST API.
///
/// Paginates backwards in time using successive `endTime` parameters because
/// Binance caps `get_klines` at 1,500 candles per request and we need 10,000.
///
/// Algorithm:
/// 1. Request the most-recent 1,500 candles (no `endTime`).
/// 2. Record `oldest_open_time = batch[0].open_time - 1 ms`.
/// 3. Repeat, passing `oldest_open_time` as `endTime`, until we have 10,000 candles.
/// 4. Reverse the accumulated vector so it is oldest-first (chronological order).
///
/// Uses `tokio::task::spawn_blocking` because the `binance` crate's
/// `FuturesMarket::get_klines` is synchronous (uses `reqwest::blocking`).
pub async fn fetch_historical_data(
    symbol: &str,
    interval: Interval,
) -> Result<Vec<Candle>, String> {
    let symbol_owned = symbol.to_string();
    let interval_str = interval.as_binance_str().to_string();

    // Accumulated candles collected in reverse-chronological batches.
    // We push the most-recent batch first, then older batches on top.
    let mut all_candles: Vec<Candle> = Vec::with_capacity(BACKFILL_TARGET);

    // `end_time_ms` is the exclusive upper bound for each successive request.
    // `None` means "fetch the most-recent candles" for the first request.
    let mut end_time_ms: Option<u64> = None;

    let max_attempts = 3;

    while all_candles.len() < BACKFILL_TARGET {
        let symbol_clone = symbol_owned.clone();
        let interval_clone = interval_str.clone();

        // ── Single paginated request (with retry) ──────────────────────────
        let mut batch: Vec<Candle> = Vec::new();
        let mut last_error = String::new();

        for attempt in 0..max_attempts {
            let sym = symbol_clone.clone();
            let ivl = interval_clone.clone();
            let end = end_time_ms;

            let result = tokio::task::spawn_blocking(move || {
                let market: FuturesMarket = Binance::new(None, None);
                market.get_klines(&sym, &ivl, BINANCE_KLINES_LIMIT, None::<u64>, end)
            })
            .await
            .map_err(|e| format!("Task join error: {}", e))?;

            match result {
                Ok(KlineSummaries::AllKlineSummaries(summaries)) => {
                    batch = summaries
                        .iter()
                        .filter_map(kline_summary_to_candle)
                        .collect();
                    break; // success — exit retry loop
                }
                Err(e) => {
                    last_error = format!("Binance API error: {}", e);
                    warn!(
                        "[BACKFILL] Attempt {}/{} failed for {} {}: {}",
                        attempt + 1,
                        max_attempts,
                        symbol,
                        interval.as_binance_str(),
                        last_error
                    );
                    if attempt + 1 < max_attempts {
                        tokio::time::sleep(std::time::Duration::from_secs(
                            2u64.pow(attempt as u32 + 1),
                        ))
                        .await;
                    }
                }
            }
        }

        if batch.is_empty() {
            // All retries exhausted with no data — abort.
            return Err(last_error);
        }

        // The batch arrives oldest-first from Binance.
        // `batch[0].open_time` is the oldest candle in this page.
        // Set next endTime to one millisecond before that so the next request
        // fetches the next older page without overlap.
        let oldest_open_ms = batch[0].timestamp.timestamp_millis() as u64;
        end_time_ms = Some(oldest_open_ms.saturating_sub(1));

        let fetched = batch.len();
        info!(
            "[BACKFILL] Fetched {} candles for {} {} (total so far: {})",
            fetched,
            symbol,
            interval.as_binance_str(),
            all_candles.len() + fetched,
        );

        // Prepend this (older) batch in front by extending then rotating, or
        // simply accumulate in reverse order and reverse the whole vector once.
        // We accumulate with newest-first: first batch = most recent candles,
        // second batch = older candles appended after, then we reverse at end.
        all_candles.extend(batch);

        // If Binance returned fewer candles than requested, we've hit the
        // exchange's available history limit — stop early.
        if fetched < BINANCE_KLINES_LIMIT as usize {
            info!(
                "[BACKFILL] Hit start of available history after {} candles for {} {}",
                all_candles.len(),
                symbol,
                interval.as_binance_str(),
            );
            break;
        }
    }

    // Trim to exactly BACKFILL_TARGET if we overshot.
    if all_candles.len() > BACKFILL_TARGET {
        all_candles.truncate(BACKFILL_TARGET);
    }

    // The vector is currently newest-first (most recent batch at index 0,
    // older batches appended after). Reverse to get chronological order.
    all_candles.reverse();

    info!(
        "[BACKFILL] Complete: {} candles for {} {}",
        all_candles.len(),
        symbol,
        interval.as_binance_str(),
    );

    Ok(all_candles)
}
