🏗️ High-Level Technical Specifications: Open-Source Trading Sandbox
1. Architectural Overview
Framework: Tauri (React/TypeScript for the frontend, Rust for the backend)
.
Deployment: Local desktop application for Windows, macOS, and Linux
.
Core Philosophy: "Shared Brain, Decoupled Execution." The user defines the entry rules in the React UI, while the Rust backend manages data ingestion, risk constraints, and order execution without relying on cloud database dependencies
.
2. Core Backend Modules (Reused Boilerplate)
To minimize redundant coding, the sandbox directly recycles the infrastructure of the existing Rust trading engine
:
Data Pipeline (WebSocketManager): Handles real-time market data ingestion, maintaining dual-stream management, proactive 23-hour rotation, and circuit breakers
.
Execution Arm (ExecutionEngine): Handles order routing via mock and live modes
.
Strategy Management (TradeManager & RiskManager): Orchestrates trade lifecycles, executing actions like moving stops to breakeven, taking partial profits, and activating ATR-based trailing stops
.
State Persistence (CsvLogger): Because there is no cloud database, local state persistence is handled entirely via trades.csv. On application boot, the app reads the CSV to reconcile local state with the exchange's live orders
.
3. The "Rule Evaluator" Engine (The Decoupled "Brain")
The hardcoded proprietary analysis engine is replaced to give users total freedom
:
Mechanism: The React frontend features a "Strategy Builder" (IF/THEN rule builder).
Data Passing: The UI compiles these custom rules into a JSON payload and passes it via Tauri IPC (Inter-Process Communication) to the Rust backend
.
Execution: A newly built Rule Evaluator module in Rust listens to the WebSocketManager. Whenever the live OHLCV data evaluates to true against the user's custom JSON conditions, it triggers the ExecutionEngine to enter the trade
.
4. UI Configurations & Strategy Parameters
The React frontend is divided into three distinct tabs to balance user freedom with mathematical safety:
⚙️ TAB 1: Basic (Risk & Style)
Fixed Fractional Risk: Slider capped strictly between 0.5% and 5.0%
.
Daily Circuit Breaker: Input to halt trading for 24 hours if a specific drawdown is reached
.
Profit Taking Aggression: Dropdown for 0% (trend follower), 33%, or 50% (conservative scalper) at Target 1
.
Trend Speed Preference: Dropdown for structural filters (Fast EMA 21, Medium EMA 25, Slow EMA 34)
.
🔧 TAB 2: Advanced (Quantitative Management)
Minimum R:R Threshold: Restricts trades unless the payout is between 1.5 and 3.0
.
Time-Based Exits: Closes trades after N-closed bars if targets are not met
.
Trailing Stops: Defines activation (e.g., 2R) and ATR multiplier tightness (1.5x - 3.0x)
.
Volatility & Small-Cap Protections: Toggles to cut position size by 50% in extreme conditions
.
🧩 TAB 3: Strategy Builder (Indicator Library)
RSI: Lookback periods, Overbought/Oversold thresholds, and Cross Over/Under logic.
Moving Averages: SMA/EMA selection, Lookback periods, and Crossover/Price-close logic.
Volume: Institutional multiplier thresholds (e.g., Volume > 1.2x SMA) and baseline lookback periods.
5. Mock Trading & Binance Testnet
The "Mock Trade" Interface: A prominent UI button allows users to paper-trade their custom strategy in real-time before risking capital.
Backend Integration: Connects seamlessly to Binance's actual matching engine by setting the testnet: true flag inside the engine's EngineConfig
. This routes all API calls to https://testnet.binancefuture.com using the user's testnet API keys, providing the most realistic simulation of slippage and execution latency possible.
6. Mandatory Security Standards & System Guardrails
To protect retail users from malicious attacks and the realities of home-computing, the following security standards are strictly enforced:
Secure Credential Management: User API keys are stored securely locally. Documentation explicitly warns users to never enable "Withdrawal" permissions on their exchange API keys.
The "Dead Man's Switch": All stops and targets are routed directly to the exchange via OCO (One-Cancels-Other) orders
. If the user's local Wi-Fi drops or the computer crashes, their capital is still completely protected on the exchange
.
Anti-Badware & Uptime Environment: The UI prominently warns users that their PC cannot go to sleep mode. Documentation advises using a hardware firewall and provides a deployment guide for hosting the application on a Virtual Private Server (VPS) for 99.9% uptime
.
Anonymized Execution Logs: Sensitive data (like raw API keys) are strictly sanitized and never printed to the application's UI console or saved into plaintext CSV log files.
