//! Strategy schema: user-defined JSON strategy configuration.
//!
//! All types here are `serde`-serialisable so they can be:
//!  - Loaded from a `.json` file on disk
//!  - Sent over Tauri IPC from the React frontend (Phase 3)
//!  - Validated at the boundary before reaching the evaluator

use serde::{Deserialize, Serialize};

use crate::types::Interval;

// ─────────────────────────────────────────────────────────────────────────────
// Risk Parameters
// ─────────────────────────────────────────────────────────────────────────────

/// User-configurable risk parameters for a strategy.
///
/// All percentages are expressed as **fractions of account equity** in the
/// range [0.0, 1.0] (e.g. `0.01` = 1%).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskParams {
    /// Fraction of account equity risked per trade.
    /// Clamped to [0.005, 0.05] (0.5 % – 5.0 %) by the engine.
    pub risk_per_trade: f64,

    /// Maximum cumulative loss in a single trading day before the engine
    /// halts all new entries (circuit-breaker).
    /// Expressed as a fraction of starting-day equity (e.g. `0.06` = 6 %).
    pub daily_loss_limit: f64,

    /// Fraction of the position closed at the first take-profit target.
    /// The remainder trails until final TP or stop-out.
    /// Range: (0.0, 1.0].
    pub profit_taking_pct: f64,

    /// Minimum acceptable reward-to-risk ratio for a trade to be placed.
    /// Trades with R:R below this value are skipped.
    pub minimum_rr: f64,
}

impl Default for RiskParams {
    fn default() -> Self {
        Self {
            risk_per_trade: 0.01,   // 1 % per trade
            daily_loss_limit: 0.05, // 5 % daily circuit-breaker
            profit_taking_pct: 0.5, // Close 50 % at first TP
            minimum_rr: 2.0,        // Minimum 2:1 R:R
        }
    }
}

impl RiskParams {
    /// Validates and clamps all fields to safe ranges.
    /// Returns `Err` with a description if a value is out of acceptable bounds.
    pub fn validate(&self) -> Result<(), String> {
        if !(0.005..=0.05).contains(&self.risk_per_trade) {
            return Err(format!(
                "risk_per_trade {:.3} out of range [0.005, 0.05]",
                self.risk_per_trade
            ));
        }
        if !(0.0..=1.0).contains(&self.daily_loss_limit) {
            return Err(format!(
                "daily_loss_limit {:.3} must be in [0.0, 1.0]",
                self.daily_loss_limit
            ));
        }
        if !(0.0..=1.0).contains(&self.profit_taking_pct) || self.profit_taking_pct == 0.0 {
            return Err(format!(
                "profit_taking_pct {:.3} must be in (0.0, 1.0]",
                self.profit_taking_pct
            ));
        }
        if self.minimum_rr < 1.0 {
            return Err(format!(
                "minimum_rr {:.2} must be >= 1.0",
                self.minimum_rr
            ));
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RSI Rule
// ─────────────────────────────────────────────────────────────────────────────

/// Which side of the threshold the RSI condition applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RsiCondition {
    /// RSI crosses above the threshold on the latest candle
    /// (previous RSI < threshold, current RSI >= threshold).
    CrossesAbove,
    /// RSI crosses below the threshold on the latest candle.
    CrossesBelow,
    /// RSI is currently below the threshold (oversold entry).
    IsBelow,
    /// RSI is currently above the threshold (overbought / momentum entry).
    IsAbove,
}

/// A rule that evaluates a Relative Strength Index condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RsiRule {
    /// Number of periods used to compute RSI. Common values: 14, 9, 21.
    pub lookback: usize,
    /// The RSI level compared against (0–100). E.g., 30 for oversold, 70 for overbought.
    pub threshold: f64,
    /// The logical condition to test against the threshold.
    pub condition: RsiCondition,
    /// Which candle timeframe to evaluate this rule on.
    #[serde(default = "default_interval")]
    pub interval: Interval,
}

// ─────────────────────────────────────────────────────────────────────────────
// Moving Average Rule
// ─────────────────────────────────────────────────────────────────────────────

/// Moving average calculation method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MaType {
    /// Simple Moving Average — equal weight to all periods.
    Sma,
    /// Exponential Moving Average — exponentially more weight to recent data.
    Ema,
}

/// The relationship between price/MAs to test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaCondition {
    /// Close price crosses above the MA on the latest candle.
    PriceCrossesAbove,
    /// Close price crosses below the MA on the latest candle.
    PriceCrossesBelow,
    /// Close price is currently above the MA.
    PriceIsAbove,
    /// Close price is currently below the MA.
    PriceIsBelow,
    /// Fast MA crosses above slow MA (golden-cross style).
    /// Requires `slow_lookback` to be set.
    FastCrossesSlow,
    /// Fast MA crosses below slow MA (death-cross style).
    /// Requires `slow_lookback` to be set.
    FastCrossesBelow,
}

/// A rule that evaluates a moving average condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaRule {
    /// Calculation method: SMA or EMA.
    pub ma_type: MaType,
    /// Primary (fast) lookback period.
    pub lookback: usize,
    /// Optional secondary (slow) lookback for crossover conditions.
    /// Must be set when using `FastCrossesSlow` / `FastCrossesBelow`.
    pub slow_lookback: Option<usize>,
    /// The logical condition to test.
    pub condition: MaCondition,
    /// Timeframe the MA is computed on.
    #[serde(default = "default_interval")]
    pub interval: Interval,
}

// ─────────────────────────────────────────────────────────────────────────────
// Volume Rule
// ─────────────────────────────────────────────────────────────────────────────

/// A rule that evaluates a volume spike condition.
///
/// Evaluates to `true` when:
///   `current_volume > multiplier × SMA(volume, lookback)`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VolumeRule {
    /// Rolling window used to compute the volume baseline SMA.
    /// Typical values: 10, 20, 50.
    pub lookback: usize,
    /// Volume must exceed this multiple of the baseline to pass.
    /// E.g., `1.5` means "volume must be at least 1.5× the 20-period average".
    pub multiplier: f64,
    /// Timeframe to evaluate volume on.
    #[serde(default = "default_interval")]
    pub interval: Interval,
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry Rule (discriminated union)
// ─────────────────────────────────────────────────────────────────────────────

/// A single technical condition.
///
/// All conditions in `UserStrategyConfig::entry_rules` must be satisfied
/// simultaneously (AND logic) for an entry signal to be emitted.
///
/// # JSON representation
/// ```json
/// { "Rsi": { "lookback": 14, "threshold": 30.0, "condition": "is_below", "interval": "H1" } }
/// { "Ma":  { "ma_type": "EMA", "lookback": 20, "condition": "price_is_above", "interval": "H1" } }
/// { "Volume": { "lookback": 20, "multiplier": 1.5, "interval": "H1" } }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum EntryRule {
    Rsi(RsiRule),
    Ma(MaRule),
    Volume(VolumeRule),
}

// ─────────────────────────────────────────────────────────────────────────────
// Top-level strategy configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Complete user-defined strategy configuration.
///
/// Serialises to/from JSON and is the single input to the `RuleEvaluator`.
///
/// # Minimal example (JSON)
/// ```json
/// {
///   "name": "RSI Oversold + Volume Spike",
///   "risk": {
///     "risk_per_trade": 0.01,
///     "daily_loss_limit": 0.05,
///     "profit_taking_pct": 0.5,
///     "minimum_rr": 2.0
///   },
///   "entry_rules": [
///     { "Rsi": { "lookback": 14, "threshold": 30.0, "condition": "is_below", "interval": "H1" } },
///     { "Volume": { "lookback": 20, "multiplier": 1.5, "interval": "H1" } }
///   ]
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserStrategyConfig {
    /// Human-readable strategy name (for logging / UI display).
    pub name: String,
    /// Risk management parameters.
    pub risk: RiskParams,
    /// List of entry conditions — ALL must be true for an entry signal.
    pub entry_rules: Vec<EntryRule>,
}

impl UserStrategyConfig {
    /// Validates the config, returning a list of all validation errors found.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if let Err(e) = self.risk.validate() {
            errors.push(format!("risk: {}", e));
        }
        if self.entry_rules.is_empty() {
            errors.push("entry_rules must contain at least one condition".to_string());
        }
        for (i, rule) in self.entry_rules.iter().enumerate() {
            if let EntryRule::Ma(r) = rule {
                let needs_slow = matches!(
                    r.condition,
                    MaCondition::FastCrossesSlow | MaCondition::FastCrossesBelow
                );
                if needs_slow && r.slow_lookback.is_none() {
                    errors.push(format!(
                        "entry_rules[{}]: FastCrosses* condition requires slow_lookback",
                        i
                    ));
                }
                if let Some(slow) = r.slow_lookback {
                    if slow <= r.lookback {
                        errors.push(format!(
                            "entry_rules[{}]: slow_lookback ({}) must be > lookback ({})",
                            i, slow, r.lookback
                        ));
                    }
                }
            }
        }
        errors
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn default_interval() -> Interval {
    Interval::H1
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> UserStrategyConfig {
        UserStrategyConfig {
            name: "Test Strategy".to_string(),
            risk: RiskParams::default(),
            entry_rules: vec![
                EntryRule::Rsi(RsiRule {
                    lookback: 14,
                    threshold: 30.0,
                    condition: RsiCondition::IsBelow,
                    interval: Interval::H1,
                }),
                EntryRule::Volume(VolumeRule {
                    lookback: 20,
                    multiplier: 1.5,
                    interval: Interval::H1,
                }),
            ],
        }
    }

    #[test]
    fn test_risk_params_default_is_valid() {
        assert!(RiskParams::default().validate().is_ok());
    }

    #[test]
    fn test_risk_params_too_low() {
        let r = RiskParams { risk_per_trade: 0.001, ..Default::default() };
        assert!(r.validate().is_err());
    }

    #[test]
    fn test_risk_params_too_high() {
        let r = RiskParams { risk_per_trade: 0.1, ..Default::default() };
        assert!(r.validate().is_err());
    }

    #[test]
    fn test_config_roundtrip_json() {
        let config = sample_config();
        let json = serde_json::to_string_pretty(&config).unwrap();
        let restored: UserStrategyConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, restored);
    }

    #[test]
    fn test_config_validation_empty_rules() {
        let mut config = sample_config();
        config.entry_rules.clear();
        let errors = config.validate();
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_config_ma_crossover_missing_slow() {
        let mut config = sample_config();
        config.entry_rules.push(EntryRule::Ma(MaRule {
            ma_type: MaType::Ema,
            lookback: 9,
            slow_lookback: None, // Missing!
            condition: MaCondition::FastCrossesSlow,
            interval: Interval::H1,
        }));
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("slow_lookback")));
    }
}
