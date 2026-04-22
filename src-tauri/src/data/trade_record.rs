//! Trade record data structure.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Number of decimal places for Binance compatibility.
pub const DECIMAL_PLACES: u32 = 8;

/// Trade direction (Long or Short).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeDirection {
    Long,
    Short,
}

impl fmt::Display for TradeDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TradeDirection::Long => write!(f, "Long"),
            TradeDirection::Short => write!(f, "Short"),
        }
    }
}

impl TradeDirection {
    /// Returns the opposite direction.
    pub fn opposite(&self) -> Self {
        match self {
            TradeDirection::Long => TradeDirection::Short,
            TradeDirection::Short => TradeDirection::Long,
        }
    }

    /// Parse from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "long" => Some(TradeDirection::Long),
            "short" => Some(TradeDirection::Short),
            _ => None,
        }
    }
}

/// Trade status (Open or Closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeStatus {
    Open,
    Closed,
}

impl fmt::Display for TradeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TradeStatus::Open => write!(f, "Open"),
            TradeStatus::Closed => write!(f, "Closed"),
        }
    }
}

impl TradeStatus {
    /// Parse from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "open" => Some(TradeStatus::Open),
            "closed" => Some(TradeStatus::Closed),
            _ => None,
        }
    }
}

/// Phase of the trade for in-trade management.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradePhase {
    /// Initial phase after entry.
    Phase1,
    /// After 1.5R: SL moved to breakeven, 33% closed.
    Phase2,
    /// After 2.5R: Trailing stop activated.
    Phase3,
}

impl fmt::Display for TradePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TradePhase::Phase1 => write!(f, "Phase1"),
            TradePhase::Phase2 => write!(f, "Phase2"),
            TradePhase::Phase3 => write!(f, "Phase3"),
        }
    }
}

impl TradePhase {
    /// Parse from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "phase1" => Some(TradePhase::Phase1),
            "phase2" => Some(TradePhase::Phase2),
            "phase3" => Some(TradePhase::Phase3),
            _ => None,
        }
    }
}

/// A complete trade record for logging and tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRecord {
    /// Unique trade identifier.
    pub id: String,
    /// Trading symbol (e.g., "BTCUSDT").
    pub symbol: String,
    /// Trade direction.
    pub direction: TradeDirection,
    /// Entry timestamp.
    pub entry_time: DateTime<Utc>,
    /// Entry price (8 decimal places).
    pub entry_price: Decimal,
    /// Stop loss price.
    pub stop_loss: Decimal,
    /// Take profit price.
    pub take_profit: Decimal,
    /// Position size in base currency.
    pub position_size: Decimal,
    /// Current trade status.
    pub status: TradeStatus,
    /// Exit timestamp (when closed).
    pub exit_time: Option<DateTime<Utc>>,
    /// Exit price (when closed).
    pub exit_price: Option<Decimal>,
    /// Realized profit/loss.
    pub realized_pnl: Decimal,
    /// Current trade phase.
    pub phase: TradePhase,
    /// ATR value for trailing stop calculation.
    pub atr_value: Decimal,
    /// Risk per unit (|entry - stop loss|).
    pub risk_per_unit: Decimal,
    /// RSI value at entry.
    pub entry_rsi: Decimal,
    /// ADX value at entry.
    pub entry_adx: Decimal,
    /// Trend condition at entry.
    pub trend_condition: String,
}

impl TradeRecord {
    /// Creates a new open trade record.
    pub fn new(
        id: impl Into<String>,
        symbol: impl Into<String>,
        direction: TradeDirection,
        entry_price: Decimal,
        stop_loss: Decimal,
        take_profit: Decimal,
        position_size: Decimal,
        atr_value: Decimal,
        entry_rsi: Decimal,
        entry_adx: Decimal,
        trend_condition: String,
    ) -> Self {
        let entry_price = entry_price.round_dp(DECIMAL_PLACES);
        let stop_loss = stop_loss.round_dp(DECIMAL_PLACES);
        let take_profit = take_profit.round_dp(DECIMAL_PLACES);
        let position_size = position_size.round_dp(DECIMAL_PLACES);
        let number_risk_per_unit = (entry_price - stop_loss).abs().round_dp(DECIMAL_PLACES);

        Self {
            id: id.into(),
            symbol: symbol.into(),
            direction,
            entry_time: Utc::now(),
            entry_price,
            stop_loss,
            take_profit,
            position_size,
            status: TradeStatus::Open,
            exit_time: None,
            exit_price: None,
            realized_pnl: Decimal::ZERO,
            phase: TradePhase::Phase1,
            atr_value: atr_value.round_dp(DECIMAL_PLACES),
            risk_per_unit: number_risk_per_unit,
            entry_rsi: entry_rsi.round_dp(DECIMAL_PLACES),
            entry_adx: entry_adx.round_dp(DECIMAL_PLACES),
            trend_condition,
        }
    }

    /// Calculates current profit in R-multiples.
    pub fn current_r_multiple(&self, current_price: Decimal) -> Decimal {
        if self.risk_per_unit == Decimal::ZERO {
            return Decimal::ZERO;
        }

        let pnl = match self.direction {
            TradeDirection::Long => current_price - self.entry_price,
            TradeDirection::Short => self.entry_price - current_price,
        };

        (pnl / self.risk_per_unit).round_dp(DECIMAL_PLACES)
    }

    /// Calculates unrealized P&L at current price.
    pub fn unrealized_pnl(&self, current_price: Decimal) -> Decimal {
        let pnl_per_unit = match self.direction {
            TradeDirection::Long => current_price - self.entry_price,
            TradeDirection::Short => self.entry_price - current_price,
        };

        (pnl_per_unit * self.position_size).round_dp(DECIMAL_PLACES)
    }

    /// Closes the trade with the given exit price.
    pub fn close(&mut self, exit_price: Decimal) {
        let exit_price = exit_price.round_dp(DECIMAL_PLACES);
        self.exit_time = Some(Utc::now());
        self.exit_price = Some(exit_price);
        self.status = TradeStatus::Closed;
        self.realized_pnl = self.unrealized_pnl(exit_price);
    }

    /// Updates to Phase 2: moves SL to breakeven.
    pub fn transition_to_phase2(&mut self, new_stop_loss: Decimal) {
        self.phase = TradePhase::Phase2;
        self.stop_loss = new_stop_loss.round_dp(DECIMAL_PLACES);
    }

    /// Updates to Phase 3: trailing stop activated.
    pub fn transition_to_phase3(&mut self) {
        self.phase = TradePhase::Phase3;
    }

    /// Reduces position size (for partial close at 1.5R).
    pub fn reduce_position(&mut self, amount: Decimal, realized_pnl: Decimal) {
        self.position_size = (self.position_size - amount).round_dp(DECIMAL_PLACES);
        self.realized_pnl += realized_pnl.round_dp(DECIMAL_PLACES);
    }

    /// Returns true if trade is still open.
    pub fn is_open(&self) -> bool {
        self.status == TradeStatus::Open
    }

    /// Calculates trailing stop price based on ATR.
    pub fn trailing_stop_price(&self, current_price: Decimal, atr_multiplier: Decimal) -> Decimal {
        let offset = (self.atr_value * atr_multiplier).round_dp(DECIMAL_PLACES);
        match self.direction {
            TradeDirection::Long => (current_price - offset).round_dp(DECIMAL_PLACES),
            TradeDirection::Short => (current_price + offset).round_dp(DECIMAL_PLACES),
        }
    }

    /// Generates CSV header row.
    pub fn csv_header() -> String {
        "id,symbol,direction,entry_time,entry_price,stop_loss,take_profit,position_size,status,exit_time,exit_price,realized_pnl,phase,atr_value,risk_per_unit,entry_rsi,entry_adx,trend_condition".to_string()
    }

    /// Converts record to CSV row.
    pub fn to_csv_row(&self) -> String {
        // Sanitize trend condition to prevent CSV breakage
        let clean_trend = self.trend_condition.replace(',', " ");

        format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            self.id,
            self.symbol,
            self.direction,
            self.entry_time.to_rfc3339(),
            self.entry_price,
            self.stop_loss,
            self.take_profit,
            self.position_size,
            self.status,
            self.exit_time.map_or("".to_string(), |t| t.to_rfc3339()),
            self.exit_price.map_or("".to_string(), |p| p.to_string()),
            self.realized_pnl,
            self.phase,
            self.atr_value,
            self.risk_per_unit,
            self.entry_rsi,
            self.entry_adx,
            clean_trend
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_trade_record_creation() {
        let record = TradeRecord::new(
            "trade-001",
            "BTCUSDT",
            TradeDirection::Long,
            dec!(50000),
            dec!(49000),
            dec!(53000),
            dec!(1),
            dec!(500),
            dec!(60),
            dec!(25),
            "StrongUptrend".to_string(),
        );

        assert_eq!(record.id, "trade-001");
        assert_eq!(record.symbol, "BTCUSDT");
        assert_eq!(record.direction, TradeDirection::Long);
        assert_eq!(record.entry_price, dec!(50000));
        assert_eq!(record.risk_per_unit, dec!(1000));
        assert_eq!(record.status, TradeStatus::Open);
        assert_eq!(record.phase, TradePhase::Phase1);
        assert_eq!(record.entry_rsi, dec!(60));
    }

    #[test]
    fn test_r_multiple_calculation() {
        let record = TradeRecord::new(
            "trade-001",
            "BTCUSDT",
            TradeDirection::Long,
            dec!(50000),
            dec!(49000),
            dec!(53000),
            dec!(1),
            dec!(500),
            dec!(0),
            dec!(0),
            "Neutral".to_string(),
        );

        // At 1.5R profit
        assert_eq!(record.current_r_multiple(dec!(51500)), dec!(1.5));

        // At 2.5R profit
        assert_eq!(record.current_r_multiple(dec!(52500)), dec!(2.5));

        // At 1R loss
        assert_eq!(record.current_r_multiple(dec!(49000)), dec!(-1));
    }

    #[test]
    fn test_unrealized_pnl() {
        let record = TradeRecord::new(
            "trade-001",
            "BTCUSDT",
            TradeDirection::Long,
            dec!(50000),
            dec!(49000),
            dec!(53000),
            dec!(2),
            dec!(500),
            dec!(0),
            dec!(0),
            "Neutral".to_string(),
        );

        // 1000 profit per unit * 2 units = 2000
        assert_eq!(record.unrealized_pnl(dec!(51000)), dec!(2000));
    }

    #[test]
    fn test_close_trade() {
        let mut record = TradeRecord::new(
            "trade-001",
            "BTCUSDT",
            TradeDirection::Long,
            dec!(50000),
            dec!(49000),
            dec!(53000),
            dec!(1),
            dec!(500),
            dec!(0),
            dec!(0),
            "Neutral".to_string(),
        );

        record.close(dec!(51500));

        assert_eq!(record.status, TradeStatus::Closed);
        assert_eq!(record.exit_price, Some(dec!(51500)));
        assert_eq!(record.realized_pnl, dec!(1500));
    }

    #[test]
    fn test_phase_transitions() {
        let mut record = TradeRecord::new(
            "trade-001",
            "BTCUSDT",
            TradeDirection::Long,
            dec!(50000),
            dec!(49000),
            dec!(53000),
            dec!(1),
            dec!(500),
            dec!(0),
            dec!(0),
            "Neutral".to_string(),
        );

        assert_eq!(record.phase, TradePhase::Phase1);

        record.transition_to_phase2(dec!(50000));
        assert_eq!(record.phase, TradePhase::Phase2);
        assert_eq!(record.stop_loss, dec!(50000)); // Breakeven

        record.transition_to_phase3();
        assert_eq!(record.phase, TradePhase::Phase3);
    }

    #[test]
    fn test_trailing_stop_price() {
        let record = TradeRecord::new(
            "trade-001",
            "BTCUSDT",
            TradeDirection::Long,
            dec!(50000),
            dec!(49000),
            dec!(53000),
            dec!(1),
            dec!(500), // ATR = 500
            dec!(0),
            dec!(0),
            "Neutral".to_string(),
        );

        // Trailing stop = current - (ATR * 2.0) = 52000 - 1000 = 51000
        assert_eq!(
            record.trailing_stop_price(dec!(52000), dec!(2)),
            dec!(51000)
        );
    }

    #[test]
    fn test_csv_row_generation() {
        let record = TradeRecord::new(
            "trade-001",
            "BTCUSDT",
            TradeDirection::Long,
            dec!(50000),
            dec!(49000),
            dec!(53000),
            dec!(1),
            dec!(500),
            dec!(0),
            dec!(0),
            "Neutral".to_string(),
        );

        let csv = record.to_csv_row();
        assert!(csv.contains("trade-001"));
        assert!(csv.contains("BTCUSDT"));
        assert!(csv.contains("Long"));
        assert!(csv.contains("Open"));
    }

    // ── Short position tests ──

    fn create_short_record() -> TradeRecord {
        // Short: entry=50000, SL=51000 (above), TP=47000 (below)
        // risk_per_unit = |50000 - 51000| = 1000
        TradeRecord::new(
            "short-001",
            "BTCUSDT",
            TradeDirection::Short,
            dec!(50000),
            dec!(51000),
            dec!(47000),
            dec!(1),
            dec!(500),
            dec!(45),
            dec!(30),
            "StrongDowntrend".to_string(),
        )
    }

    #[test]
    fn test_short_r_multiple() {
        let record = create_short_record();

        // Price drops 1500 => +1.5R for Short
        assert_eq!(record.current_r_multiple(dec!(48500)), dec!(1.5));

        // Price drops 2500 => +2.5R
        assert_eq!(record.current_r_multiple(dec!(47500)), dec!(2.5));

        // Price rises 1000 => -1R
        assert_eq!(record.current_r_multiple(dec!(51000)), dec!(-1));
    }

    #[test]
    fn test_short_unrealized_pnl() {
        let record = create_short_record();

        // Price drops 1000 => PnL = +1000 * 1 unit = +1000
        assert_eq!(record.unrealized_pnl(dec!(49000)), dec!(1000));

        // Price rises 500 => PnL = -500
        assert_eq!(record.unrealized_pnl(dec!(50500)), dec!(-500));
    }

    #[test]
    fn test_short_close_trade() {
        let mut record = create_short_record();

        // Close at 48500 => PnL = (50000 - 48500) * 1 = 1500
        record.close(dec!(48500));
        assert_eq!(record.status, TradeStatus::Closed);
        assert_eq!(record.exit_price, Some(dec!(48500)));
        assert_eq!(record.realized_pnl, dec!(1500));
    }

    #[test]
    fn test_short_trailing_stop_price() {
        let record = create_short_record();

        // Short trailing stop = current + (ATR * multiplier)
        // = 47500 + (500 * 2) = 48500
        assert_eq!(
            record.trailing_stop_price(dec!(47500), dec!(2)),
            dec!(48500)
        );
    }
}
