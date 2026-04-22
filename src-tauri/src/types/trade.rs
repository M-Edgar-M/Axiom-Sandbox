//! Trade management types with take-profit targets.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::position::Position;
use super::primitives::{Balance, Price, Volume};

/// Take-profit target with price level and R-multiple.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TakeProfitTarget {
    /// Target price level.
    pub price: Price,
    /// Risk-reward multiple (e.g., 2.0R, 3.0R).
    pub r_multiple: Decimal,
    /// Whether this target has been hit.
    pub hit: bool,
}

impl TakeProfitTarget {
    /// Creates a new take-profit target.
    pub fn new(price: Price, r_multiple: Decimal) -> Self {
        Self {
            price,
            r_multiple,
            hit: false,
        }
    }
}

/// Represents a trade with entry, stop loss, and multiple take-profit targets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    /// Trading pair symbol (e.g., "BTCUSDT").
    pub symbol: String,
    /// Position direction.
    pub position: Position,
    /// Entry price.
    pub entry_price: Price,
    /// Stop loss price.
    pub stop_loss: Price,
    /// Position size in base currency.
    pub size: Volume,
    /// Take-profit targets (TP1 at 2.0R, TP2 at 3.0R).
    pub take_profits: Vec<TakeProfitTarget>,
    /// Trade open timestamp.
    pub opened_at: DateTime<Utc>,
    /// Trade close timestamp (if closed).
    pub closed_at: Option<DateTime<Utc>>,
    /// Realized PnL (profit/loss) in quote currency.
    pub realized_pnl: Option<Balance>,
}

impl Trade {
    /// Creates a new trade with default take-profit targets at 2.0R and 3.0R.
    ///
    /// # Arguments
    /// * `symbol` - Trading pair symbol
    /// * `position` - Long or Short
    /// * `entry_price` - Entry price
    /// * `stop_loss` - Stop loss price
    /// * `size` - Position size
    ///
    /// # Panics
    /// Panics if position is `Position::None`.
    pub fn new(
        symbol: impl Into<String>,
        position: Position,
        entry_price: Price,
        stop_loss: Price,
        size: Volume,
    ) -> Self {
        assert!(
            position.is_active(),
            "Cannot create trade with Position::None"
        );

        // Calculate risk per unit (distance from entry to stop loss)
        let risk = (entry_price - stop_loss).abs();

        // Calculate take-profit prices based on R-multiples
        let tp1_price = Self::calculate_tp_price(position, entry_price, risk, Decimal::TWO);
        let tp2_price = Self::calculate_tp_price(position, entry_price, risk, Decimal::new(3, 0));

        Self {
            symbol: symbol.into(),
            position,
            entry_price,
            stop_loss,
            size,
            take_profits: vec![
                TakeProfitTarget::new(tp1_price, Decimal::TWO),
                TakeProfitTarget::new(tp2_price, Decimal::new(3, 0)),
            ],
            opened_at: Utc::now(),
            closed_at: None,
            realized_pnl: None,
        }
    }

    /// Calculates take-profit price based on R-multiple.
    fn calculate_tp_price(
        position: Position,
        entry: Price,
        risk: Price,
        r_multiple: Decimal,
    ) -> Price {
        let reward = risk * r_multiple;
        match position {
            Position::Long => entry + reward,
            Position::Short => entry - reward,
            Position::None => entry,
        }
    }

    /// Returns the risk amount per unit (entry to stop loss distance).
    pub fn risk_per_unit(&self) -> Price {
        (self.entry_price - self.stop_loss).abs()
    }

    /// Returns the total risk for the position in quote currency.
    pub fn total_risk(&self) -> Balance {
        self.risk_per_unit() * self.size
    }

    /// Returns true if the trade is still open.
    pub fn is_open(&self) -> bool {
        self.closed_at.is_none()
    }

    /// Checks if the stop loss has been hit given the current price.
    pub fn is_stopped_out(&self, current_price: Price) -> bool {
        match self.position {
            Position::Long => current_price <= self.stop_loss,
            Position::Short => current_price >= self.stop_loss,
            Position::None => false,
        }
    }

    /// Updates take-profit targets based on current price.
    /// Returns the indices of newly hit targets.
    pub fn update_take_profits(&mut self, current_price: Price) -> Vec<usize> {
        let mut hit_indices = Vec::new();

        for (i, tp) in self.take_profits.iter_mut().enumerate() {
            if tp.hit {
                continue;
            }

            let is_hit = match self.position {
                Position::Long => current_price >= tp.price,
                Position::Short => current_price <= tp.price,
                Position::None => false,
            };

            if is_hit {
                tp.hit = true;
                hit_indices.push(i);
            }
        }

        hit_indices
    }

    /// Calculates unrealized PnL at the given price.
    pub fn unrealized_pnl(&self, current_price: Price) -> Balance {
        let price_diff = match self.position {
            Position::Long => current_price - self.entry_price,
            Position::Short => self.entry_price - current_price,
            Position::None => Decimal::ZERO,
        };
        price_diff * self.size
    }

    /// Closes the trade and calculates realized PnL.
    pub fn close(&mut self, exit_price: Price) {
        self.closed_at = Some(Utc::now());
        self.realized_pnl = Some(self.unrealized_pnl(exit_price));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_trade_creation_long() {
        let trade = Trade::new(
            "BTCUSDT",
            Position::Long,
            dec!(50000),
            dec!(49000),
            dec!(0.1),
        );

        assert_eq!(trade.symbol, "BTCUSDT");
        assert_eq!(trade.position, Position::Long);
        assert_eq!(trade.entry_price, dec!(50000));
        assert_eq!(trade.stop_loss, dec!(49000));
        assert_eq!(trade.risk_per_unit(), dec!(1000));

        // Check TP1 at 2.0R: 50000 + (1000 * 2) = 52000
        assert_eq!(trade.take_profits[0].price, dec!(52000));
        assert_eq!(trade.take_profits[0].r_multiple, dec!(2));

        // Check TP2 at 3.0R: 50000 + (1000 * 3) = 53000
        assert_eq!(trade.take_profits[1].price, dec!(53000));
        assert_eq!(trade.take_profits[1].r_multiple, dec!(3));
    }

    #[test]
    fn test_trade_creation_short() {
        let trade = Trade::new(
            "ETHUSDT",
            Position::Short,
            dec!(3000),
            dec!(3100),
            dec!(1.0),
        );

        assert_eq!(trade.position, Position::Short);
        assert_eq!(trade.risk_per_unit(), dec!(100));

        // Check TP1 at 2.0R: 3000 - (100 * 2) = 2800
        assert_eq!(trade.take_profits[0].price, dec!(2800));

        // Check TP2 at 3.0R: 3000 - (100 * 3) = 2700
        assert_eq!(trade.take_profits[1].price, dec!(2700));
    }

    #[test]
    fn test_trade_stop_loss() {
        let trade = Trade::new(
            "BTCUSDT",
            Position::Long,
            dec!(50000),
            dec!(49000),
            dec!(0.1),
        );

        assert!(!trade.is_stopped_out(dec!(50000)));
        assert!(!trade.is_stopped_out(dec!(49001)));
        assert!(trade.is_stopped_out(dec!(49000)));
        assert!(trade.is_stopped_out(dec!(48000)));
    }

    #[test]
    fn test_trade_unrealized_pnl() {
        let trade = Trade::new(
            "BTCUSDT",
            Position::Long,
            dec!(50000),
            dec!(49000),
            dec!(0.1),
        );

        // Price up 1000 -> 0.1 * 1000 = 100 profit
        assert_eq!(trade.unrealized_pnl(dec!(51000)), dec!(100));

        // Price down 500 -> 0.1 * -500 = -50 loss
        assert_eq!(trade.unrealized_pnl(dec!(49500)), dec!(-50));
    }

    #[test]
    fn test_trade_update_take_profits() {
        let mut trade = Trade::new(
            "BTCUSDT",
            Position::Long,
            dec!(50000),
            dec!(49000),
            dec!(0.1),
        );

        // Price below TP1
        let hit = trade.update_take_profits(dec!(51000));
        assert!(hit.is_empty());

        // Price hits TP1 (52000)
        let hit = trade.update_take_profits(dec!(52000));
        assert_eq!(hit, vec![0]);
        assert!(trade.take_profits[0].hit);
        assert!(!trade.take_profits[1].hit);

        // Price hits TP2 (53000)
        let hit = trade.update_take_profits(dec!(53500));
        assert_eq!(hit, vec![1]);
        assert!(trade.take_profits[1].hit);
    }
}
