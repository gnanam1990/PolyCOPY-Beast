# ◈ SuperFast PolyBot v3.0

> **Self-hosted, high-performance copy-trading system for Polymarket CLOB**  
> Zero heavy dependencies · Windows-native · Sub-550ms latency

---

## Overview

**SuperFast PolyBot v3.0** is a production-grade, Rust-powered copy-trading engine built specifically for Polymarket's Central Limit Order Book (CLOB).

After deep research into 15+ active open-source repos and the official Polymarket SDK, this bot delivers a realistic, low-latency solution for solo developers running on Windows — with **zero Docker, zero Redis, and zero cloud dependencies**.

### What it does
- Monitors target wallets in real-time via **WebSocket** + **Data API polling**
- Auto-executes copy trades with **FOK** or **GTC** orders
- Applies a **multiplier risk engine** (confidence × secret × drawdown)
- Tracks PnL, positions, and daily stats in an embedded **SQLite** database
- Serves a live **dark-themed WASM dashboard**
- Offers **Telegram remote control** with whitelisted auth

---

## ✨ Key Features

| Feature | Description |
|---------|-------------|
| ◈ **Dual Signal Ingestion** | WebSocket user channel + Data API polling (2s) with tx-hash deduplication |
| ⚡ **Sub-550ms Latency** | Realistic end-to-end target after Polymarket's Feb 2026 taker-delay removal |
| 🛡️ **Risk Engine** | Dynamic sizing, drawdown multipliers, circuit breakers, daily loss halt |
| 💻 **Native Windows** | Pure `cargo run --release`. No Docker, no Redis, no WSL needed |
| 📊 **Dark Dashboard** | Real-time Leptos/WASM frontend with live PnL, positions, signals, and system health |
| 🤖 **Telegram Bot** | Chat-ID whitelist, 2-step confirmation for all destructive actions |
| 💾 **SQLite Persistence** | Full trade, position, signal, and daily-stats journaling |
| 🧪 **Simulation Mode** | 100% no-op safe mode for full validation before live trading |
| 🔧 **Operator Controls** | Dashboard-native Pause / Resume / Emergency Stop with confirmation dialogs |

---

## 🏗️ Architecture

```
┌────────────────────────────────────────────────────────────────┐
│                    SuperFast PolyBot v3.0                        │
├──────────────────┐    ┌───────────────────────────────────────┤
│ Signal Ingestion │    │        Risk & Sizing Engine           │
│ ◈ WS User Channel├────┤◈ Confidence × Secret × Drawdown      │
│ ◈ Data API (2s)  │    │◈ Clamp($5, max_position)               │
└────────┬─────────┘    └───────────────┬───────────────────────┤
         │                              │                       │
         ▼                              ▼                       │
┌──────────────────┐    ┌───────────────────────────────────────┤
│ Wallet Tracker   │    │       CLOB Execution Engine           │
│ ◈ Multi-target   │    │◈ Fetch price → size → EIP-712 sign   │
│ ◈ Performance    │    │◈ FOK/GTC → POST /order → confirm fill│
│   scoring        │    └───────────────┬───────────────────────┤
└──────────────────┘                    │                       │
         │           ┌──────────────────┤                       │
         ▼           ▼                  ▼                       │
┌──────────────────┐    ┌───────────────────────────────────────┤
│ Dark Leptos Dash │    │         SQLite (`polybot.db`)         │
│ ◈ Live PnL       │    │◈ signals / trades / positions         │
│ ◈ Positions      │    │◈ targets / daily_stats / config         │
│ ◈ System Health  │    └───────────────────────────────────────┘
│ ◈ Operator       │                                      │
│   Controls       │    ┌───────────────────────────────────────┤
└──────────────────┘    │  Telegram Bot (whitelist + confirm) │
                        └───────────────────────────────────────┘
```

---

## 🚀 Quick Start

### Prerequisites
- [Rust](https://rustup.rs/) 1.70+ stable toolchain
- [Trunk](https://trunkrs.dev/) for the WASM frontend
- A Polygon RPC endpoint (free public ones work for simulation)

### 1. Clone & Build
```bash
git clone <repo-url>
cd polybot

cargo build --release
```

### 2. Configure Environment
Copy `.env.example` to `.env` and fill in your values:
```bash
cp .env.example .env
```

#### Required
- `POLYMARKET_PRIVATE_KEY` — Your EOA private key (with `0x` prefix)
- `POLYGON_RPC_URL` — e.g. `https://polygon-rpc.com`

#### Trading Setup
- `TARGET_WALLETS` — Comma-separated addresses to copy
- `POSITION_MULTIPLIER` — Copy scale (e.g. `0.1` for 10%)

#### Optional
- `TELEGRAM_BOT_TOKEN` — From @BotFather
- `TELEGRAM_ALLOWED_CHAT_IDS` — Your Telegram user ID

> ⚠️ **Always start with `SIMULATION_MODE=true`**. Flip to `false` only after validating behavior.

Full `.env` reference is documented in the [PRD](SuperFast_PolyBot_v3_PRD_Enhanced.md).

### 3. Initialize the Database
The SQLite database auto-initializes on first run. No manual setup needed.

### 4. Run the Backend
```bash
cd polybot-core
cargo run --release
```

Expected startup log:
```json
{"level":"INFO","message":"SuperFast PolyBot v3 starting","simulation":true}
{"level":"INFO","message":"Running in SIMULATION mode — no real orders will be placed"}
{"level":"INFO","message":"Health/metrics server starting on 0.0.0.0:8080"}
{"level":"INFO","message":"HTTP ingestion server starting on 0.0.0.0:8081"}
```

### 5. Run the Dashboard
Open a **new terminal**:
```bash
cd polybot-dashboard
trunk serve
# → http://127.0.0.1:8082
```

Open your browser to **`http://127.0.0.1:8082`**.

---

## 📊 Dashboard Features

The dashboard is a **WebAssembly single-page app** built with Leptos, featuring a dark command-center aesthetic.

| Panel | Metrics |
|-------|---------|
| **Stats Grid** | Portfolio Balance, Daily PnL, Open Positions, Signals Received |
| **System Health** | WebSocket status, RPC health, uptime, last signal, emergency stops, drawdown bar |
| **Execution Summary** | Signals processed/skipped, trades executed, PnL, drawdown % |
| **Operator Controls** | Pause / Resume / Emergency Stop with native browser confirmations |
| **Open Positions** | Sortable table: Market, Side (tagged), Avg Price, Size, Category, Status |
| **Recent Signals** | Sortable table: Market, Side, Confidence, Secret, Category, Disposition |

- **Auto-refresh**: Every 5 seconds
- **Responsive**: Collapses to stacked layout on mobile
- **Color-coded tags**: Green=Buy/Yes/Open, Red=Sell/No, Blue=Politics, Orange=Crypto

---

## 🔌 API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/` | GET | Live dashboard HTML |
| `/health` | GET | JSON health snapshot (mode, balance, PnL, drawdown, WS status, etc.) |
| `/metrics` | GET | Prometheus-style metrics for Grafana ingestion |
| `/positions` | GET | JSON array of open positions |
| `/signals` | GET | JSON array of recent signals (`?limit=N`) |
| `/executions` | GET | JSON array of recent trades |
| `/control/pause` | POST | Pause all new trading |
| `/control/resume` | POST | Resume trading |
| `/control/emergency-stop` | POST | Flatten all positions + halt |

---

## 📁 Project Structure

```
├── polybot-common/        # Shared types, errors, constants
├── polybot-core/          # Main backend application
│   ├── src/config.rs      # .env / config.toml loader
│   ├── src/scanner/       # Data API + WebSocket ingestion
│   ├── src/risk/          # Sizing engine + limits
│   ├── src/execution/     # CLOB client + order builder
│   ├── src/state/         # SQLite persistence + reconciliation
│   ├── src/telegram_bot/  # Telegram commands + alerts
│   └── src/health.rs      # Axum dashboard + metrics server
├── polybot-dashboard/     # Leptos WASM frontend
│   ├── src/app.rs         # Main UI layout
│   ├── src/data.rs        # API fetchers
│   └── style.css          # Dark theme design system
├── signals/               # File-drop signal input (optional)
├── SuperFast_PolyBot_v3_PRD_Enhanced.md
└── .env
```

---

## 🛡️ Safety & Risk

| Guard | Behavior |
|-------|----------|
| **Simulation Mode** | All execution is no-op. Safe for 100% of testing. |
| **Daily Loss Limit** | Auto-pause when drawdown hits 5% (configurable) |
| **Circuit Breaker** | Pause after 5 consecutive losses (configurable) |
| **Min Balance** | Halt if USDC balance falls below $20 |
| **Anti-Duplication** | One position owner per token — never double-trade |
| **Staleness Guard** | Ignore signals older than 30s or from resolved markets |
| **Emergency Stop** | Dashboard + Telegram — flattens all positions immediately |

---

## 🧪 Development

### Run in Simulation
```bash
cd polybot-core
cargo run --release
# No real transactions. Validates entire pipeline safely.
```

### Run Tests
```bash
cargo test --workspace
```

### Build Dashboard Only
```bash
cd polybot-dashboard
trunk build --release
```

### Linting
```bash
cargo clippy --workspace -- -D warnings
```

---

## 🐛 Troubleshooting

| Issue | Fix |
|-------|-----|
| `Polygon RPC 401` | Your RPC requires auth. Use a public endpoint or add API key. |
| `429 Too Many Requests` | RPC rate-limited. Add `POLYGON_RPC_URL` backup or slow polling. |
| Dashboard blank | Ensure `polybot-core` is running on port 8080. Trunk proxies to it. |
| `Telegram bot not started` | Token is missing or invalid. Check `.env`. |
| Port 8080 in use | Change `DASHBOARD_PORT` in `.env` or kill the existing process. |

---

## 📜 License & Disclaimer

This software executes real financial transactions on the Polygon blockchain. All trading involves risk of loss.

Features like `SIMULATION_MODE`, daily loss limits, and circuit breakers are provided to reduce unintended exposure — **but they do not eliminate risk**.

**Start with the minimum viable capital ($100–200) and validate all behavior in simulation mode before going live.**

---

Built with ◈ **Rust**, ◈ **Tokio**, ◈ **Leptos**, and ◈ **aggression**.
