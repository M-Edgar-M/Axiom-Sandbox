# 🏗️ Axiom Trading Sandbox

High-Level Technical Specifications: Open-Source Trading Sandbox

## 1. Architectural Overview

- **Framework**: Tauri (React/TypeScript for the frontend, Rust for the backend)
- **Deployment**: Local desktop application (Windows, macOS, Linux)
- **Core Philosophy**: *"Shared Brain, Decoupled Execution."* The user defines the rules in the UI, and the Rust backend manages the data ingestion, risk constraints, and order execution without cloud database dependencies.

## 2. Core Backend Modules (Reused from Boilerplate)

To minimize redundant coding, the sandbox will directly recycle the infrastructure of your existing Rust trading engine:

### Data Pipeline (WebSocketManager)
Handles real-time market data ingestion, maintaining the existing dual-stream management, proactive 23-hour rotation, and circuit breakers.

### Execution Arm (ExecutionEngine & IbkrExecutionEngine)
Handles order routing for both crypto (Binance) and TradFi (Interactive Brokers) via mock and live modes. Enforces exchange-side native OCO (One-Cancels-Other) orders to limit slippage.

### Strategy Management (TradeManager & RiskManager)
Continues to orchestrate trade lifecycles, executing actions like moving stops to breakeven, taking partial profits (e.g., 33%), and activating ATR-based trailing stops.

### State Persistence (CsvLogger)
Because there is no cloud backend, local state persistence is handled entirely via `trades.csv`. On application boot, the app reads the CSV to reconcile local state with the exchange's live orders.

## 3. The New "Rule Evaluator" Engine (Replaces AnalysisEngine)

To transition from a proprietary trading bot to a user-driven sandbox, the hardcoded AnalysisEngine must be entirely decoupled.

- **Mechanism**: The React frontend will feature a "Rule Builder" interface (e.g., drag-and-drop or IF/THEN statements)
- **Data Passing**: The UI compiles these custom rules into a JSON object and passes it via Tauri IPC (Inter-Process Communication) to the Rust backend
- **Execution**: A newly built Rule Evaluator module in Rust listens to the WebSocketManager. Whenever the live OHLCV data matches the user's custom JSON conditions, the Rule Evaluator triggers the ExecutionEngine to enter the trade

## 4. Exposed Indicator Library

The sandbox will expose your existing `yata` Rust library indicators to the frontend, allowing users to build strategies using:

- **Trend Indicators**: Moving Averages (SMA, EMA)
- **Oscillators & Momentum**: Relative Strength Index (RSI), Average True Range (ATR), Average Directional Index (ADX)
- **Volume Indicators**: Volume Multipliers (compared to SMA)

## 5. Mandatory Risk & System Guardrails

Because retail users often over-leverage or fall victim to home-computing realities, the sandbox imposes strict, hardcoded guardrails:

### The "Dead Man's Switch"
All stops and targets are routed directly to the exchange via OCO orders. If the user's local Wi-Fi drops or their computer crashes, their capital is still protected on the exchange.

### Forced Risk Ceilings
The UI will strictly cap the Fixed Fractional Risk (RISK_PER_TRADE) parameter between **0.5% and 5.0%**. Users cannot risk 50% of their account on a single trade.

### Daily Circuit Breaker
The DAILY_LOSS_LIMIT parameter will automatically halt the bot's execution for 24 hours if a predefined drawdown threshold is breached.

### Uptime Warning
The UI must feature a prominent warning banner instructing users that the application must remain open and their PC cannot go to sleep mode, paired with a guide suggesting the use of a Virtual Private Server (VPS).

## 6. Trade Management Customization (User Configuration)

While users dictate their own entry rules, they can use the "Advanced Tab" in the React UI to customize how the Rust engine manages their open trades:

### Time-Based Exits
Automatically close trades after N-closed bars if profit targets are not met.

### Trailing Stops
Define the activation threshold (e.g., at 2R profit) and the trailing tightness (e.g., 2.0x ATR).

### Systematic Volatility Protections
Toggles to automatically halve position sizes if the market enters extreme volatility regimes or if trading highly illiquid small-cap assets.

---

**Status**: Work in Progress | **Last Updated**: April 20, 2026