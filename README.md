# SuperFast PolyBot

> Windows-first Polymarket copy-trading command center, built in Rust.
>
> Paper-ready today. Live-gated by design.

SuperFast PolyBot is a self-hosted copy-trading system for Polymarket's CLOB. It watches target wallets, turns observed trades into risk-scored copy signals, simulates or submits CLOB V2-shaped orders, persists the full lifecycle in SQLite, and gives the operator a local dashboard plus Telegram controls.

This project is intentionally practical: no Redis requirement, no Docker requirement, no hosted backend, and no hidden SaaS dependency. It is designed for one serious operator running a local Windows machine.

## Current Status

| Area | Status | Notes |
|---|---:|---|
| Paper trading | Ready | Simulation path exercises signals, risk, V2-shaped relayer lifecycle, pUSD accounting, dashboard, and persistence. |
| Dashboard | Ready | Served local dashboard includes health, positions, executions, V2 readiness, pUSD, relayer transactions, and authenticated controls. |
| Live trading code | Live-gated ready | Live mode requires explicit V2 gate, relayer config, builder code, dashboard control key, collateral plan validation, endpoint verification, and approvals. |
| Real-money production | Needs smoke test | Do a tiny-capital live order before trusting meaningful funds. |

## What It Does

- Tracks target wallet activity through Polymarket/Data API ingestion paths.
- Deduplicates signals by transaction/hash and rejects stale or unsafe signals.
- Scores each copy attempt through confidence, secret level, drawdown, exposure, liquidity, category caps, and minimum-balance rules.
- Builds fee-aware CLOB V2-style order plans using maker-first GTC behavior and FOK only under configured fee limits.
- Simulates relayer transaction states and stores V2 transaction records in SQLite.
- Tracks virtual pUSD, reserved pUSD, fees, rebates, positions, daily stats, and recent signals.
- Serves a local operator dashboard on port `8080`.
- Exposes pause, resume, and emergency stop controls guarded by `POLYBOT_DASHBOARD_CONTROL_KEY`.
- Supports Telegram operator commands with allowlisted users, confirmation safety, wallet management, and collateral plan previews for `/wrap` and `/redeem`.

## Safety Model

The default mode is simulation. Live trading cannot happen accidentally.

Live mode requires all of these to pass:

- `POLYBOT_EXECUTION_MODE=live`
- `POLYBOT_ENABLE_LIVE_V2=true`
- complete `RELAYER_*` config
- valid `BUILDER_CODE`
- valid pUSD/onramp/offramp/USDC.e collateral addresses
- `POLYBOT_DASHBOARD_CONTROL_KEY`
- `POLYBOT_V2_VERIFY_CONDITION_ID`
- V2 market/fee endpoint verification
- wallet authentication and allowance checks
- dry-run validation of wrap, unwrap, approval, and redeem transaction calldata

If any of those are missing or malformed, startup preflight fails before trading.

## Architecture

```text
Target Wallet Activity
        |
        v
Scanner + Deduplication
        |
        v
Risk Engine
  - confidence and secret multipliers
  - drawdown protection
  - exposure caps
  - liquidity cap
  - stale/resolved market guards
        |
        v
Execution Engine
  - simulation relayer
  - V2 order payload/signing
  - relayer submit/poll client
  - pUSD accounting
        |
        v
SQLite System of Record
  - signals
  - trades
  - positions
  - copied lots
  - daily stats
  - V2 transactions
        |
        v
Operator Surfaces
  - local dashboard
  - health and metrics API
  - Telegram controls
```

## Repository Layout

```text
.
|-- polybot-common/          Shared domain types, constants, and errors
|-- polybot-core/            Main bot runtime, APIs, dashboard, execution, state
|   |-- src/scanner/         Signal ingestion and normalization
|   |-- src/risk/            Sizing, limits, drawdown, copied-lot exits
|   |-- src/execution/       CLOB client, V2 signing, relayer, collateral plans
|   |-- src/state/           SQLite, PnL, positions, pUSD, reconciliation
|   |-- src/telegram_bot/    Telegram commands, auth, confirmations, alerts
|   `-- src/health.rs        Dashboard/API server
|-- polybot-dashboard/       Leptos dashboard app for development/alternate UI
|-- docs/                    Windows runbook and migration notes
|-- config.toml              Default local config
|-- .env.example             Environment variable reference
`-- SuperFast_PolyBot_v3_2_CLOB_V2_PRD.md
```

## Quick Start: Paper Trading

### 1. Install Requirements

- Rust stable toolchain
- PowerShell
- Optional: Trunk, if you want to build the Leptos dashboard app

### 2. Configure `.env`

Copy the example file:

```powershell
Copy-Item .env.example .env
```

For paper mode, this is enough to start safely:

```env
POLYBOT_EXECUTION_MODE=simulation
POLYBOT_SQLITE_PATH=./polybot.db
POLYBOT_LOG_LEVEL=info
POLYBOT_DASHBOARD_CONTROL_KEY=change-this-local-control-key
POLYBOT_PAPER_STARTING_BALANCE_USD=1000
POLYBOT_PAPER_FIXED_ENTRY_PRICE=0.50
```

Target wallets and API keys can be added when you are ready to ingest real activity.

### 3. Run Setup Check

```powershell
cargo run -p polybot-core -- --setup-check
```

Expected paper-mode result:

```text
Startup preflight completed successfully: mode=Simulation simulation_preflight=true
```

### 4. Start The Bot

```powershell
cargo run -p polybot-core
```

Open:

```text
http://127.0.0.1:8080
```

The dashboard served by `polybot-core` is the main local operator surface.

## Paper Balance And Entry Price

Paper capital is now explicit. It is not derived from risk sizing.

Default paper config:

```toml
[paper]
starting_balance_usd = 1000
fixed_entry_price = 0.50
```

Environment overrides:

```env
POLYBOT_PAPER_STARTING_BALANCE_USD=1000
POLYBOT_PAPER_FIXED_ENTRY_PRICE=0.50
```

The dashboard portfolio value starts from the configured paper balance plus realized and unrealized PnL. Virtual pUSD is available cash after open exposure is reserved. Paper fills use the fixed entry price so local simulations are deterministic and do not pretend to have live orderbook liquidity.

## Dashboard

The local dashboard shows:

- mode: simulation or live
- portfolio value
- daily PnL
- drawdown
- virtual pUSD and reserved pUSD
- fees paid and rebates earned
- live-disabled/live-gate reason
- system health and WebSocket state
- recent signals
- open positions
- recent executions
- V2 relayer transactions
- pause, resume, and emergency stop controls

Control actions require:

```env
POLYBOT_DASHBOARD_CONTROL_KEY=your-local-control-key
```

The dashboard asks for this key the first time you use a control action and stores it in browser local storage.

## Live-Gated Setup

Do not use meaningful funds until paper behavior is boring and a tiny live smoke test passes.

Minimum live-gated environment:

```env
POLYBOT_EXECUTION_MODE=live
POLYBOT_ENABLE_LIVE_V2=true
POLYMARKET_PRIVATE_KEY=0x...
POLYBOT_SIGNATURE_TYPE=0
POLYBOT_DASHBOARD_CONTROL_KEY=...
POLYBOT_V2_VERIFY_CONDITION_ID=0x...

RELAYER_URL=...
RELAYER_API_KEY=...
RELAYER_API_KEY_ADDRESS=...
BUILDER_CODE=0x...
POLYBOT_COLLATERAL_RECIPIENT_ADDRESS=0x...

POLYBOT_CLOB_ENDPOINT=https://clob.polymarket.com
POLYBOT_WS_ENDPOINT=wss://ws-subscriptions-clob.polymarket.com
```

Wallet mode:

| Value | Mode | Notes |
|---:|---|---|
| `0` | EOA | Simplest mode. |
| `1` | Proxy | Requires or derives proxy wallet; funder can be supplied. |
| `2` | Gnosis Safe | Requires safe/funder address. |

Before starting live:

```powershell
cargo run -p polybot-core -- --setup-check
```

Only after setup-check passes should you run a tiny-capital smoke test.

## Configuration Highlights

Primary local config lives in `config.toml`.

Important paper/risk values:

```toml
[system]
execution_mode = "simulation"

[paper]
starting_balance_usd = 1000
fixed_entry_price = 0.50

[risk]
base_size_usd = 50
base_size_pct = 0.015
daily_max_loss_pct = 0.05
max_position_size_usd = 500
max_concurrent_positions = 20
min_confidence = 6
min_secret_level = 5
position_multiplier = 1.0
min_trade_size_usdc = 1.0
min_usdc_balance = 20
```

Important V2 values:

```toml
[collateral]
token = "pUSD"
pusd_address = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB"
onramp_address = "0x93070a847efEf7F70739046A929D47a521F5B8ee"
offramp_address = "0x2957922Eb93258b93368531d39fAcCA3B4dC5854"
usdc_e_address = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174"
```

Environment variables override config where supported. See `.env.example` for the full list.

## HTTP API

| Endpoint | Method | Purpose |
|---|---|---|
| `/` | GET | Served local dashboard |
| `/dashboard` | GET | Served local dashboard |
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

Control endpoints require the `X-PolyBot-Control-Key` header.

## Verification

Run the full local quality gate:

```powershell
cargo fmt --check
cargo check --workspace
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

Build the Leptos dashboard app:

```powershell
Set-Location polybot-dashboard
trunk build --release
```

Run startup preflight:

```powershell
cargo run -p polybot-core -- --setup-check
```

## Operator Runbook

What is still operator-only before real live trading:

- Add real `POLYMARKET_PRIVATE_KEY`, complete `RELAYER_*`, and valid `BUILDER_CODE`.
- Set `POLYBOT_ENABLE_LIVE_V2=true` and `POLYBOT_EXECUTION_MODE=live` intentionally.
- Set `POLYBOT_V2_VERIFY_CONDITION_ID` to an active market condition ID.
- Fund the wallet with tiny smoke-test capital first, not meaningful capital.
- Run `cargo run -p polybot-core -- --setup-check` and do not continue unless it passes.
- Submit one tiny order, verify `/positions`, `/transactions`, dashboard controls, and Telegram alerts.
- Use Telegram `/wrap <amount>` and `/redeem <condition_id> <index_sets>` to preview gasless collateral calldata plans before any signed relayer submission.

Useful local checks:

```powershell
Invoke-WebRequest http://127.0.0.1:8080/health | Select-Object -ExpandProperty Content
Invoke-WebRequest http://127.0.0.1:8080/metrics | Select-Object -ExpandProperty Content
Invoke-WebRequest "http://127.0.0.1:8080/signals?limit=5" | Select-Object -ExpandProperty Content
Invoke-WebRequest http://127.0.0.1:8080/positions | Select-Object -ExpandProperty Content
Invoke-WebRequest "http://127.0.0.1:8080/transactions?limit=5" | Select-Object -ExpandProperty Content
```

Pause from PowerShell:

```powershell
$headers = @{ "X-PolyBot-Control-Key" = $env:POLYBOT_DASHBOARD_CONTROL_KEY }
Invoke-WebRequest -Method POST -Headers $headers http://127.0.0.1:8080/control/pause
```

Resume:

```powershell
Invoke-WebRequest -Method POST -Headers $headers http://127.0.0.1:8080/control/resume
```

Emergency stop:

```powershell
Invoke-WebRequest -Method POST -Headers $headers http://127.0.0.1:8080/control/emergency-stop
```

## Troubleshooting

| Symptom | Likely Fix |
|---|---|
| Dashboard control says key missing | Set `POLYBOT_DASHBOARD_CONTROL_KEY`, restart the bot, and re-enter it in the browser. |
| Setup-check says live V2 disabled | Set `POLYBOT_ENABLE_LIVE_V2=true` only when intentionally testing live. |
| Setup-check requires condition ID | Set `POLYBOT_V2_VERIFY_CONDITION_ID` to an active market condition ID. |
| Partial `RELAYER_*` config ignored | Provide all of `RELAYER_URL`, `RELAYER_API_KEY`, and `RELAYER_API_KEY_ADDRESS`. |
| Invalid builder code | `BUILDER_CODE` must be `0x` plus 64 hex chars. |
| No trades in paper mode | Add target wallets/signals and confirm confidence/secret/category thresholds are not blocking. |
| Port `8080` busy | Change `[dashboard].port` in `config.toml` or stop the process using the port. |
| Blank Leptos dev dashboard | Ensure `polybot-core` is running; `trunk serve` proxies API calls to port `8080`. |

## Product Readiness

Current honest score:

| Product Slice | Rating | Why |
|---|---:|---|
| Paper trading | 8/10 | The full loop is usable and observable. |
| Dashboard/operator surface | 7/10 | Ready for local operation, not yet premium SaaS polish. |
| Live trading | 6.5-7/10 | Code path is gated and serious, but still needs live credential smoke testing. |

The project is usable. It is not a toy anymore. Treat live capital with respect anyway.

## Disclaimer

This software can interact with financial markets and may submit real orders when configured for live mode. Trading involves risk, including loss of funds. Start in simulation, validate behavior, then use tiny live capital before scaling.

You are responsible for wallet security, API keys, configuration, market risk, and all orders submitted by your instance.
