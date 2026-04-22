//! Strategy module — Phase 2: JSON-driven Rule Evaluator.
//!
//! ## Architecture
//! The "brain" is fully decoupled from the "body" (ExecutionEngine).
//! A user defines their strategy as a JSON document ([`UserStrategyConfig`]),
//! which the [`RuleEvaluator`] evaluates against live [`MarketData`].
//!
//! ## Phase roadmap
//! - **Phase 1** *(complete)*: Empty placeholder — AnalysisEngine intentionally excluded.
//! - **Phase 2** *(this module)*: JSON schema + stateless RuleEvaluator.
//! - **Phase 3** *(upcoming)*: Tauri IPC commands to load/save strategies from the UI.
//!
//! ## Usage
//! ```ignore
//! use crate::strategy::{RuleEvaluator, UserStrategyConfig};
//!
//! let config: UserStrategyConfig = serde_json::from_str(json_str)?;
//! let evaluator = RuleEvaluator::new();
//! if evaluator.evaluate(&market_data, &config)? {
//!     // All conditions met — pass signal to ExecutionEngine
//! }
//! ```

pub mod evaluator;
pub mod models;

// ── Public re-exports (Phase 3 IPC surface) ────────────────────────────────

pub use evaluator::{EvaluatorError, RuleEvaluator};
pub use models::{
    EntryRule, MaCondition, MaRule, MaType, RiskParams, RsiCondition, RsiRule,
    UserStrategyConfig, VolumeRule,
};
