//! Position direction type.

use serde::{Deserialize, Serialize};

/// Current position direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Position {
    /// Long position (betting on price increase).
    Long,
    /// Short position (betting on price decrease).
    Short,
    /// No active position.
    #[default]
    None,
}

impl Position {
    /// Returns true if there is an active position.
    pub fn is_active(&self) -> bool {
        !matches!(self, Position::None)
    }

    /// Returns the opposite position direction.
    pub fn opposite(&self) -> Self {
        match self {
            Position::Long => Position::Short,
            Position::Short => Position::Long,
            Position::None => Position::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_is_active() {
        assert!(Position::Long.is_active());
        assert!(Position::Short.is_active());
        assert!(!Position::None.is_active());
    }

    #[test]
    fn test_position_opposite() {
        assert_eq!(Position::Long.opposite(), Position::Short);
        assert_eq!(Position::Short.opposite(), Position::Long);
        assert_eq!(Position::None.opposite(), Position::None);
    }
}
