# SuperFast PolyBot

> A self-hosted Polymarket copy-trading command center, built in Rust. Paper-ready by default, live-gated by design.

## Overview

SuperFast PolyBot is a self-hosted copy-trading system for Polymarket's CLOB. It watches target wallets, turns observed trades into risk-scored copy signals, simulates or submits CLOB V2-shaped orders, persists the full lifecycle in SQLite, and gives the operator a local dashboard plus Telegram controls.

The project is intentionally practical and single-operator focused: no required hosted backend and no hidden SaaS dependency. The default mode is simulation, and live trading cannot happen accidentally — it requires explicit gating, credentials, and a passing startup preflight.

## Features

- Tracks target wallet activity through Polymarket Data API ingestion (polling and optional WebSocket).
- Deduplicates signals by transaction/hash and rejects stale or unsafe signals.
- Scores each copy attempt through confidence, secret level, drawdown, exposure, liquidity, category caps, and minimum-balance rules.
- Builds fee-aware CLOB V2-style order plans using maker-first GTC behavior and FOK only under a configured fee ceiling.
- Simulates relayer transaction states and stores V2 transaction records in SQLite.
- Tracks virtual pUSD, reserved pUSD, fees, rebates, positions, daily stats, and recent signals.
- Serves a local operator dashboard and JSON/metrics API on port `8080`.
- Exposes pause, resume, and emergency-stop controls guarded by a control key.
- Supports Telegram operator commands with allowlisted users, confirmation safety, wallet management, and collateral plan previews for `/wrap` and `/redeem`.

## Tech stack

- **Rust** (edition 2021, Cargo workspace) with the **Tokio** async runtime.
- **axum** for the HTTP/WebSocket dashboard and API server.
- **rusqlite** (bundled SQLite) for the local system of record.
- **teloxide** for the Telegram bot.
- A Polymarket client SDK for CLOB, WebSocket, and Data API access, plus **alloy** for order/calldata signing types.
- **Leptos** (CSR/WASM) for the alternate dashboard app, built with **Trunk**.

## Architecture

This is a Cargo workspace with three crates:

- `polybot-common/` — shared domain types, constants, and errors (also compiles to WASM for the dashboard).
- `polybot-core/` — the main bot runtime: scanner, risk engine, execution, state, Telegram bot, and the served dashboard/API. This is the binary you run.
- `polybot-dashboard/` — a Leptos client-side dashboard app for development or an alternate UI.

High-level data flow inside `polybot-core`:

```text
Target Wallet Activity
        |
        v
Scanner + Deduplication        (src/scanner)
        |
        v
Risk Engine                    (src/risk)
  - confidence and secret multipliers
  - drawdown protection
  - exposure and liquidity caps
  - stale/resolved market guards
        |
        v
Execution Engine               (src/execution)
  - simulation relayer
  - V2 order payload/signing
  - relayer submit/poll client
  - pUSD accounting
        |
        v
SQLite System of Record        (src/state)
  - signals, trades, positions, copied lots, daily stats, V2 transactions
        |
        v
Operator Surfaces
  - local dashboard, health/metrics API (src/health.rs)
  - Telegram controls (src/telegram_bot)
```

## Getting started

### Prerequisites

- Rust stable toolchain (Cargo).
- A shell (PowerShell on Windows, or any shell on macOS/Linux).
- Optional: [Trunk](https://trunkrs.dev/) if you want to build the Leptos dashboard app.
- Optional: Docker / Docker Compose if you prefer a containerized run.

### Installation

Clone the repository and copy the environment template:

```bash
cp .env.example .env
```

On Windows PowerShell:

```powershell
Copy-Item .env.example .env
```

No further install step is needed — Cargo builds the workspace on first run.

### Configuration

Local defaults live in `config.toml`. Any value can be overridden by the corresponding environment variable; see `.env.example` for the full list. The variables below are the ones the project reads — list of **names and purpose only; never commit secret values**.

| Variable | Purpose |
|---|---|
| `POLYMARKET_PRIVATE_KEY` | Trading wallet private key (0x-prefixed). Required before live mode. |
| `POLYBOT_EXECUTION_MODE` | Execution mode: `simulation`, `shadow`, or `live`. |
| `POLYBOT_ENABLE_LIVE_V2` | Explicit safety gate for CLOB V2 live submission. Keep `false` for paper trading. |
| `POLYBOT_V2_VERIFY_CONDITION_ID` | Active market condition ID used by setup-check to verify V2 market/fee responses. |
| `POLYBOT_CLOB_ENDPOINT` | CLOB API endpoint. |
| `POLYBOT_SQLITE_PATH` | Path to the local SQLite database. |
| `POLYBOT_LOG_LEVEL` | Log level (default `info`). |
| `POLYBOT_DASHBOARD_CONTROL_KEY` | Key required for dashboard pause/resume/emergency-stop control routes. |
| `POLYBOT_BASE_SIZE_USD` | Override base position size in USD. |
| `POLYBOT_PAPER_STARTING_BALANCE_USD` | Paper-trading capital shown on the dashboard. |
| `POLYBOT_PAPER_FIXED_ENTRY_PRICE` | Fixed simulation fill price (deterministic paper fills). |
| `POLYBOT_API_KEY` | API key for the optional HTTP signal-ingestion server (blank disables it). |
| `POLYBOT_TELEGRAM_TOKEN` | Telegram bot token (optional). |
| `POLYBOT_TELEGRAM_ALLOWED_USER_IDS` | Comma-separated allowlist of Telegram user IDs. |
| `POLYBOT_REDIS_ENABLED` / `POLYBOT_REDIS_URL` | Optional Redis integration (disabled by default). |
| `RELAYER_URL` / `RELAYER_API_KEY` / `RELAYER_API_KEY_ADDRESS` | Gasless relayer config. All three required together for live mode. |
| `BUILDER_CODE` | V2 order-signing builder code (0x + 64 hex chars). |
| `COLLATERAL_TOKEN`, `PUSD_ADDRESS`, `COLLATERAL_ONRAMP_ADDRESS`, `COLLATERAL_OFFRAMP_ADDRESS`, `USDC_E_ADDRESS` | Collateral / pUSD wrapping addresses. |
| `POLYBOT_COLLATERAL_RECIPIENT_ADDRESS` | Wallet address used by `/wrap` plan previews when it cannot be derived locally. |
| `FOK_MAX_FEE_BPS` | Taker-fee ceiling (basis points) above which FOK orders are not issued. |
| `MAX_POSITION_POLITICS_USDC`, `MAX_POSITION_CRYPTO_USDC`, `MAX_POSITION_SPORTS_USDC`, `MAX_POSITION_OTHER_USDC` | Per-category position caps in USDC. |
| `POLYBOT_RECONCILIATION_AUTO_HEAL` | When `true`, reconciliation may overwrite in-memory positions to match the reference. Default off. |

A minimal paper-mode `.env` is enough to start safely:

```env
POLYBOT_EXECUTION_MODE=simulation
POLYBOT_SQLITE_PATH=./polybot.db
POLYBOT_LOG_LEVEL=info
POLYBOT_DASHBOARD_CONTROL_KEY=change-this-local-control-key
POLYBOT_PAPER_STARTING_BALANCE_USD=1000
POLYBOT_PAPER_FIXED_ENTRY_PRICE=0.50
```

### Running

Run the startup preflight check:

```bash
cargo run -p polybot-core -- --setup-check
```

Expected paper-mode result:

```text
Startup preflight completed successfully: mode=Simulation simulation_preflight=true
```

Start the bot:

```bash
cargo run -p polybot-core
```

Then open the dashboard at `http://127.0.0.1:8080`. The dashboard served by `polybot-core` is the main local operator surface.

Build the alternate Leptos dashboard app (optional):

```bash
cd polybot-dashboard
trunk build --release
```

Containerized run (optional) — note `docker-compose.yml` also starts a Redis service:

```bash
docker compose up --build
```

## Usage

### HTTP API

| Endpoint | Method | Purpose |
|---|---|---|
| `/` , `/dashboard` | GET | Served local dashboard |
| `/health` | GET | JSON bot health snapshot |
| `/metrics` | GET | Prometheus-style metrics |
| `/positions` | GET | Open positions |
| `/signals?limit=N` | GET | Recent signals |
| `/executions?limit=N` | GET | Recent trades/executions |
| `/transactions?limit=N` | GET | V2 relayer transactions |
| `/daily?days=N` | GET | Daily stats |
| `/ws` | GET | Dashboard event stream |
| `/control/pause` | POST | Pause new trading |
| `/control/resume` | POST | Resume trading |
| `/control/emergency-stop` | POST | Pause and flatten local open positions |

Control endpoints require the `X-PolyBot-Control-Key` header, matching `POLYBOT_DASHBOARD_CONTROL_KEY`. Example pause (PowerShell):

```powershell
$headers = @{ "X-PolyBot-Control-Key" = $env:POLYBOT_DASHBOARD_CONTROL_KEY }
Invoke-WebRequest -Method POST -Headers $headers http://127.0.0.1:8080/control/pause
```

### Telegram controls

When a bot token and allowed user IDs are configured, operator commands include wallet management, confirmation-guarded actions, and collateral plan previews via `/wrap <amount>` and `/redeem <condition_id> <index_sets>` (these preview gasless calldata plans before any signed relayer submission).

### Safety / live-gating model

The default mode is simulation. Live mode requires all of the following to pass startup preflight:

- `POLYBOT_EXECUTION_MODE=live` and `POLYBOT_ENABLE_LIVE_V2=true`
- complete `RELAYER_*` config and a valid `BUILDER_CODE`
- valid pUSD / on-ramp / off-ramp / USDC.e collateral addresses
- `POLYBOT_DASHBOARD_CONTROL_KEY` and `POLYBOT_V2_VERIFY_CONDITION_ID`
- V2 market/fee endpoint verification, wallet authentication and allowance checks
- dry-run validation of wrap, unwrap, approval, and redeem calldata

If any are missing or malformed, startup preflight fails before trading. Always run a tiny-capital live smoke test before trusting meaningful funds.

## Testing

Run the full local quality gate:

```bash
cargo fmt --check
cargo check --workspace
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

## Project structure

```text
.
|-- polybot-common/          Shared domain types, constants, errors
|-- polybot-core/            Main bot runtime (binary)
|   |-- src/scanner/         Signal ingestion and normalization
|   |-- src/risk/            Sizing, limits, drawdown, copied-lot exits
|   |-- src/execution/       CLOB client, V2 signing, relayer, collateral plans
|   |-- src/state/           SQLite, PnL, positions, pUSD, reconciliation
|   |-- src/telegram_bot/    Telegram commands, auth, confirmations, alerts
|   `-- src/health.rs        Dashboard/API server
|-- polybot-dashboard/       Leptos dashboard app (development/alternate UI)
|-- docs/                    Windows runbook and migration notes
|-- config.toml              Default local config
|-- .env.example             Environment variable reference
`-- Dockerfile, docker-compose.yml
```

## Status

Honest maturity by area:

| Area | Status | Notes |
|---|---|---|
| Paper trading | Ready | The full loop — signals, risk, V2-shaped relayer lifecycle, pUSD accounting, dashboard, persistence — is usable and observable. |
| Dashboard / operator surface | Ready | Suitable for local operation; not premium SaaS polish. |
| Live trading code | Gated | The code path is gated and serious, but still needs live-credential smoke testing before real capital. |
| Real-money production | Needs smoke test | Run a tiny-capital live order before trusting meaningful funds. |

This software can interact with financial markets and may submit real orders when configured for live mode. Trading involves risk, including loss of funds. You are responsible for wallet security, API keys, configuration, market risk, and all orders submitted by your instance.

## License

No license specified.
