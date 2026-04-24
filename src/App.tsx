import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import type {
  UserStrategyConfig,
  EntryRule,
  RsiRule,
  MaRule,
  VolumeRule,
  TradeRecordDto,
  SystemStatus,
  Interval,
  RsiCondition,
  MaType,
  MaCondition,
} from "./types";

// ─── Default Config ──────────────────────────────────────────────────────────

const DEFAULT_CONFIG: UserStrategyConfig = {
  name: "Default RSI + Volume Strategy",
  risk: {
    risk_per_trade: 0.01,   // 1% per trade
    daily_loss_limit: 0.05, // 5% daily circuit-breaker
    profit_taking_pct: 0.5, // Close 50% at first TP
    minimum_rr: 2.0,        // Minimum 2:1 R:R
  },
  entry_rules: [
    {
      Rsi: {
        lookback: 14,
        threshold: 30,
        condition: "is_below",
        interval: "H1",
      },
    },
    {
      Volume: {
        lookback: 20,
        multiplier: 1.5,
        interval: "H1",
      },
    },
  ],
};

// ─── Helper: format percentage display ───────────────────────────────────────

function fmtPct(val: number): string {
  return (val * 100).toFixed(2) + "%";
}

// ─── Subcomponent: Basic Risk Tab ─────────────────────────────────────────────

interface BasicRiskTabProps {
  config: UserStrategyConfig;
  onChange: (updated: UserStrategyConfig) => void;
}

function BasicRiskTab({ config, onChange }: BasicRiskTabProps) {
  const risk = config.risk;

  function setRisk(field: keyof typeof risk, value: number) {
    onChange({ ...config, risk: { ...risk, [field]: value } });
  }

  return (
    <div style={styles.tabContent}>
      <label style={styles.label}>Strategy Name</label>
      <input
        style={styles.input}
        type="text"
        value={config.name}
        onChange={(e) => onChange({ ...config, name: e.target.value })}
      />

      <fieldset style={styles.fieldset}>
        <legend style={styles.legend}>Risk Parameters</legend>

        <label style={styles.label}>
          Fractional Risk per Trade — {fmtPct(risk.risk_per_trade)}
          <span style={styles.hint}>&nbsp;(0.5% – 5.0%)</span>
        </label>
        <input
          style={styles.range}
          type="range"
          min={0.005}
          max={0.05}
          step={0.001}
          value={risk.risk_per_trade}
          onChange={(e) => setRisk("risk_per_trade", Number(e.target.value))}
        />
        <input
          style={{ ...styles.input, width: "100px" }}
          type="number"
          min={0.005}
          max={0.05}
          step={0.001}
          value={risk.risk_per_trade}
          onChange={(e) => setRisk("risk_per_trade", Number(e.target.value))}
        />

        <label style={styles.label}>
          Daily Loss Limit (Circuit-Breaker) — {fmtPct(risk.daily_loss_limit)}
          <span style={styles.hint}>&nbsp;(stops all entries when hit)</span>
        </label>
        <input
          style={styles.range}
          type="range"
          min={0.01}
          max={0.20}
          step={0.005}
          value={risk.daily_loss_limit}
          onChange={(e) => setRisk("daily_loss_limit", Number(e.target.value))}
        />
        <input
          style={{ ...styles.input, width: "100px" }}
          type="number"
          min={0.01}
          max={0.20}
          step={0.005}
          value={risk.daily_loss_limit}
          onChange={(e) => setRisk("daily_loss_limit", Number(e.target.value))}
        />

        <label style={styles.label}>
          Profit-Taking Aggression — {fmtPct(risk.profit_taking_pct)}
          <span style={styles.hint}>&nbsp;(% of position closed at first TP)</span>
        </label>
        <input
          style={styles.range}
          type="range"
          min={0.1}
          max={1.0}
          step={0.05}
          value={risk.profit_taking_pct}
          onChange={(e) => setRisk("profit_taking_pct", Number(e.target.value))}
        />
        <input
          style={{ ...styles.input, width: "100px" }}
          type="number"
          min={0.1}
          max={1.0}
          step={0.05}
          value={risk.profit_taking_pct}
          onChange={(e) => setRisk("profit_taking_pct", Number(e.target.value))}
        />
      </fieldset>
    </div>
  );
}

// ─── Subcomponent: Advanced Tab ───────────────────────────────────────────────

interface AdvancedTabProps {
  config: UserStrategyConfig;
  onChange: (updated: UserStrategyConfig) => void;
}

function AdvancedTab({ config, onChange }: AdvancedTabProps) {
  const risk = config.risk;

  function setRisk(field: keyof typeof risk, value: number) {
    onChange({ ...config, risk: { ...risk, [field]: value } });
  }

  return (
    <div style={styles.tabContent}>
      <fieldset style={styles.fieldset}>
        <legend style={styles.legend}>Trade Quality Filters</legend>

        <label style={styles.label}>
          Minimum Reward : Risk Ratio — {risk.minimum_rr.toFixed(1)} : 1
          <span style={styles.hint}>&nbsp;(trades below this R:R are skipped)</span>
        </label>
        <input
          style={styles.range}
          type="range"
          min={1.0}
          max={5.0}
          step={0.25}
          value={risk.minimum_rr}
          onChange={(e) => setRisk("minimum_rr", Number(e.target.value))}
        />
        <input
          style={{ ...styles.input, width: "100px" }}
          type="number"
          min={1.0}
          max={5.0}
          step={0.25}
          value={risk.minimum_rr}
          onChange={(e) => setRisk("minimum_rr", Number(e.target.value))}
        />
      </fieldset>

      <fieldset style={styles.fieldset}>
        <legend style={styles.legend}>In-Trade Management (Phase-Based)</legend>
        <p style={styles.hint}>
          The engine automatically promotes trades through three phases:
        </p>
        <ul style={{ margin: "8px 0 0 16px", lineHeight: "1.8" }}>
          <li>
            <strong>Phase 1</strong> — Entry to 1.5R: initial risk. Stop-loss held at entry level.
          </li>
          <li>
            <strong>Phase 2</strong> — At 1.5R: stop moved to breakeven, partial close at{" "}
            <strong>{fmtPct(risk.profit_taking_pct)}</strong> of position.
          </li>
          <li>
            <strong>Phase 3</strong> — At 2.5R: trailing ATR-based stop activated. Remainder runs.
          </li>
        </ul>
        <p style={{ ...styles.hint, marginTop: "10px" }}>
          Phase thresholds and trailing multiplier will be configurable in a future release.
        </p>
      </fieldset>
    </div>
  );
}

// ─── Subcomponent: Strategy Builder Tab ──────────────────────────────────────

interface StrategyBuilderTabProps {
  config: UserStrategyConfig;
  onChange: (updated: UserStrategyConfig) => void;
}

// Default new-rule drafts
const DEFAULT_RSI_DRAFT: RsiRule = { lookback: 14, threshold: 30, condition: "is_below", interval: "H1" };
const DEFAULT_MA_DRAFT: MaRule = { ma_type: "EMA", lookback: 20, slow_lookback: undefined, condition: "price_is_above", interval: "H1" };
const DEFAULT_VOL_DRAFT: VolumeRule = { lookback: 20, multiplier: 1.5, interval: "H1" };

function StrategyBuilderTab({ config, onChange }: StrategyBuilderTabProps) {
  const [rsiDraft, setRsiDraft] = useState<RsiRule>({ ...DEFAULT_RSI_DRAFT });
  const [maDraft, setMaDraft] = useState<MaRule>({ ...DEFAULT_MA_DRAFT });
  const [volDraft, setVolDraft] = useState<VolumeRule>({ ...DEFAULT_VOL_DRAFT });

  function addRule(rule: EntryRule) {
    onChange({ ...config, entry_rules: [...config.entry_rules, rule] });
  }

  function removeRule(index: number) {
    const updated = config.entry_rules.filter((_, i) => i !== index);
    onChange({ ...config, entry_rules: updated });
  }

  function labelForRule(rule: EntryRule, index: number): string {
    if ("Rsi" in rule) {
      const r = rule.Rsi;
      return `#${index + 1}  RSI(${r.lookback}) ${r.condition} ${r.threshold}  [${r.interval}]`;
    }
    if ("Ma" in rule) {
      const r = rule.Ma;
      const slow = r.slow_lookback !== undefined ? `/${r.slow_lookback}` : "";
      return `#${index + 1}  ${r.ma_type}(${r.lookback}${slow}) ${r.condition}  [${r.interval}]`;
    }
    if ("Volume" in rule) {
      const r = rule.Volume;
      return `#${index + 1}  Volume SMA(${r.lookback}) × ${r.multiplier}  [${r.interval}]`;
    }
    return `#${index + 1}  Unknown`;
  }

  const intervals: Interval[] = ["M15", "H1", "H4"];
  const rsiConditions: RsiCondition[] = ["crosses_above", "crosses_below", "is_above", "is_below"];
  const maTypes: MaType[] = ["SMA", "EMA"];
  const maConditions: MaCondition[] = [
    "price_crosses_above",
    "price_crosses_below",
    "price_is_above",
    "price_is_below",
    "fast_crosses_slow",
    "fast_crosses_below",
  ];

  const needsSlowLookback =
    maDraft.condition === "fast_crosses_slow" || maDraft.condition === "fast_crosses_below";

  return (
    <div style={styles.tabContent}>
      {/* Current Rules */}
      <fieldset style={styles.fieldset}>
        <legend style={styles.legend}>Active Entry Rules (AND logic — all must be true)</legend>
        {config.entry_rules.length === 0 ? (
          <p style={{ ...styles.hint, color: "#c0392b" }}>
            No entry rules defined — add at least one rule below.
          </p>
        ) : (
          <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
            {config.entry_rules.map((rule, i) => (
              <li key={i} style={styles.ruleRow}>
                <code style={styles.ruleCode}>{labelForRule(rule, i)}</code>
                <button
                  style={styles.removeBtn}
                  onClick={() => removeRule(i)}
                  title="Remove this rule"
                >
                  ✕
                </button>
              </li>
            ))}
          </ul>
        )}
      </fieldset>

      {/* RSI Rule Builder */}
      <fieldset style={styles.fieldset}>
        <legend style={styles.legend}>Add RSI Rule</legend>
        <div style={styles.ruleForm}>
          <label style={styles.smallLabel}>Lookback</label>
          <input
            style={{ ...styles.input, width: "70px" }}
            type="number"
            min={2}
            max={100}
            value={rsiDraft.lookback}
            onChange={(e) => setRsiDraft({ ...rsiDraft, lookback: Number(e.target.value) })}
          />
          <label style={styles.smallLabel}>Threshold (0–100)</label>
          <input
            style={{ ...styles.input, width: "70px" }}
            type="number"
            min={0}
            max={100}
            value={rsiDraft.threshold}
            onChange={(e) => setRsiDraft({ ...rsiDraft, threshold: Number(e.target.value) })}
          />
          <label style={styles.smallLabel}>Condition</label>
          <select
            style={styles.select}
            value={rsiDraft.condition}
            onChange={(e) => setRsiDraft({ ...rsiDraft, condition: e.target.value as RsiCondition })}
          >
            {rsiConditions.map((c) => (
              <option key={c} value={c}>{c}</option>
            ))}
          </select>
          <label style={styles.smallLabel}>Timeframe</label>
          <select
            style={styles.select}
            value={rsiDraft.interval}
            onChange={(e) => setRsiDraft({ ...rsiDraft, interval: e.target.value as Interval })}
          >
            {intervals.map((iv) => (
              <option key={iv} value={iv}>{iv}</option>
            ))}
          </select>
          <button style={styles.addBtn} onClick={() => addRule({ Rsi: { ...rsiDraft } })}>
            + Add RSI Rule
          </button>
        </div>
      </fieldset>

      {/* MA Rule Builder */}
      <fieldset style={styles.fieldset}>
        <legend style={styles.legend}>Add Moving Average Rule</legend>
        <div style={styles.ruleForm}>
          <label style={styles.smallLabel}>MA Type</label>
          <select
            style={styles.select}
            value={maDraft.ma_type}
            onChange={(e) => setMaDraft({ ...maDraft, ma_type: e.target.value as MaType })}
          >
            {maTypes.map((t) => (
              <option key={t} value={t}>{t}</option>
            ))}
          </select>
          <label style={styles.smallLabel}>Fast Lookback</label>
          <input
            style={{ ...styles.input, width: "70px" }}
            type="number"
            min={2}
            max={200}
            value={maDraft.lookback}
            onChange={(e) => setMaDraft({ ...maDraft, lookback: Number(e.target.value) })}
          />
          <label style={styles.smallLabel}>Slow Lookback {needsSlowLookback ? "(required)" : "(optional)"}</label>
          <input
            style={{ ...styles.input, width: "70px", opacity: needsSlowLookback ? 1 : 0.5 }}
            type="number"
            min={maDraft.lookback + 1}
            max={500}
            placeholder="—"
            value={maDraft.slow_lookback ?? ""}
            onChange={(e) =>
              setMaDraft({
                ...maDraft,
                slow_lookback: e.target.value === "" ? undefined : Number(e.target.value),
              })
            }
          />
          <label style={styles.smallLabel}>Condition</label>
          <select
            style={styles.select}
            value={maDraft.condition}
            onChange={(e) => setMaDraft({ ...maDraft, condition: e.target.value as MaCondition })}
          >
            {maConditions.map((c) => (
              <option key={c} value={c}>{c}</option>
            ))}
          </select>
          <label style={styles.smallLabel}>Timeframe</label>
          <select
            style={styles.select}
            value={maDraft.interval}
            onChange={(e) => setMaDraft({ ...maDraft, interval: e.target.value as Interval })}
          >
            {intervals.map((iv) => (
              <option key={iv} value={iv}>{iv}</option>
            ))}
          </select>
          <button style={styles.addBtn} onClick={() => addRule({ Ma: { ...maDraft } })}>
            + Add MA Rule
          </button>
        </div>
      </fieldset>

      {/* Volume Rule Builder */}
      <fieldset style={styles.fieldset}>
        <legend style={styles.legend}>Add Volume Rule</legend>
        <div style={styles.ruleForm}>
          <label style={styles.smallLabel}>SMA Lookback</label>
          <input
            style={{ ...styles.input, width: "70px" }}
            type="number"
            min={2}
            max={200}
            value={volDraft.lookback}
            onChange={(e) => setVolDraft({ ...volDraft, lookback: Number(e.target.value) })}
          />
          <label style={styles.smallLabel}>Spike Multiplier</label>
          <input
            style={{ ...styles.input, width: "70px" }}
            type="number"
            min={1.0}
            max={10.0}
            step={0.1}
            value={volDraft.multiplier}
            onChange={(e) => setVolDraft({ ...volDraft, multiplier: Number(e.target.value) })}
          />
          <label style={styles.smallLabel}>Timeframe</label>
          <select
            style={styles.select}
            value={volDraft.interval}
            onChange={(e) => setVolDraft({ ...volDraft, interval: e.target.value as Interval })}
          >
            {intervals.map((iv) => (
              <option key={iv} value={iv}>{iv}</option>
            ))}
          </select>
          <button style={styles.addBtn} onClick={() => addRule({ Volume: { ...volDraft } })}>
            + Add Volume Rule
          </button>
        </div>
      </fieldset>
    </div>
  );
}

// ─── Main App ─────────────────────────────────────────────────────────────────

type Tab = "basic" | "advanced" | "builder";

export default function App() {
  const [config, setConfig] = useState<UserStrategyConfig>(DEFAULT_CONFIG);
  const [activeTab, setActiveTab] = useState<Tab>("basic");

  const [sessionActive, setSessionActive] = useState(false);
  const [statusMessage, setStatusMessage] = useState<string>("");
  const [errorMessage, setErrorMessage] = useState<string>("");

  const [systemStatus, setSystemStatus] = useState<SystemStatus | null>(null);
  const [tradeHistory, setTradeHistory] = useState<TradeRecordDto[]>([]);

  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // ── Polling ───────────────────────────────────────────────────────────────

  const poll = useCallback(async () => {
    try {
      const [status, trades] = await Promise.all([
        invoke<SystemStatus>("get_system_status"),
        invoke<TradeRecordDto[]>("get_trade_history"),
      ]);
      setSystemStatus(status);
      setTradeHistory(trades);
      // Sync session flag if backend reports it stopped
      if (!status.session_active && sessionActive) {
        setSessionActive(false);
        setStatusMessage("Session ended (detected via poll).");
      }
    } catch (err) {
      // Non-fatal — backend may be initialising
      console.warn("[poll]", err);
    }
  }, [sessionActive]);

  useEffect(() => {
    if (sessionActive) {
      poll(); // immediate fetch
      pollRef.current = setInterval(poll, 4000);
    } else {
      if (pollRef.current) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
      // Always fetch status once when idle so equity/stats are visible
      poll();
    }
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, [sessionActive, poll]);

  // ── Session Control ───────────────────────────────────────────────────────

  async function handleStartSession() {
    setErrorMessage("");
    setStatusMessage("Starting session…");
    try {
      const result = await invoke<string>("start_mock_session", { config });
      setSessionActive(true);
      setStatusMessage(result);
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      setErrorMessage(msg);
      setStatusMessage("");
    }
  }

  async function handleStopSession() {
    setErrorMessage("");
    setStatusMessage("Stopping session…");
    try {
      const result = await invoke<string>("stop_session");
      setSessionActive(false);
      setStatusMessage(result);
      await poll(); // final status refresh
    } catch (err) {
      const msg = typeof err === "string" ? err : String(err);
      setErrorMessage(msg);
    }
  }

  // ── Derived display helpers ───────────────────────────────────────────────

  const winRatePct = systemStatus
    ? (Number(systemStatus.daily_stats.win_rate) * 100).toFixed(1) + "%"
    : "—";

  const dailyLossPct = systemStatus
    ? (Number(systemStatus.daily_loss_pct) * 100).toFixed(2) + "%"
    : "—";

  // ── Render ────────────────────────────────────────────────────────────────

  return (
    <div style={styles.root}>
      <header style={styles.header}>
        <h1 style={styles.title}>AXIOM SANDBOX</h1>
        <p style={styles.subtitle}>Mock Trading Dashboard — Phase 4</p>
      </header>

      {/* ── Control Panel ── */}
      <section style={styles.controlPanel}>
        <div style={styles.controlButtons}>
          <button
            style={{
              ...styles.startBtn,
              opacity: sessionActive ? 0.4 : 1,
              cursor: sessionActive ? "not-allowed" : "pointer",
            }}
            onClick={handleStartSession}
            disabled={sessionActive}
          >
            ▶ START MOCK TRADING
          </button>
          <button
            style={{
              ...styles.stopBtn,
              opacity: !sessionActive ? 0.4 : 1,
              cursor: !sessionActive ? "not-allowed" : "pointer",
            }}
            onClick={handleStopSession}
            disabled={!sessionActive}
          >
            ■ STOP SESSION
          </button>

          <span
            style={{
              ...styles.sessionBadge,
              background: sessionActive ? "#27ae60" : "#555",
            }}
          >
            {sessionActive ? "● LIVE" : "○ IDLE"}
          </span>
        </div>

        {statusMessage && <p style={styles.statusMsg}>{statusMessage}</p>}
        {errorMessage && <p style={styles.errorMsg}>⚠ {errorMessage}</p>}
      </section>

      {/* ── System Status Panel ── */}
      {systemStatus && (
        <section style={styles.statusPanel}>
          <div style={styles.statCard}>
            <span style={styles.statLabel}>Equity</span>
            <span style={styles.statValue}>
              ${Number(systemStatus.current_equity).toLocaleString(undefined, { minimumFractionDigits: 2 })}
            </span>
          </div>
          <div style={styles.statCard}>
            <span style={styles.statLabel}>Daily P&amp;L</span>
            <span
              style={{
                ...styles.statValue,
                color: Number(systemStatus.daily_stats.realized_pnl) >= 0 ? "#27ae60" : "#c0392b",
              }}
            >
              ${Number(systemStatus.daily_stats.realized_pnl).toFixed(2)}
            </span>
          </div>
          <div style={styles.statCard}>
            <span style={styles.statLabel}>Daily Loss Used</span>
            <span
              style={{
                ...styles.statValue,
                color: Number(systemStatus.daily_loss_pct) > 0.04 ? "#c0392b" : "#ecf0f1",
              }}
            >
              {dailyLossPct}
            </span>
          </div>
          <div style={styles.statCard}>
            <span style={styles.statLabel}>Trades</span>
            <span style={styles.statValue}>{systemStatus.daily_stats.trade_count}</span>
          </div>
          <div style={styles.statCard}>
            <span style={styles.statLabel}>Win Rate</span>
            <span style={styles.statValue}>{winRatePct}</span>
          </div>
          <div style={styles.statCard}>
            <span style={styles.statLabel}>Circuit Breaker</span>
            <span
              style={{
                ...styles.statValue,
                color: systemStatus.circuit_breaker_active ? "#c0392b" : "#27ae60",
              }}
            >
              {systemStatus.circuit_breaker_active ? "TRIPPED" : "OK"}
            </span>
          </div>
        </section>
      )}

      {/* ── Config Tabs ── */}
      <section style={styles.configSection}>
        <nav style={styles.tabNav}>
          {(["basic", "advanced", "builder"] as Tab[]).map((tab) => (
            <button
              key={tab}
              style={{
                ...styles.tabBtn,
                borderBottom: activeTab === tab ? "2px solid #3498db" : "2px solid transparent",
                color: activeTab === tab ? "#3498db" : "#bdc3c7",
              }}
              onClick={() => setActiveTab(tab)}
            >
              {tab === "basic" && "Basic Risk"}
              {tab === "advanced" && "Advanced"}
              {tab === "builder" && "Strategy Builder"}
            </button>
          ))}
        </nav>

        {activeTab === "basic" && (
          <BasicRiskTab config={config} onChange={setConfig} />
        )}
        {activeTab === "advanced" && (
          <AdvancedTab config={config} onChange={setConfig} />
        )}
        {activeTab === "builder" && (
          <StrategyBuilderTab config={config} onChange={setConfig} />
        )}
      </section>

      {/* ── Trade History Table ── */}
      <section style={styles.tableSection}>
        <h2 style={styles.sectionTitle}>
          Trade History
          {sessionActive && (
            <span style={styles.pollBadge}> — polling every 4s</span>
          )}
        </h2>
        {tradeHistory.length === 0 ? (
          <p style={styles.hint}>No trades recorded yet. Start a session to begin.</p>
        ) : (
          <div style={{ overflowX: "auto" }}>
            <table style={styles.table}>
              <thead>
                <tr>
                  {[
                    "ID",
                    "Symbol",
                    "Dir",
                    "Entry Time",
                    "Entry $",
                    "Stop $",
                    "TP $",
                    "Size",
                    "Status",
                    "Exit $",
                    "P&L",
                    "Phase",
                  ].map((h) => (
                    <th key={h} style={styles.th}>
                      {h}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {tradeHistory.map((trade) => {
                  const pnl = Number(trade.realized_pnl);
                  return (
                    <tr key={trade.id} style={styles.tr}>
                      <td style={styles.td}>{trade.id.slice(0, 8)}…</td>
                      <td style={styles.td}>{trade.symbol}</td>
                      <td
                        style={{
                          ...styles.td,
                          color: trade.direction === "Long" ? "#27ae60" : "#c0392b",
                          fontWeight: "bold",
                        }}
                      >
                        {trade.direction}
                      </td>
                      <td style={styles.td}>
                        {new Date(trade.entry_time).toLocaleString()}
                      </td>
                      <td style={styles.td}>{Number(trade.entry_price).toFixed(2)}</td>
                      <td style={styles.td}>{Number(trade.stop_loss).toFixed(2)}</td>
                      <td style={styles.td}>{Number(trade.take_profit).toFixed(2)}</td>
                      <td style={styles.td}>{Number(trade.position_size).toFixed(4)}</td>
                      <td
                        style={{
                          ...styles.td,
                          color: trade.status === "Open" ? "#f39c12" : "#95a5a6",
                        }}
                      >
                        {trade.status}
                      </td>
                      <td style={styles.td}>
                        {trade.exit_price ? Number(trade.exit_price).toFixed(2) : "—"}
                      </td>
                      <td
                        style={{
                          ...styles.td,
                          color: pnl > 0 ? "#27ae60" : pnl < 0 ? "#c0392b" : "#ecf0f1",
                        }}
                      >
                        {pnl === 0 ? "—" : (pnl > 0 ? "+" : "") + pnl.toFixed(2)}
                      </td>
                      <td style={styles.td}>{trade.phase}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </div>
  );
}

// ─── Inline Styles ─────────────────────────────────────────────────────────────

const styles = {
  root: {
    fontFamily: "monospace, sans-serif",
    background: "#1a1a2e",
    color: "#ecf0f1",
    minHeight: "100vh",
    padding: "24px",
    boxSizing: "border-box" as const,
    maxWidth: "1200px",
    margin: "0 auto",
  },
  header: {
    borderBottom: "1px solid #2c3e50",
    paddingBottom: "12px",
    marginBottom: "24px",
  },
  title: {
    margin: 0,
    fontSize: "24px",
    letterSpacing: "4px",
    color: "#3498db",
  },
  subtitle: {
    margin: "4px 0 0",
    fontSize: "12px",
    color: "#7f8c8d",
    letterSpacing: "1px",
  },
  controlPanel: {
    background: "#16213e",
    border: "1px solid #2c3e50",
    borderRadius: "4px",
    padding: "20px",
    marginBottom: "20px",
  },
  controlButtons: {
    display: "flex",
    alignItems: "center",
    gap: "12px",
    flexWrap: "wrap" as const,
  },
  startBtn: {
    background: "#27ae60",
    color: "#fff",
    border: "none",
    padding: "12px 28px",
    fontSize: "14px",
    fontWeight: "bold" as const,
    letterSpacing: "1px",
    borderRadius: "3px",
    cursor: "pointer",
  },
  stopBtn: {
    background: "#c0392b",
    color: "#fff",
    border: "none",
    padding: "12px 28px",
    fontSize: "14px",
    fontWeight: "bold" as const,
    letterSpacing: "1px",
    borderRadius: "3px",
    cursor: "pointer",
  },
  sessionBadge: {
    padding: "6px 14px",
    borderRadius: "20px",
    fontSize: "12px",
    fontWeight: "bold" as const,
    letterSpacing: "1px",
    color: "#fff",
  },
  statusMsg: {
    margin: "12px 0 0",
    fontSize: "13px",
    color: "#bdc3c7",
  },
  errorMsg: {
    margin: "12px 0 0",
    fontSize: "13px",
    color: "#e74c3c",
    background: "rgba(231, 76, 60, 0.12)",
    padding: "10px 14px",
    borderRadius: "3px",
    border: "1px solid rgba(231,76,60,0.3)",
  },
  statusPanel: {
    display: "flex",
    gap: "12px",
    flexWrap: "wrap" as const,
    marginBottom: "20px",
  },
  statCard: {
    background: "#16213e",
    border: "1px solid #2c3e50",
    borderRadius: "4px",
    padding: "14px 20px",
    minWidth: "120px",
    display: "flex",
    flexDirection: "column" as const,
    gap: "4px",
  },
  statLabel: {
    fontSize: "10px",
    color: "#7f8c8d",
    letterSpacing: "1px",
    textTransform: "uppercase" as const,
  },
  statValue: {
    fontSize: "18px",
    fontWeight: "bold" as const,
    color: "#ecf0f1",
  },
  configSection: {
    background: "#16213e",
    border: "1px solid #2c3e50",
    borderRadius: "4px",
    marginBottom: "20px",
  },
  tabNav: {
    display: "flex",
    borderBottom: "1px solid #2c3e50",
  },
  tabBtn: {
    background: "none",
    border: "none",
    padding: "12px 24px",
    fontSize: "13px",
    cursor: "pointer",
    letterSpacing: "0.5px",
    transition: "color 0.15s",
  },
  tabContent: {
    padding: "20px",
    display: "flex",
    flexDirection: "column" as const,
    gap: "12px",
  },
  fieldset: {
    border: "1px solid #2c3e50",
    borderRadius: "3px",
    padding: "16px",
    margin: 0,
  },
  legend: {
    color: "#7f8c8d",
    fontSize: "11px",
    letterSpacing: "1px",
    textTransform: "uppercase" as const,
    padding: "0 6px",
  },
  label: {
    display: "block",
    fontSize: "13px",
    color: "#bdc3c7",
    marginBottom: "6px",
    marginTop: "12px",
  },
  smallLabel: {
    fontSize: "11px",
    color: "#7f8c8d",
    letterSpacing: "0.5px",
  },
  input: {
    background: "#0f3460",
    border: "1px solid #2c3e50",
    color: "#ecf0f1",
    padding: "6px 10px",
    fontSize: "13px",
    borderRadius: "3px",
    outline: "none",
  },
  range: {
    width: "100%",
    maxWidth: "400px",
    display: "block",
    margin: "4px 0",
    accentColor: "#3498db",
  },
  select: {
    background: "#0f3460",
    border: "1px solid #2c3e50",
    color: "#ecf0f1",
    padding: "6px 10px",
    fontSize: "13px",
    borderRadius: "3px",
    outline: "none",
  },
  hint: {
    fontSize: "11px",
    color: "#7f8c8d",
    margin: 0,
  },
  ruleForm: {
    display: "flex",
    flexWrap: "wrap" as const,
    gap: "8px",
    alignItems: "flex-end",
  },
  ruleRow: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    padding: "8px 0",
    borderBottom: "1px solid #2c3e50",
  },
  ruleCode: {
    fontSize: "12px",
    color: "#3498db",
    background: "rgba(52, 152, 219, 0.08)",
    padding: "4px 10px",
    borderRadius: "3px",
  },
  removeBtn: {
    background: "none",
    border: "1px solid #c0392b",
    color: "#c0392b",
    padding: "4px 10px",
    fontSize: "12px",
    cursor: "pointer",
    borderRadius: "3px",
  },
  addBtn: {
    background: "#2c3e50",
    border: "1px solid #3498db",
    color: "#3498db",
    padding: "7px 16px",
    fontSize: "12px",
    cursor: "pointer",
    borderRadius: "3px",
    letterSpacing: "0.5px",
  },
  tableSection: {
    background: "#16213e",
    border: "1px solid #2c3e50",
    borderRadius: "4px",
    padding: "20px",
  },
  sectionTitle: {
    margin: "0 0 16px",
    fontSize: "14px",
    letterSpacing: "1px",
    color: "#bdc3c7",
    fontWeight: "normal" as const,
    textTransform: "uppercase" as const,
  },
  pollBadge: {
    fontSize: "11px",
    color: "#27ae60",
    fontWeight: "normal" as const,
    letterSpacing: "0px",
  },
  table: {
    width: "100%",
    borderCollapse: "collapse" as const,
    fontSize: "12px",
  },
  th: {
    background: "#0f3460",
    color: "#7f8c8d",
    padding: "10px 12px",
    textAlign: "left" as const,
    letterSpacing: "0.5px",
    fontWeight: "normal" as const,
    borderBottom: "1px solid #2c3e50",
    whiteSpace: "nowrap" as const,
  },
  tr: {
    borderBottom: "1px solid #1e2a3a",
  },
  td: {
    padding: "9px 12px",
    color: "#ecf0f1",
    whiteSpace: "nowrap" as const,
  },
} as const;
