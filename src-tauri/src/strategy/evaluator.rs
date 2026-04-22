//! Rule Evaluator — dynamically evaluates user-defined JSON strategy conditions
//! against live `MarketData` using the `yata` technical analysis library.
//!
//! ## Design
//! - **Stateless per call**: every `evaluate()` call recomputes indicators from
//!   the full candle series. This keeps the evaluator simple and correct even
//!   when candles are backfilled or reordered.
//! - **AND logic**: all `entry_rules` must evaluate to `true` simultaneously.
//! - **Precision**: `rust_decimal` candle prices are downcast to `f64` only
//!   inside this module, purely for `yata` compatibility.  All order sizing
//!   stays in `Decimal` in the exchange layer.

use yata::methods::{EMA, SMA};
use yata::prelude::Method;

use crate::types::{Candle, Interval, MarketData};

use super::models::{
    EntryRule, MaCondition, MaRule, MaType, RsiCondition, RsiRule, UserStrategyConfig, VolumeRule,
};

// ─────────────────────────────────────────────────────────────────────────────
// Inline Wilder's RSI
// ─────────────────────────────────────────────────────────────────────────────
// yata 0.6's RSI lives in `indicators` and requires a full Candle, not an f64
// slice. We implement Wilder's smoothed RSI here directly — identical math,
// no external coupling.

/// Computes Wilder's RSI over a `f64` slice, returning the last two values.
///
/// Returns `(penultimate_rsi, ultimate_rsi)` so crossover detection works.
/// Requires `closes.len() >= period + 2`.
fn wilder_rsi_last_two(closes: &[f64], period: usize) -> (f64, f64) {
    debug_assert!(closes.len() >= period + 2, "caller must guarantee enough data");

    // Seed: first `period` candles form the initial average gain/loss.
    let mut avg_gain: f64 = 0.0;
    let mut avg_loss: f64 = 0.0;

    for i in 1..=period {
        let diff = closes[i] - closes[i - 1];
        if diff > 0.0 {
            avg_gain += diff;
        } else {
            avg_loss += diff.abs();
        }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;

    let rsi_from = |ag: f64, al: f64| -> f64 {
        if al == 0.0 {
            return 100.0;
        }
        100.0 - 100.0 / (1.0 + ag / al)
    };

    // Roll forward, tracking the second-to-last and last values.
    let mut prev = rsi_from(avg_gain, avg_loss);
    let mut curr = prev;

    for i in (period + 1)..closes.len() {
        let diff = closes[i] - closes[i - 1];
        let gain = if diff > 0.0 { diff } else { 0.0 };
        let loss = if diff < 0.0 { diff.abs() } else { 0.0 };

        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;

        prev = curr;
        curr = rsi_from(avg_gain, avg_loss);
    }

    (prev, curr)
}

// ─────────────────────────────────────────────────────────────────────────────
// Error type
// ─────────────────────────────────────────────────────────────────────────────

/// Errors that can occur during rule evaluation.
#[derive(Debug, thiserror::Error)]
pub enum EvaluatorError {
    #[error("insufficient candle data: need at least {required} candles on {interval:?}, got {got}")]
    InsufficientData {
        required: usize,
        got: usize,
        interval: Interval,
    },

    #[error("indicator initialisation failed for {indicator}: {source}")]
    IndicatorInit {
        indicator: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("strategy config is invalid: {0:?}")]
    InvalidConfig(Vec<String>),
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helper: extract f64 closes + volumes from a candle slice
// ─────────────────────────────────────────────────────────────────────────────

struct Series {
    closes: Vec<f64>,
    volumes: Vec<f64>,
}

impl Series {
    fn from_candles(candles: &[Candle]) -> Self {
        let closes = candles
            .iter()
            .map(|c| {
                c.close
                    .try_into()
                    .unwrap_or_else(|_| f64::from_bits(c.close.mantissa() as u64))
            })
            .collect();
        let volumes = candles
            .iter()
            .map(|v| {
                v.volume
                    .try_into()
                    .unwrap_or_else(|_| f64::from_bits(v.volume.mantissa() as u64))
            })
            .collect();
        Self { closes, volumes }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RuleEvaluator
// ─────────────────────────────────────────────────────────────────────────────

/// Evaluates a [`UserStrategyConfig`] against live [`MarketData`].
///
/// ## Usage
/// ```ignore
/// let evaluator = RuleEvaluator::new();
/// let signal = evaluator.evaluate(&market_data, &config)?;
/// if signal { /* place entry */ }
/// ```
#[derive(Debug, Default)]
pub struct RuleEvaluator;

impl RuleEvaluator {
    /// Creates a new stateless evaluator.
    pub fn new() -> Self {
        Self
    }

    /// Evaluates all entry rules in `config` against `market_data`.
    ///
    /// Returns `Ok(true)` when **all** conditions are satisfied (AND logic),
    /// `Ok(false)` when at least one condition fails,
    /// and `Err(EvaluatorError)` when data is insufficient or the config is invalid.
    pub fn evaluate(
        &self,
        market_data: &MarketData,
        config: &UserStrategyConfig,
    ) -> Result<bool, EvaluatorError> {
        // Validate config at the boundary before any heavy computation.
        let errors = config.validate();
        if !errors.is_empty() {
            return Err(EvaluatorError::InvalidConfig(errors));
        }

        for rule in &config.entry_rules {
            let passed = match rule {
                EntryRule::Rsi(r) => self.evaluate_rsi(r, market_data)?,
                EntryRule::Ma(r) => self.evaluate_ma(r, market_data)?,
                EntryRule::Volume(r) => self.evaluate_volume(r, market_data)?,
            };
            if !passed {
                // Short-circuit: fail fast on first false condition.
                return Ok(false);
            }
        }

        Ok(true)
    }

    // ── RSI ──────────────────────────────────────────────────────────────────

    fn evaluate_rsi(
        &self,
        rule: &RsiRule,
        market_data: &MarketData,
    ) -> Result<bool, EvaluatorError> {
        // Need at least `period + 2` candles: `period` to seed the initial avg,
        // +1 for the second-to-last RSI value (crossover detection), +1 for the last.
        let required = rule.lookback + 2;
        let candles = market_data.candles(rule.interval);

        if candles.len() < required {
            return Err(EvaluatorError::InsufficientData {
                required,
                got: candles.len(),
                interval: rule.interval,
            });
        }

        let series = Series::from_candles(candles);
        let (prev, curr) = wilder_rsi_last_two(&series.closes, rule.lookback);

        let threshold = rule.threshold;
        Ok(match rule.condition {
            RsiCondition::IsBelow => curr < threshold,
            RsiCondition::IsAbove => curr > threshold,
            RsiCondition::CrossesAbove => prev < threshold && curr >= threshold,
            RsiCondition::CrossesBelow => prev > threshold && curr <= threshold,
        })
    }

    // ── Moving Average ────────────────────────────────────────────────────────

    fn evaluate_ma(
        &self,
        rule: &MaRule,
        market_data: &MarketData,
    ) -> Result<bool, EvaluatorError> {
        let candles = market_data.candles(rule.interval);
        let required = rule.slow_lookback.unwrap_or(rule.lookback) + 2;

        if candles.len() < required {
            return Err(EvaluatorError::InsufficientData {
                required,
                got: candles.len(),
                interval: rule.interval,
            });
        }

        let series = Series::from_candles(candles);
        let closes = &series.closes;
        let last_close = *closes.last().unwrap();
        let prev_close = closes[closes.len() - 2];

        match rule.condition {
            // ── Price vs single MA ────────────────────────────────────────────
            MaCondition::PriceCrossesAbove
            | MaCondition::PriceCrossesBelow
            | MaCondition::PriceIsAbove
            | MaCondition::PriceIsBelow => {
                let (prev_ma, curr_ma) =
                    self.compute_ma_last_two(closes, rule.lookback, rule.ma_type)?;

                Ok(match rule.condition {
                    MaCondition::PriceIsAbove => last_close > curr_ma,
                    MaCondition::PriceIsBelow => last_close < curr_ma,
                    MaCondition::PriceCrossesAbove => {
                        prev_close <= prev_ma && last_close > curr_ma
                    }
                    MaCondition::PriceCrossesBelow => {
                        prev_close >= prev_ma && last_close < curr_ma
                    }
                    _ => unreachable!(),
                })
            }

            // ── Fast MA vs slow MA crossover ──────────────────────────────────
            MaCondition::FastCrossesSlow | MaCondition::FastCrossesBelow => {
                let slow = rule.slow_lookback.ok_or_else(|| {
                    EvaluatorError::InvalidConfig(vec![
                        "slow_lookback required for FastCrosses* conditions".to_string(),
                    ])
                })?;

                let (prev_fast, curr_fast) =
                    self.compute_ma_last_two(closes, rule.lookback, rule.ma_type)?;
                let (prev_slow, curr_slow) =
                    self.compute_ma_last_two(closes, slow, rule.ma_type)?;

                Ok(match rule.condition {
                    MaCondition::FastCrossesSlow => prev_fast <= prev_slow && curr_fast > curr_slow,
                    MaCondition::FastCrossesBelow => {
                        prev_fast >= prev_slow && curr_fast < curr_slow
                    }
                    _ => unreachable!(),
                })
            }
        }
    }

    /// Runs a full MA pass over `closes` and returns the **last two** values.
    ///
    /// Returns `(penultimate, ultimate)`.
    fn compute_ma_last_two(
        &self,
        closes: &[f64],
        period: usize,
        ma_type: MaType,
    ) -> Result<(f64, f64), EvaluatorError> {
        let p = period as u8;
        let seed = closes[0];

        let mut prev = seed;
        let mut curr = seed;

        match ma_type {
            MaType::Sma => {
                let mut ma =
                    SMA::new(p, &seed).map_err(|e| EvaluatorError::IndicatorInit {
                        indicator: "SMA",
                        source: e.into(),
                    })?;
                for &close in &closes[1..] {
                    prev = curr;
                    curr = ma.next(&close);
                }
            }
            MaType::Ema => {
                let mut ma =
                    EMA::new(p, &seed).map_err(|e| EvaluatorError::IndicatorInit {
                        indicator: "EMA",
                        source: e.into(),
                    })?;
                for &close in &closes[1..] {
                    prev = curr;
                    curr = ma.next(&close);
                }
            }
        }

        Ok((prev, curr))
    }

    // ── Volume ────────────────────────────────────────────────────────────────

    fn evaluate_volume(
        &self,
        rule: &VolumeRule,
        market_data: &MarketData,
    ) -> Result<bool, EvaluatorError> {
        let required = rule.lookback + 1;
        let candles = market_data.candles(rule.interval);

        if candles.len() < required {
            return Err(EvaluatorError::InsufficientData {
                required,
                got: candles.len(),
                interval: rule.interval,
            });
        }

        let series = Series::from_candles(candles);

        // Current volume is the last element; baseline is computed on the window
        // BEFORE the current candle (so the spike candle itself doesn't inflate the average).
        let current_volume = *series.volumes.last().unwrap();
        let baseline_window = &series.volumes[..series.volumes.len() - 1];

        // Take only the last `lookback` candles for the SMA window.
        let window_start = baseline_window.len().saturating_sub(rule.lookback);
        let window = &baseline_window[window_start..];

        let baseline_avg: f64 = window.iter().sum::<f64>() / window.len() as f64;
        let threshold = baseline_avg * rule.multiplier;

        Ok(current_volume > threshold)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use rust_decimal_macros::dec;

    use crate::types::{Candle, Interval, MarketData};

    use super::super::models::{
        EntryRule, MaCondition, MaRule, MaType, RiskParams, RsiCondition, RsiRule,
        UserStrategyConfig, VolumeRule,
    };
    use super::*;

    /// Build a MarketData with `n` synthetic 1H candles.
    /// Prices gently rise from 100.0 to capture a golden-cross / RSI movement.
    fn synthetic_market_data(n: usize) -> MarketData {
        let mut md = MarketData::new("BTCUSDT");
        for i in 0..n {
            let base = 100.0 + i as f64 * 0.5;
            let close = dec!(1) * rust_decimal::Decimal::try_from(base).unwrap();
            let vol = if i == n - 1 {
                // Spike on the last candle for volume tests.
                dec!(5000)
            } else {
                dec!(1000)
            };
            md.candles_1h.push(Candle::new(
                Utc::now(),
                close,
                close + dec!(1),
                close - dec!(1),
                close,
                vol,
            ));
        }
        md
    }

    fn base_config(rules: Vec<EntryRule>) -> UserStrategyConfig {
        UserStrategyConfig {
            name: "Test".to_string(),
            risk: RiskParams::default(),
            entry_rules: rules,
        }
    }

    // ── RSI ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_rsi_is_below_enough_data() {
        let md = synthetic_market_data(50);
        let config = base_config(vec![EntryRule::Rsi(RsiRule {
            lookback: 14,
            threshold: 80.0, // Rising prices → RSI high → IsBelow(80) likely false
            condition: RsiCondition::IsBelow,
            interval: Interval::H1,
        })]);
        let evaluator = RuleEvaluator::new();
        // Should not error; result depends on synthetic data direction.
        let result = evaluator.evaluate(&md, &config);
        assert!(result.is_ok(), "Should not error with sufficient data");
    }

    #[test]
    fn test_rsi_insufficient_data() {
        let md = synthetic_market_data(5); // Far too few candles.
        let config = base_config(vec![EntryRule::Rsi(RsiRule {
            lookback: 14,
            threshold: 30.0,
            condition: RsiCondition::IsBelow,
            interval: Interval::H1,
        })]);
        let evaluator = RuleEvaluator::new();
        assert!(matches!(
            evaluator.evaluate(&md, &config),
            Err(EvaluatorError::InsufficientData { .. })
        ));
    }

    // ── Moving Average ────────────────────────────────────────────────────────

    #[test]
    fn test_ma_price_is_above_ema20() {
        let md = synthetic_market_data(60);
        let config = base_config(vec![EntryRule::Ma(MaRule {
            ma_type: MaType::Ema,
            lookback: 20,
            slow_lookback: None,
            condition: MaCondition::PriceIsAbove,
            interval: Interval::H1,
        })]);
        let evaluator = RuleEvaluator::new();
        // Steadily rising prices → close should be above EMA(20).
        let result = evaluator.evaluate(&md, &config).unwrap();
        assert!(result, "Rising prices should be above their EMA(20)");
    }

    #[test]
    fn test_ma_golden_cross() {
        let md = synthetic_market_data(60);
        let config = base_config(vec![EntryRule::Ma(MaRule {
            ma_type: MaType::Ema,
            lookback: 9,
            slow_lookback: Some(21),
            condition: MaCondition::FastCrossesSlow,
            interval: Interval::H1,
        })]);
        let evaluator = RuleEvaluator::new();
        // Evaluating doesn't error (correctness of signal depends on data shape).
        assert!(evaluator.evaluate(&md, &config).is_ok());
    }

    // ── Volume ────────────────────────────────────────────────────────────────

    #[test]
    fn test_volume_spike_detected() {
        let md = synthetic_market_data(30); // Last candle has 5× normal volume.
        let config = base_config(vec![EntryRule::Volume(VolumeRule {
            lookback: 20,
            multiplier: 2.0, // Spike is 5000 vs baseline ~1000 → passes 2×.
            interval: Interval::H1,
        })]);
        let evaluator = RuleEvaluator::new();
        let result = evaluator.evaluate(&md, &config).unwrap();
        assert!(result, "5× volume spike should exceed 2× multiplier threshold");
    }

    #[test]
    fn test_volume_no_spike() {
        let md = synthetic_market_data(30);
        let config = base_config(vec![EntryRule::Volume(VolumeRule {
            lookback: 20,
            multiplier: 10.0, // Threshold too high — spike won't pass.
            interval: Interval::H1,
        })]);
        let evaluator = RuleEvaluator::new();
        let result = evaluator.evaluate(&md, &config).unwrap();
        assert!(!result, "5× spike should not exceed 10× threshold");
    }

    // ── AND logic ────────────────────────────────────────────────────────────

    #[test]
    fn test_and_logic_short_circuits_on_first_false() {
        let md = synthetic_market_data(30);
        // Volume spike passes (2×), but RSI IsBelow(10) almost certainly fails.
        let config = base_config(vec![
            EntryRule::Volume(VolumeRule {
                lookback: 20,
                multiplier: 2.0,
                interval: Interval::H1,
            }),
            EntryRule::Rsi(RsiRule {
                lookback: 14,
                threshold: 10.0, // Virtually impossible.
                condition: RsiCondition::IsBelow,
                interval: Interval::H1,
            }),
        ]);
        let evaluator = RuleEvaluator::new();
        let result = evaluator.evaluate(&md, &config).unwrap();
        assert!(!result, "RSI condition should fail, causing overall false");
    }
}
