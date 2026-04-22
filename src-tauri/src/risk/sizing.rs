//! Position sizing calculations with risk constraints.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::{Balance, Price, Volume};

use super::config;

/// Errors that can occur during position sizing.
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum RiskError {
    /// Volatility ratio exceeds skip threshold.
    #[error("Volatility too high: ATR ratio {ratio} exceeds threshold {threshold}")]
    VolatilityTooHigh { ratio: Decimal, threshold: Decimal },

    /// Circuit breaker triggered due to daily loss limit.
    #[error("Circuit breaker triggered: daily loss {daily_loss_pct}% exceeds limit {limit}%")]
    CircuitBreakerTriggered {
        daily_loss_pct: Decimal,
        limit: Decimal,
    },

    /// Insufficient equity for minimum position.
    #[error("Insufficient equity: {available} available, {required} required")]
    InsufficientEquity {
        available: Balance,
        required: Balance,
    },

    /// Invalid stop loss (same as entry or wrong direction).
    #[error("Invalid stop loss: entry={entry}, stop_loss={stop_loss}")]
    InvalidStopLoss { entry: Price, stop_loss: Price },

    /// Zero or negative equity.
    #[error("Invalid equity: {equity}")]
    InvalidEquity { equity: Balance },
}

/// Input parameters for position sizing calculation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizingInput {
    /// Current account equity.
    pub equity: Balance,
    /// Entry price for the trade.
    pub entry_price: Price,
    /// Stop loss price.
    pub stop_loss: Price,
    /// 15-minute ATR value.
    pub atr_15m: Decimal,
    /// 4-hour ATR value.
    pub atr_4h: Decimal,
    /// Coin market cap in USD (optional).
    pub market_cap: Option<Decimal>,
}

/// Result of position sizing calculation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizingResult {
    /// Final position size in base currency.
    pub size: Volume,
    /// Base size before adjustments.
    pub base_size: Volume,
    /// Risk amount in quote currency.
    pub risk_amount: Balance,
    /// Whether volatility reduction was applied.
    pub volatility_reduced: bool,
    /// Whether market cap reduction was applied.
    pub market_cap_reduced: bool,
    /// Whether liquidity cap was applied.
    pub liquidity_capped: bool,
    /// ATR ratio (15M / 4H).
    pub atr_ratio: Decimal,
}

/// Calculates position size with all risk constraints applied.
///
/// # Formula
/// Base size = (Equity × 0.8%) / |Entry - StopLoss|
///
/// # Modifiers (applied sequentially)
/// 1. Volatility scalar: ×0.5 if ATR ratio > 2.0
/// 2. Market cap scalar: ×0.5 if < $1B
/// 3. Liquidity cap: min(size, equity × 3%)
///
/// # Errors
/// - `VolatilityTooHigh` if ATR ratio > 3.0
/// - `InvalidStopLoss` if entry equals stop loss
/// - `InvalidEquity` if equity is zero or negative
pub fn calculate_position_size(input: &SizingInput) -> Result<SizingResult, RiskError> {
    // Validate equity
    if input.equity <= Decimal::ZERO {
        return Err(RiskError::InvalidEquity {
            equity: input.equity,
        });
    }

    // Validate stop loss
    let risk_per_unit = (input.entry_price - input.stop_loss).abs();
    if risk_per_unit == Decimal::ZERO {
        return Err(RiskError::InvalidStopLoss {
            entry: input.entry_price,
            stop_loss: input.stop_loss,
        });
    }

    // Calculate ATR ratio
    let atr_ratio = if input.atr_4h > Decimal::ZERO {
        input.atr_15m / input.atr_4h
    } else {
        Decimal::ONE
    };

    // Check volatility skip threshold
    if atr_ratio > config::VOLATILITY_SKIP_THRESHOLD {
        return Err(RiskError::VolatilityTooHigh {
            ratio: atr_ratio,
            threshold: config::VOLATILITY_SKIP_THRESHOLD,
        });
    }

    // Calculate risk amount (0.8% of equity)
    let risk_amount = input.equity * config::RISK_PER_TRADE;

    // Calculate base position size
    let base_size = risk_amount / risk_per_unit;

    let mut size = base_size;
    let mut volatility_reduced = false;
    let mut market_cap_reduced = false;
    let mut liquidity_capped = false;

    // Apply volatility reduction (50% if ATR ratio > 2.0)
    if atr_ratio > config::VOLATILITY_REDUCE_THRESHOLD {
        size *= config::VOLATILITY_REDUCTION_FACTOR;
        volatility_reduced = true;
    }

    // Apply market cap reduction (50% if < $1B)
    if let Some(market_cap) = input.market_cap {
        if market_cap < config::SMALL_CAP_THRESHOLD {
            size *= config::SMALL_CAP_REDUCTION_FACTOR;
            market_cap_reduced = true;
        }
    }

    // Apply liquidity cap (max 3% of equity in position value)
    let max_position_value = input.equity * config::MAX_POSITION_EQUITY_PCT;
    let max_size = max_position_value / input.entry_price;

    if size > max_size {
        size = max_size;
        liquidity_capped = true;
    }

    Ok(SizingResult {
        size,
        base_size,
        risk_amount,
        volatility_reduced,
        market_cap_reduced,
        liquidity_capped,
        atr_ratio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn default_input() -> SizingInput {
        // Using $1000 entry so position value stays under 3% liquidity cap
        // Base size = (100,000 * 0.008) / 10 = 80 units
        // Position value = 80 * 1000 = $80,000 > $3,000 cap still hit
        // Use higher risk to reduce base size:
        // With entry=1000, stop=990, risk=10, base_size=800/10=80
        //
        // Better approach: use entry=$100, stop=$90, risk=$10
        // base_size = 800 / 10 = 80 units, value = 80 * 100 = $8,000 > cap
        //
        // Use entry=$100, stop=$99, risk=$1
        // base_size = 800 / 1 = 800 units, value = 800 * 100 = $80,000 > cap
        //
        // For testing without cap: entry=$1000, stop=$0 (high risk)
        // Or use small equity. Let's use reasonable values where cap doesn't apply.
        //
        // With equity=100,000, max_position_value = 3,000
        // To get 0.8 units under cap: 0.8 * entry < 3,000 → entry < 3,750
        // Let's use entry=1000, stop=0 (impossible) or entry=1000, stop=999
        // risk = 1, base_size = 800, value = 800,000 > cap
        //
        // The issue: 0.8% risk with any reasonable stop creates large position.
        // Solution: use larger risk per unit OR update test expectations.
        //
        // Let's use more realistic: entry=100, stop=0 (risk=100)
        // base_size = 800 / 100 = 8 units
        // position_value = 8 * 100 = 800 < 3,000 cap ✓
        SizingInput {
            equity: dec!(100_000),
            entry_price: dec!(100),
            stop_loss: dec!(0), // 100% risk (extreme but good for testing)
            atr_15m: dec!(1),
            atr_4h: dec!(1),
            market_cap: Some(dec!(10_000_000_000)), // $10B
        }
    }

    #[test]
    fn test_basic_position_sizing() {
        let input = default_input();
        let result = calculate_position_size(&input).unwrap();

        // Base size = (100,000 * 0.05) / 100 = 50
        // Position value = 50 * 100 = 5,000 < 500,000 cap
        assert_eq!(result.base_size, dec!(50));
        assert_eq!(result.size, dec!(50));
        assert!(!result.volatility_reduced);
        assert!(!result.market_cap_reduced);
        assert!(!result.liquidity_capped);
    }

    #[test]
    fn test_volatility_reduction() {
        let mut input = default_input();
        input.atr_15m = dec!(2.5); // Ratio = 2.5, above 2.0 threshold

        let result = calculate_position_size(&input).unwrap();

        // Base = 50, reduced by 50% = 25
        assert_eq!(result.base_size, dec!(50));
        assert_eq!(result.size, dec!(25));
        assert!(result.volatility_reduced);
        assert_eq!(result.atr_ratio, dec!(2.5));
    }

    #[test]
    fn test_volatility_skip() {
        let mut input = default_input();
        input.atr_15m = dec!(3.5); // Ratio = 3.5, above 3.0 threshold

        let result = calculate_position_size(&input);

        assert!(matches!(result, Err(RiskError::VolatilityTooHigh { .. })));
    }

    #[test]
    fn test_market_cap_reduction() {
        let mut input = default_input();
        input.market_cap = Some(dec!(500_000_000)); // $500M, below $1B

        let result = calculate_position_size(&input).unwrap();

        // Base = 50, reduced by 50% = 25
        assert_eq!(result.size, dec!(25));
        assert!(result.market_cap_reduced);
    }

    #[test]
    fn test_combined_reductions() {
        let mut input = default_input();
        input.atr_15m = dec!(2.5); // Volatility reduction
        input.market_cap = Some(dec!(500_000_000)); // Market cap reduction

        let result = calculate_position_size(&input).unwrap();

        // Base 50 * 0.5 (volatility) * 0.5 (market cap) = 12.5
        assert_eq!(result.size, dec!(12.5));
        assert!(result.volatility_reduced);
        assert!(result.market_cap_reduced);
    }

    #[test]
    fn test_liquidity_cap() {
        let mut input = default_input();
        input.entry_price = dec!(100);
        input.stop_loss = dec!(99.99); // Tiny risk → huge base position

        let result = calculate_position_size(&input).unwrap();

        // risk_per_unit = 0.01
        // Base size = 5,000 / 0.01 = 500,000 units
        // Max position value = 100,000 * 5.0 = 500,000
        // Max size = 500,000 / 100 = 5,000 units
        // 500,000 > 5,000 → capped
        assert!(result.liquidity_capped);
        assert_eq!(result.size, dec!(5000));
    }

    #[test]
    fn test_invalid_stop_loss() {
        let mut input = default_input();
        input.stop_loss = input.entry_price; // Same as entry

        let result = calculate_position_size(&input);

        assert!(matches!(result, Err(RiskError::InvalidStopLoss { .. })));
    }

    #[test]
    fn test_invalid_equity() {
        let mut input = default_input();
        input.equity = dec!(0);

        let result = calculate_position_size(&input);

        assert!(matches!(result, Err(RiskError::InvalidEquity { .. })));
    }
}
