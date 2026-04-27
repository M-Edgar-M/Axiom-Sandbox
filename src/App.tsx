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

function fmtPct(val: number): string {
  return (val * 100).toFixed(2) + "%";
}

// ─── Subcomponents ───────────────────────────────────────────────────────────

function BasicRiskTab({ config, onChange }: { config: UserStrategyConfig; onChange: React.Dispatch<React.SetStateAction<UserStrategyConfig>> }) {
  const setRisk = (field: keyof typeof config.risk, value: number) => onChange(prev => ({ ...prev, risk: { ...prev.risk, [field]: value } }));

  return (
    <div className="flex flex-col gap-sm p-lg">
      <div className="flex flex-col gap-xs mb-md">
        <label className="font-label-caps text-label-caps text-on-surface-variant">Strategy Name</label>
        <input
          className="bg-surface-container border border-outline-variant text-inverse-surface rounded focus:ring-primary-container focus:border-primary-container font-body-md p-2"
          type="text"
          value={config.name}
          onChange={(e) => onChange(prev => ({ ...prev, name: e.target.value }))}
        />
      </div>

      <fieldset className="border border-white/5 rounded-lg p-md bg-[#0A0A0A]">
        <legend className="font-label-caps text-label-caps text-primary-container px-2">Risk Parameters</legend>
        
        <div className="grid grid-cols-1 md:grid-cols-2 gap-lg mt-sm">
          <div className="flex flex-col gap-xs">
            <div className="flex items-center gap-1">
              <label className="font-body-sm text-body-sm text-on-surface-variant">
                Fractional Risk per Trade — <span className="text-inverse-surface font-data-md">{fmtPct(risk.risk_per_trade)}</span>
              </label>
              <span className="material-symbols-outlined text-[14px] cursor-help opacity-70 hover:opacity-100 transition-opacity" title="A position-sizing plan that always risks a consistent fraction of your account's total equity on each trade to prevent catastrophic drawdowns.">help</span>
            </div>
            <div className="flex gap-4 items-center">
              <input type="range" className="flex-1 accent-primary-container" min={0.005} max={0.05} step={0.001} value={risk.risk_per_trade} onChange={(e) => setRisk("risk_per_trade", Number(e.target.value))} />
            </div>
          </div>

          <div className="flex flex-col gap-xs">
            <div className="flex items-center gap-1">
              <label className="font-body-sm text-body-sm text-on-surface-variant">
                Daily Loss Limit — <span className="text-inverse-surface font-data-md">{fmtPct(risk.daily_loss_limit)}</span>
              </label>
              <span className="material-symbols-outlined text-[14px] cursor-help opacity-70 hover:opacity-100 transition-opacity" title="A circuit breaker that automatically halts trading for the day if this loss threshold is reached, protecting your account from a series of losing trades.">help</span>
            </div>
            <div className="flex gap-4 items-center">
              <input type="range" className="flex-1 accent-tertiary-container" min={0.01} max={0.20} step={0.005} value={risk.daily_loss_limit} onChange={(e) => setRisk("daily_loss_limit", Number(e.target.value))} />
            </div>
          </div>

          <div className="flex flex-col gap-xs">
            <div className="flex items-center gap-1">
              <label className="font-body-sm text-body-sm text-on-surface-variant">
                Profit-Taking Aggression — <span className="text-inverse-surface font-data-md">{fmtPct(risk.profit_taking_pct)}</span>
              </label>
              <span className="material-symbols-outlined text-[14px] cursor-help opacity-70 hover:opacity-100 transition-opacity" title="The percentage of your total position that will automatically be sold to lock in gains when the first profit target is reached.">help</span>
            </div>
            <div className="flex gap-4 items-center">
              <input type="range" className="flex-1 accent-secondary-container" min={0.1} max={1.0} step={0.05} value={risk.profit_taking_pct} onChange={(e) => setRisk("profit_taking_pct", Number(e.target.value))} />
            </div>
          </div>
        </div>
      </fieldset>
    </div>
  );
}

function AdvancedTab({ config, onChange }: { config: UserStrategyConfig; onChange: React.Dispatch<React.SetStateAction<UserStrategyConfig>> }) {
  const setRisk = (field: keyof typeof config.risk, value: number) => onChange(prev => ({ ...prev, risk: { ...prev.risk, [field]: value } }));

  return (
    <div className="flex flex-col gap-lg p-lg">
      <fieldset className="border border-white/5 rounded-lg p-md bg-[#0A0A0A]">
        <legend className="font-label-caps text-label-caps text-primary-container px-2">Trade Quality Filters</legend>
        <div className="flex flex-col gap-xs mt-sm">
          <div className="flex items-center gap-1">
            <label className="font-body-sm text-body-sm text-on-surface-variant">
              Minimum Reward : Risk Ratio — <span className="text-inverse-surface font-data-md">{risk.minimum_rr.toFixed(1)} : 1</span>
            </label>
            <span className="material-symbols-outlined text-[14px] cursor-help opacity-70 hover:opacity-100 transition-opacity" title="A mathematical filter that prevents the bot from executing a trade unless the potential reward is strictly greater than the initial risk.">help</span>
          </div>
          <div className="flex gap-4 items-center w-full max-w-md">
            <input type="range" className="flex-1 accent-primary-container" min={1.0} max={5.0} step={0.25} value={risk.minimum_rr} onChange={(e) => setRisk("minimum_rr", Number(e.target.value))} />
          </div>
        </div>
      </fieldset>

      <fieldset className="border border-white/5 rounded-lg p-md bg-[#0A0A0A]">
        <legend className="font-label-caps text-label-caps text-primary-container px-2">In-Trade Management</legend>
        <ul className="list-disc list-inside text-body-sm text-on-surface-variant space-y-2 mt-sm">
          <li><strong className="text-inverse-surface">Phase 1</strong> — Entry to 1.5R: initial risk. Stop-loss held at entry level.</li>
          <li><strong className="text-inverse-surface">Phase 2</strong> — At 1.5R: stop moved to breakeven, partial close at <strong className="text-inverse-surface">{fmtPct(risk.profit_taking_pct)}</strong>.</li>
          <li><strong className="text-inverse-surface">Phase 3</strong> — At 2.5R: trailing ATR-based stop activated. Remainder runs.</li>
        </ul>
      </fieldset>
    </div>
  );
}

function StrategyBuilderTab({ config, onChange }: { config: UserStrategyConfig; onChange: React.Dispatch<React.SetStateAction<UserStrategyConfig>> }) {
  const [rsiDraft, setRsiDraft] = useState<RsiRule>({ lookback: 14, threshold: 30, condition: "is_below", interval: "H1" });
  const [maDraft, setMaDraft] = useState<MaRule>({ ma_type: "EMA", lookback: 20, slow_lookback: undefined, condition: "price_is_above", interval: "H1" });
  const [volDraft, setVolDraft] = useState<VolumeRule>({ lookback: 20, multiplier: 1.5, interval: "H1" });

  const addRule = (rule: EntryRule) => onChange(prev => ({ ...prev, entry_rules: [...prev.entry_rules, rule] }));
  const removeRule = (idx: number) => onChange(prev => ({ ...prev, entry_rules: prev.entry_rules.filter((_, i) => i !== idx) }));

  return (
    <div className="flex flex-col gap-lg p-lg">
      <div className="flex flex-col gap-sm">
        {config.entry_rules.map((rule, i) => {
          let label = "";
          if ("Rsi" in rule) label = `RSI(${rule.Rsi.lookback}) ${rule.Rsi.condition.replace(/_/g, " ")} ${rule.Rsi.threshold} [${rule.Rsi.interval}]`;
          else if ("Ma" in rule) label = `${rule.Ma.ma_type}(${rule.Ma.lookback}${rule.Ma.slow_lookback ? "/" + rule.Ma.slow_lookback : ""}) ${rule.Ma.condition.replace(/_/g, " ")} [${rule.Ma.interval}]`;
          else if ("Volume" in rule) label = `Volume SMA(${rule.Volume.lookback}) × ${rule.Volume.multiplier} [${rule.Volume.interval}]`;
          
          return (
            <div key={i} className="flex items-center gap-sm bg-[#0A0A0A] p-sm rounded border border-white/5 hover:border-white/10 transition-colors group">
              <span className="font-data-md text-data-md text-on-surface-variant w-8">AND</span>
              <span className="font-data-md text-data-md text-inverse-surface uppercase flex-1">{label}</span>
              <button onClick={() => removeRule(i)} className="text-on-surface-variant hover:text-tertiary-container transition-all">
                <span className="material-symbols-outlined text-[18px]">close</span>
              </button>
            </div>
          );
        })}
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-md">
        {/* RSI */}
        <div className="bg-[#0A0A0A] border border-white/5 rounded-lg p-md flex flex-col gap-sm">
          <div className="flex items-center gap-1">
            <h4 className="font-label-caps text-label-caps text-primary-container">Add RSI Rule</h4>
            <span className="material-symbols-outlined text-[14px] cursor-help opacity-70 hover:opacity-100 transition-opacity" title="A momentum oscillator that measures the speed and change of price movements, used to identify when a market is overbought or oversold.">help</span>
          </div>
          <div className="grid grid-cols-2 gap-sm">
            <input type="number" className="bg-surface-container border-outline-variant text-inverse-surface text-sm rounded focus:ring-primary-container" value={rsiDraft.lookback} onChange={e => setRsiDraft({...rsiDraft, lookback: Number(e.target.value)})} placeholder="Lookback"/>
            <input type="number" className="bg-surface-container border-outline-variant text-inverse-surface text-sm rounded focus:ring-primary-container" value={rsiDraft.threshold} onChange={e => setRsiDraft({...rsiDraft, threshold: Number(e.target.value)})} placeholder="Threshold"/>
            <select className="bg-surface-container border-outline-variant text-inverse-surface text-sm rounded focus:ring-primary-container col-span-2" value={rsiDraft.condition} onChange={e => setRsiDraft({...rsiDraft, condition: e.target.value as RsiCondition})}>
              <option value="crosses_above">Crosses Above</option><option value="crosses_below">Crosses Below</option><option value="is_above">Is Above</option><option value="is_below">Is Below</option>
            </select>
            <select className="bg-surface-container border-outline-variant text-inverse-surface text-sm rounded focus:ring-primary-container col-span-2" value={rsiDraft.interval} onChange={e => setRsiDraft({...rsiDraft, interval: e.target.value as Interval})}>
              <option value="M15">M15</option><option value="H1">H1</option><option value="H4">H4</option>
            </select>
            <button onClick={() => addRule({ Rsi: { ...rsiDraft } })} className="col-span-2 border border-primary-container text-primary-container hover:bg-primary-container hover:text-on-primary-container py-1 rounded transition-colors text-xs font-bold uppercase tracking-widest mt-2">+ Add RSI</button>
          </div>
        </div>

        {/* MA */}
        <div className="bg-[#0A0A0A] border border-white/5 rounded-lg p-md flex flex-col gap-sm">
          <div className="flex items-center gap-1">
            <h4 className="font-label-caps text-label-caps text-primary-container">Add MA Rule</h4>
            <span className="material-symbols-outlined text-[14px] cursor-help opacity-70 hover:opacity-100 transition-opacity" title="A trend-following indicator that averages historical prices over a specific window to smooth out market noise and reveal the true trend direction.">help</span>
          </div>
          <div className="grid grid-cols-2 gap-sm">
            <select className="bg-surface-container border-outline-variant text-inverse-surface text-sm rounded focus:ring-primary-container" value={maDraft.ma_type} onChange={e => setMaDraft({...maDraft, ma_type: e.target.value as MaType})}><option value="SMA">SMA</option><option value="EMA">EMA</option></select>
            <select className="bg-surface-container border-outline-variant text-inverse-surface text-sm rounded focus:ring-primary-container" value={maDraft.interval} onChange={e => setMaDraft({...maDraft, interval: e.target.value as Interval})}><option value="M15">M15</option><option value="H1">H1</option><option value="H4">H4</option></select>
            <input type="number" className="bg-surface-container border-outline-variant text-inverse-surface text-sm rounded focus:ring-primary-container" value={maDraft.lookback} onChange={e => setMaDraft({...maDraft, lookback: Number(e.target.value)})} placeholder="Fast Lookback"/>
            <input type="number" className="bg-surface-container border-outline-variant text-inverse-surface text-sm rounded focus:ring-primary-container" value={maDraft.slow_lookback || ""} onChange={e => setMaDraft({...maDraft, slow_lookback: e.target.value ? Number(e.target.value) : undefined})} placeholder="Slow (Opt)"/>
            <select className="bg-surface-container border-outline-variant text-inverse-surface text-sm rounded focus:ring-primary-container col-span-2" value={maDraft.condition} onChange={e => setMaDraft({...maDraft, condition: e.target.value as MaCondition})}>
              <option value="price_crosses_above">Price Crosses Above</option><option value="price_crosses_below">Price Crosses Below</option><option value="price_is_above">Price Is Above</option><option value="price_is_below">Price Is Below</option><option value="fast_crosses_slow">Fast Crosses Slow</option><option value="fast_crosses_below">Fast Crosses Below</option>
            </select>
            <button onClick={() => addRule({ Ma: { ...maDraft } })} className="col-span-2 border border-primary-container text-primary-container hover:bg-primary-container hover:text-on-primary-container py-1 rounded transition-colors text-xs font-bold uppercase tracking-widest mt-2">+ Add MA</button>
          </div>
        </div>

        {/* Volume */}
        <div className="bg-[#0A0A0A] border border-white/5 rounded-lg p-md flex flex-col gap-sm">
          <div className="flex items-center gap-1">
            <h4 className="font-label-caps text-label-caps text-primary-container">Add Volume Rule</h4>
            <span className="material-symbols-outlined text-[14px] cursor-help opacity-70 hover:opacity-100 transition-opacity" title="Measures the strength of a price move by comparing the current volume to its historical average, helping confirm institutional participation.">help</span>
          </div>
          <div className="grid grid-cols-2 gap-sm">
            <input type="number" className="bg-surface-container border-outline-variant text-inverse-surface text-sm rounded focus:ring-primary-container col-span-2" value={volDraft.lookback} onChange={e => setVolDraft({...volDraft, lookback: Number(e.target.value)})} placeholder="Lookback"/>
            <input type="number" step={0.1} className="bg-surface-container border-outline-variant text-inverse-surface text-sm rounded focus:ring-primary-container col-span-2" value={volDraft.multiplier} onChange={e => setVolDraft({...volDraft, multiplier: Number(e.target.value)})} placeholder="Multiplier"/>
            <select className="bg-surface-container border-outline-variant text-inverse-surface text-sm rounded focus:ring-primary-container col-span-2" value={volDraft.interval} onChange={e => setVolDraft({...volDraft, interval: e.target.value as Interval})}><option value="M15">M15</option><option value="H1">H1</option><option value="H4">H4</option></select>
            <button onClick={() => addRule({ Volume: { ...volDraft } })} className="col-span-2 border border-primary-container text-primary-container hover:bg-primary-container hover:text-on-primary-container py-1 rounded transition-colors text-xs font-bold uppercase tracking-widest mt-2">+ Add Volume</button>
          </div>
        </div>
      </div>
    </div>
  );
}

// ─── Main App ─────────────────────────────────────────────────────────────────

type Tab = "basic" | "advanced" | "builder";

export default function App() {
  const [config, setConfig] = useState<UserStrategyConfig>(DEFAULT_CONFIG);
  const [activeTab, setActiveTab] = useState<Tab>("builder");

  const [sessionActive, setSessionActive] = useState(false);
  const [statusMessage, setStatusMessage] = useState<string>("");
  const [errorMessage, setErrorMessage] = useState<string>("");

  const [systemStatus, setSystemStatus] = useState<SystemStatus | null>(null);
  const [tradeHistory, setTradeHistory] = useState<TradeRecordDto[]>([]);

  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const poll = useCallback(async () => {
    try {
      const [status, trades] = await Promise.all([
        invoke<SystemStatus>("get_system_status"),
        invoke<TradeRecordDto[]>("get_trade_history"),
      ]);
      setSystemStatus(status);
      setTradeHistory(trades);
      if (!status.session_active && sessionActive) {
        setSessionActive(false);
        setStatusMessage("Session ended (detected via poll).");
      }
    } catch (err) {
      console.warn("[poll]", err);
    }
  }, [sessionActive]);

  useEffect(() => {
    if (sessionActive) {
      poll();
      pollRef.current = setInterval(poll, 4000);
    } else {
      if (pollRef.current) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
      poll();
    }
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, [sessionActive, poll]);

  async function handleStartSession() {
    setErrorMessage("");
    setStatusMessage("Starting session…");
    try {
      const result = await invoke<string>("start_mock_session", { config });
      setSessionActive(true);
      setStatusMessage(result);
    } catch (err) {
      setErrorMessage(typeof err === "string" ? err : String(err));
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
      await poll();
    } catch (err) {
      setErrorMessage(typeof err === "string" ? err : String(err));
    }
  }

  const winRatePct = systemStatus ? (Number(systemStatus.daily_stats.win_rate) * 100).toFixed(1) + "%" : "—";
  const dailyLossPct = systemStatus ? (Number(systemStatus.daily_loss_pct) * 100).toFixed(2) + "%" : "—";

  return (
    <>
      {/* TopAppBar */}
      <header className="bg-[#121212]/80 backdrop-blur-[20px] shadow-[0_4px_12px_rgba(0,0,0,0.5)] border-b border-white/10 w-full z-50 sticky top-0 h-16 flex justify-between items-center px-6 max-w-full">
        <div className="flex items-center gap-lg h-full">
          <span className="text-sm font-black tracking-widest text-white italic mr-4">QUANT_SANDBOX</span>
          <nav className="hidden md:flex items-end h-full gap-lg font-['Inter'] text-xs font-medium tracking-tight uppercase">
            <button 
              className={`pb-4 transition-all duration-200 active:scale-[0.97] ${activeTab === 'basic' ? 'text-[#0070FF] border-b-2 border-[#0070FF]' : 'text-gray-400 hover:text-white hover:bg-white/5'}`} 
              onClick={() => setActiveTab('basic')}>Basic Risk</button>
            <button 
              className={`pb-4 transition-all duration-200 active:scale-[0.97] ${activeTab === 'advanced' ? 'text-[#0070FF] border-b-2 border-[#0070FF]' : 'text-gray-400 hover:text-white hover:bg-white/5'}`} 
              onClick={() => setActiveTab('advanced')}>Advanced Management</button>
            <button 
              className={`pb-4 transition-all duration-200 active:scale-[0.97] ${activeTab === 'builder' ? 'text-[#0070FF] border-b-2 border-[#0070FF]' : 'text-gray-400 hover:text-white hover:bg-white/5'}`} 
              onClick={() => setActiveTab('builder')}>Strategy Builder</button>
          </nav>
        </div>
        <div className="flex items-center gap-md">
          <button 
            className="bg-secondary-container text-on-secondary-container px-4 py-2 rounded font-label-caps text-label-caps hover:bg-secondary-fixed-dim transition-colors shadow-[0_0_10px_rgba(47,248,1,0.2)] disabled:opacity-50 disabled:cursor-not-allowed"
            onClick={handleStartSession}
            disabled={sessionActive}
          >START MOCK TRADING</button>
          <button 
            className="border border-outline-variant text-on-surface-variant px-4 py-2 rounded font-label-caps text-label-caps hover:bg-white/5 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            onClick={handleStopSession}
            disabled={!sessionActive}
          >STOP SESSION</button>
          <div className="h-6 w-px bg-outline-variant mx-2"></div>
          <button className="text-on-surface-variant hover:text-white transition-colors"><span className="material-symbols-outlined text-[20px]">sensors</span></button>
          <button className="text-on-surface-variant hover:text-white transition-colors"><span className="material-symbols-outlined text-[20px]">settings</span></button>
          <button className="text-on-surface-variant hover:text-white transition-colors"><span className="material-symbols-outlined text-[20px]">account_circle</span></button>
        </div>
      </header>
      
      <div className="flex flex-1 pt-16 -mt-16">
        {/* SideNavBar */}
        <aside className="fixed left-0 top-16 bottom-0 z-40 flex flex-col justify-between bg-[#0A0A0A] h-[calc(100vh-64px)] w-16 hover:w-60 transition-all duration-300 ease-in-out border-r border-white/5 overflow-hidden group">
          <div className="flex flex-col py-lg gap-sm px-2">
            <a className="flex items-center gap-4 px-3 py-3 rounded-DEFAULT bg-[#0070FF]/10 text-[#0070FF] border-r-2 border-[#0070FF] transition-all duration-300 ease-in-out" href="#">
              <span className="material-symbols-outlined">dashboard</span>
              <span className="font-['Roboto_Mono'] text-[11px] uppercase tracking-tighter opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap">Dashboard</span>
            </a>
            <a className="flex items-center gap-4 px-3 py-3 rounded-DEFAULT text-gray-500 hover:text-gray-200 hover:bg-[#1E1E1E] transition-all duration-300 ease-in-out" href="#">
              <span className="material-symbols-outlined">terminal</span>
              <span className="font-['Roboto_Mono'] text-[11px] uppercase tracking-tighter opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap">Terminals</span>
            </a>
            <a className="flex items-center gap-4 px-3 py-3 rounded-DEFAULT text-gray-500 hover:text-gray-200 hover:bg-[#1E1E1E] transition-all duration-300 ease-in-out" href="#">
              <span className="material-symbols-outlined">history</span>
              <span className="font-['Roboto_Mono'] text-[11px] uppercase tracking-tighter opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap">Backtest</span>
            </a>
            <a className="flex items-center gap-4 px-3 py-3 rounded-DEFAULT text-gray-500 hover:text-gray-200 hover:bg-[#1E1E1E] transition-all duration-300 ease-in-out" href="#">
              <span className="material-symbols-outlined">analytics</span>
              <span className="font-['Roboto_Mono'] text-[11px] uppercase tracking-tighter opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap">Logs</span>
            </a>
          </div>
          <div className="flex flex-col pb-lg px-2 gap-sm">
            <div className="mt-4 px-3 flex items-center gap-3 opacity-0 group-hover:opacity-100 transition-opacity border-t border-white/5 pt-4">
              <div className="w-8 h-8 rounded-full bg-surface-container-high flex items-center justify-center shrink-0">
                <span className="material-symbols-outlined text-[16px] text-on-surface-variant">person</span>
              </div>
              <div className="flex flex-col overflow-hidden">
                <span className="font-data-md text-data-md truncate">TRADER_01</span>
                <span className="font-label-caps text-label-caps text-on-surface-variant truncate">PRO_ACCOUNT</span>
              </div>
            </div>
          </div>
        </aside>

        {/* Main Content Canvas */}
        <main className="flex-1 ml-16 p-lg bg-surface-dim min-h-[calc(100vh-64px)] overflow-y-auto">
          {errorMessage && <div className="mb-4 bg-error-container text-on-error-container p-sm rounded text-body-sm font-bold border border-error">Error: {errorMessage}</div>}
          {statusMessage && <div className="mb-4 bg-surface-container border border-white/10 p-sm rounded text-on-surface text-body-sm">{statusMessage}</div>}

          {/* Top Control Panel / Status */}
          <div className="flex flex-col xl:flex-row gap-gutter mb-lg">
            <div className="grid grid-cols-2 md:grid-cols-3 xl:grid-cols-6 gap-sm flex-1">
              <div className="bg-surface-container/60 backdrop-blur-[12px] border border-white/5 rounded-lg p-sm flex flex-col justify-between h-20 shadow-[0_4px_12px_rgba(0,0,0,0.1)]">
                <span className="font-body-sm text-body-sm text-on-surface-variant">Equity</span>
                <span className="font-data-lg text-data-lg text-inverse-surface text-right">${systemStatus ? Number(systemStatus.current_equity).toLocaleString(undefined, { minimumFractionDigits: 2 }) : "—"}</span>
              </div>
              <div className="bg-surface-container/60 backdrop-blur-[12px] border border-white/5 rounded-lg p-sm flex flex-col justify-between h-20 shadow-[0_4px_12px_rgba(0,0,0,0.1)]">
                <span className="font-body-sm text-body-sm text-on-surface-variant">Daily P&L</span>
                <span className={`font-data-lg text-data-lg text-right ${systemStatus && Number(systemStatus.daily_stats.realized_pnl) >= 0 ? "text-secondary-container" : "text-tertiary-container"}`}>
                  {systemStatus ? (Number(systemStatus.daily_stats.realized_pnl) >= 0 ? "+" : "") + "$" + Number(systemStatus.daily_stats.realized_pnl).toFixed(2) : "—"}
                </span>
              </div>
              <div className="bg-surface-container/60 backdrop-blur-[12px] border border-white/5 rounded-lg p-sm flex flex-col justify-between h-20 shadow-[0_4px_12px_rgba(0,0,0,0.1)]">
                <span className="font-body-sm text-body-sm text-on-surface-variant">Daily Loss Limit</span>
                <div className="w-full bg-surface-container-highest h-1.5 rounded-full mt-2 mb-1 overflow-hidden">
                  <div className={`h-1.5 rounded-full ${systemStatus && Number(systemStatus.daily_loss_pct) > 0.04 ? "bg-tertiary-container" : "bg-primary-container"}`} style={{ width: systemStatus ? `${Math.min(100, Number(systemStatus.daily_loss_pct) * 100)}%` : "0%" }}></div>
                </div>
                <span className="font-data-md text-data-md text-inverse-surface text-right">{dailyLossPct}</span>
              </div>
              <div className="bg-surface-container/60 backdrop-blur-[12px] border border-white/5 rounded-lg p-sm flex flex-col justify-between h-20 shadow-[0_4px_12px_rgba(0,0,0,0.1)]">
                <span className="font-body-sm text-body-sm text-on-surface-variant">Trades</span>
                <span className="font-data-lg text-data-lg text-inverse-surface text-right">{systemStatus ? systemStatus.daily_stats.trade_count : "—"}</span>
              </div>
              <div className="bg-surface-container/60 backdrop-blur-[12px] border border-white/5 rounded-lg p-sm flex flex-col justify-between h-20 shadow-[0_4px_12px_rgba(0,0,0,0.1)]">
                <span className="font-body-sm text-body-sm text-on-surface-variant">Win Rate</span>
                <span className="font-data-lg text-data-lg text-inverse-surface text-right">{winRatePct}</span>
              </div>
              <div className="bg-surface-container/60 backdrop-blur-[12px] border border-white/5 rounded-lg p-sm flex flex-col justify-between h-20 shadow-[0_4px_12px_rgba(0,0,0,0.1)]">
                <span className="font-body-sm text-body-sm text-on-surface-variant">Circuit Breaker</span>
                <span className={`font-data-md text-data-md px-2 py-1 rounded w-max self-end mt-1 ${systemStatus?.circuit_breaker_active ? "text-tertiary-container bg-tertiary-container/10" : "text-secondary-container bg-secondary-container/10"}`}>
                  {systemStatus?.circuit_breaker_active ? "TRIPPED" : "ARMED"}
                </span>
              </div>
            </div>
            <div className={`flex items-center bg-surface-container/60 backdrop-blur-[12px] border border-white/5 rounded-lg px-md h-20 shrink-0 gap-sm ${!sessionActive && 'opacity-50 grayscale'}`}>
              <div className={`w-3 h-3 rounded-full ${sessionActive ? 'bg-secondary-container shadow-[0_0_8px_rgba(47,248,1,0.6)] animate-pulse' : 'bg-outline-variant'}`}></div>
              <span className={`font-label-caps text-label-caps tracking-widest ${sessionActive ? 'text-secondary-container' : 'text-outline-variant'}`}>{sessionActive ? 'LIVE' : 'IDLE'}</span>
            </div>
          </div>

          {/* Strategy Builder Workspace */}
          <div className="bg-surface-container/60 backdrop-blur-[12px] border border-white/5 rounded-lg flex flex-col mb-lg shadow-[0_4px_12px_rgba(0,0,0,0.1)]">
            <div className="border-b border-white/5 p-md flex items-center justify-between">
              <h2 className="font-headline-md text-headline-md text-inverse-surface flex items-center gap-2 text-[18px]">
                <span className="material-symbols-outlined text-[20px] text-primary-container">account_tree</span>
                {activeTab === 'basic' ? "Basic Risk Config" : activeTab === 'advanced' ? "Advanced Config" : "Logic Builder"}
              </h2>
            </div>
            
            {activeTab === 'basic' && <BasicRiskTab config={config} onChange={setConfig} />}
            {activeTab === 'advanced' && <AdvancedTab config={config} onChange={setConfig} />}
            {activeTab === 'builder' && <StrategyBuilderTab config={config} onChange={setConfig} />}
          </div>

          {/* Bottom Section: Trade History */}
          <div className="bg-surface-container/60 backdrop-blur-[12px] border border-white/5 rounded-lg flex flex-col shadow-[0_4px_12px_rgba(0,0,0,0.1)]">
            <div className="border-b border-white/5 p-md flex justify-between items-center">
              <h3 className="font-headline-md text-headline-md text-inverse-surface text-base flex items-center gap-2">
                <span className="material-symbols-outlined text-[20px] text-on-surface-variant">table_rows</span>
                Session Ledger {sessionActive && <span className="text-secondary-container text-xs ml-2"> (Polling)</span>}
              </h3>
            </div>
            <div className="overflow-x-auto">
              <table className="w-full text-left border-collapse">
                <thead>
                  <tr className="border-b border-white/5 bg-surface-container-low/50">
                    <th className="p-sm font-label-caps text-label-caps text-on-surface-variant font-medium">TID</th>
                    <th className="p-sm font-label-caps text-label-caps text-on-surface-variant font-medium">SYM</th>
                    <th className="p-sm font-label-caps text-label-caps text-on-surface-variant font-medium">DIR</th>
                    <th className="p-sm font-label-caps text-label-caps text-on-surface-variant font-medium text-right">ENTRY</th>
                    <th className="p-sm font-label-caps text-label-caps text-on-surface-variant font-medium text-right">EXIT</th>
                    <th className="p-sm font-label-caps text-label-caps text-on-surface-variant font-medium">PHASE</th>
                    <th className="p-sm font-label-caps text-label-caps text-on-surface-variant font-medium text-right pr-lg">P&L</th>
                  </tr>
                </thead>
                <tbody className="font-data-md text-data-md text-inverse-surface">
                  {tradeHistory.length === 0 ? (
                    <tr><td colSpan={7} className="p-md text-center text-on-surface-variant text-sm">No trades recorded yet. Start a session to begin.</td></tr>
                  ) : (
                    tradeHistory.map(trade => {
                      const pnl = Number(trade.realized_pnl);
                      return (
                        <tr key={trade.id} className="border-b border-white/5 hover:bg-[#1E1E1E] transition-colors">
                          <td className="p-sm text-on-surface-variant">{trade.id.slice(0, 8)}…</td>
                          <td className="p-sm">{trade.symbol}</td>
                          <td className="p-sm">
                            <span className={`px-2 py-0.5 rounded text-[11px] ${trade.direction === 'Long' ? 'bg-secondary-container/10 text-secondary-container' : 'bg-tertiary-container/10 text-tertiary-container'}`}>
                              {trade.direction.toUpperCase()}
                            </span>
                          </td>
                          <td className="p-sm text-right">{Number(trade.entry_price).toFixed(2)}</td>
                          <td className="p-sm text-right">{trade.exit_price ? Number(trade.exit_price).toFixed(2) : "-"}</td>
                          <td className={`p-sm ${trade.status === 'Open' ? 'text-primary-container' : 'text-on-surface-variant'}`}>{trade.status === 'Open' ? 'OPEN' : 'CLOSED'}</td>
                          <td className={`p-sm text-right pr-lg ${pnl > 0 ? 'text-secondary-container' : pnl < 0 ? 'text-tertiary-container' : 'text-on-surface-variant'}`}>
                            {pnl === 0 ? "—" : (pnl > 0 ? "+" : "") + pnl.toFixed(2)}
                          </td>
                        </tr>
                      );
                    })
                  )}
                </tbody>
              </table>
            </div>
          </div>
        </main>
      </div>
    </>
  );
}
