🏗️ Algorithmic Trading Sandbox
📖 Project Overview
The Algorithmic Trading Sandbox is a high-performance, open-source desktop application designed to empower retail traders to build, test, and automate their own quantitative trading strategies
.
Built on a "Shared Brain, Decoupled Execution" philosophy, the application provides a user-friendly Integrated Development Environment (IDE) where traders can define custom entry and exit rules
. These rules are processed by a decoupled Rust backend that automatically manages real-time data ingestion, strict risk constraints, and order routing, entirely eliminating the need for cloud database dependencies
.
By forcing institutional-grade risk guardrails and providing native mock-trading capabilities, the sandbox protects users from over-leveraging and local hardware failures while they explore algorithmic trading
.
💻 Technical Stack
Frontend Framework: React and TypeScript, packaged as a lightweight desktop application using Tauri
.
Backend Execution Engine: Rust (utilizing tokio for asynchronous event-driven architecture)
.
Data Pipeline: A custom WebSocketManager handling dual-stream real-time market data (OHLCV) with proactive 23-hour rotation and circuit breakers
.
State Persistence: Local, cloud-free state management using a Rust-based CsvLogger (trades.csv) that reconciles application state upon boot
.
Exchange Integration: Binance USD(S)-M Futures (with full support for the Binance Testnet matching engine)
.
✨ Core Features
The Rule Evaluator: A highly decoupled module that ingests custom JSON rule payloads from the React UI (e.g., RSI and Moving Average crossovers) and triggers automated market executions
.
The "Dead Man's Switch": To protect against local Wi-Fi drops or PC crashes, all stop-loss and take-profit targets are routed directly to the exchange via native OCO (One-Cancels-Other) orders
.
Hardcoded Risk Guardrails: The engine mathematically restricts maximum risk per trade (capped at 5.0%) and enforces a 24-hour circuit breaker if daily loss limits are breached
.
Zero-Risk Mock Trading: A dedicated testing environment that routes strategies directly to the Binance Testnet, providing highly realistic slippage and execution latency simulations before risking real capital
.
Secure Local Execution: API credentials never leave the user's local machine, adhering to strict cybersecurity standards

