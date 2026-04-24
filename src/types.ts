/**
 * TypeScript interfaces that exactly mirror the Rust JSON schema.
 *
 * Serialisation notes (derived from Rust serde attributes):
 *   - `Interval`      → no rename attr  → variant name as-is: "M15" | "H1" | "H4"
 *   - `RsiCondition`  → snake_case      → "crosses_above" | "crosses_below" | ...
 *   - `MaType`        → SCREAMING_SNAKE_CASE → "SMA" | "EMA"
 *   - `MaCondition`   → snake_case      → "price_crosses_above" | ...
 *   - `EntryRule`     → PascalCase      → { Rsi: RsiRule } | { Ma: MaRule } | { Volume: VolumeRule }
 */

// ─── Primitive Enums ─────────────────────────────────────────────────────────

/** Candlestick timeframe. Serialised as-is (no Rust rename attr). */
export type Interval = "M15" | "H1" | "H4";

/** RSI direction condition. Rust: `#[serde(rename_all = "snake_case")]` */
export type RsiCondition =
  | "crosses_above"
  | "crosses_below"
  | "is_below"
  | "is_above";

/** Moving average calculation method. Rust: `#[serde(rename_all = "SCREAMING_SNAKE_CASE")]` */
export type MaType = "SMA" | "EMA";

/** Moving average condition. Rust: `#[serde(rename_all = "snake_case")]` */
export type MaCondition =
  | "price_crosses_above"
  | "price_crosses_below"
  | "price_is_above"
  | "price_is_below"
  | "fast_crosses_slow"
  | "fast_crosses_below";

// ─── Risk Parameters ─────────────────────────────────────────────────────────

/**
 * Risk management parameters. All percentages are fractions of account equity
 * in [0.0, 1.0] (e.g. 0.01 = 1%).
 */
export interface RiskParams {
  /** Fraction of equity risked per trade. Clamped to [0.005, 0.05]. */
  risk_per_trade: number;
  /** Maximum daily cumulative loss fraction before circuit-breaker fires. */
  daily_loss_limit: number;
  /** Fraction of position closed at first take-profit target. Range: (0.0, 1.0]. */
  profit_taking_pct: number;
  /** Minimum acceptable reward-to-risk ratio. Must be >= 1.0. */
  minimum_rr: number;
}

// ─── Entry Rule Components ────────────────────────────────────────────────────

/** Evaluates a Relative Strength Index condition. */
export interface RsiRule {
  /** Lookback period (e.g. 14, 9, 21). */
  lookback: number;
  /** RSI level to compare against (0–100). */
  threshold: number;
  /** Logical condition to test. */
  condition: RsiCondition;
  /** Candle timeframe to evaluate on. */
  interval: Interval;
}

/** Evaluates a moving average condition. */
export interface MaRule {
  /** SMA or EMA. */
  ma_type: MaType;
  /** Primary (fast) lookback period. */
  lookback: number;
  /** Secondary (slow) lookback for crossover conditions. Required for FastCrosses* conditions. */
  slow_lookback?: number;
  /** Logical condition to test. */
  condition: MaCondition;
  /** Timeframe the MA is computed on. */
  interval: Interval;
}

/** Evaluates a volume spike condition: current_volume > multiplier × SMA(volume, lookback). */
export interface VolumeRule {
  /** Rolling window for the volume baseline SMA. */
  lookback: number;
  /** Volume must exceed this multiple of the baseline to pass. */
  multiplier: number;
  /** Timeframe to evaluate volume on. */
  interval: Interval;
}

/**
 * A single technical entry condition.
 * Rust: `#[serde(rename_all = "PascalCase")]` on an enum → `{ "Rsi": {...} }` etc.
 * All rules in `entry_rules` must be true simultaneously (AND logic).
 */
export type EntryRule =
  | { Rsi: RsiRule }
  | { Ma: MaRule }
  | { Volume: VolumeRule };

// ─── Top-Level Strategy Config ────────────────────────────────────────────────

/** Complete user-defined strategy configuration passed to `start_mock_session`. */
export interface UserStrategyConfig {
  /** Human-readable strategy name shown in logs and UI. */
  name: string;
  /** Risk management parameters. */
  risk: RiskParams;
  /** List of entry conditions — ALL must be true for a signal (AND logic). */
  entry_rules: EntryRule[];
}

// ─── Response DTOs (from Rust backend) ───────────────────────────────────────

/** Daily trading statistics snapshot. */
export interface DailyStatsDto {
  /** Net realised P&L for the day (serialised as string from Decimal). */
  realized_pnl: string;
  trade_count: number;
  wins: number;
  losses: number;
  /** Win rate fraction as string (e.g. "0.6667"). */
  win_rate: string;
}

/**
 * Lightweight trade record safe for IPC serialisation.
 * All Decimal fields arrive as strings — use Number() to parse.
 */
export interface TradeRecordDto {
  id: string;
  symbol: string;
  /** "Long" or "Short" */
  direction: string;
  /** ISO 8601 / RFC 3339 timestamp string. */
  entry_time: string;
  entry_price: string;
  stop_loss: string;
  take_profit: string;
  position_size: string;
  /** "Open" or "Closed" */
  status: string;
  exit_time: string | null;
  exit_price: string | null;
  realized_pnl: string;
  /** "Phase1" | "Phase2" | "Phase3" */
  phase: string;
  risk_per_unit: string;
}

/** System health status snapshot returned by `get_system_status`. */
export interface SystemStatus {
  session_active: boolean;
  circuit_breaker_active: boolean;
  /** Daily loss as fraction of starting equity — serialised as string. */
  daily_loss_pct: string;
  /** Current paper equity — serialised as string. */
  current_equity: string;
  daily_stats: DailyStatsDto;
}
