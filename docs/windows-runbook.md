# SuperFast PolyBot v3 Windows Runbook

## 1. Prerequisites

- Windows PowerShell
- Rust stable toolchain (`rustup`, `cargo`)
- A repo-root `config.toml`
- A repo-root `.env` copied from `.env.example`

## 2. Minimal `.env` for simulation boot

Create a `.env` file in the repo root with:

```env
POLYBOT_EXECUTION_MODE=simulation
POLYBOT_LOG_LEVEL=info
POLYBOT_SQLITE_PATH=./polybot.db
POLYBOT_REDIS_ENABLED=false
POLYBOT_DASHBOARD_CONTROL_KEY=change-this-local-control-key
POLYBOT_PAPER_STARTING_BALANCE_USD=1000
POLYBOT_PAPER_FIXED_ENTRY_PRICE=0.50
```

This starts the safe paper path. No live order can be submitted from simulation mode.

## 3. Simulation setup check

Run startup checks without starting the full bot:

```powershell
cargo run -p polybot-core -- --setup-check
```

Expected simulation result:

```text
Startup preflight completed successfully: mode=Simulation simulation_preflight=true
```

## 4. Start the local bot

```powershell
cargo run -p polybot-core
```

Open the local operator dashboard:

```text
http://127.0.0.1:8080
```

## 5. Local health verification

After boot, verify the local operator surfaces:

```powershell
Invoke-WebRequest http://127.0.0.1:8080/health | Select-Object -ExpandProperty Content
Invoke-WebRequest http://127.0.0.1:8080/metrics | Select-Object -ExpandProperty Content
Invoke-WebRequest "http://127.0.0.1:8080/signals?limit=5" | Select-Object -ExpandProperty Content
Invoke-WebRequest http://127.0.0.1:8080/positions | Select-Object -ExpandProperty Content
Invoke-WebRequest "http://127.0.0.1:8080/transactions?limit=5" | Select-Object -ExpandProperty Content
```

Dashboard controls require `POLYBOT_DASHBOARD_CONTROL_KEY`.

## 6. Live-gated variables

Only add these when intentionally preparing a tiny live smoke test:

```env
POLYBOT_EXECUTION_MODE=live
POLYBOT_ENABLE_LIVE_V2=true
POLYMARKET_PRIVATE_KEY=0xYOUR_PRIVATE_KEY
POLYBOT_SIGNATURE_TYPE=0
POLYBOT_DASHBOARD_CONTROL_KEY=your-long-random-control-key
POLYBOT_V2_VERIFY_CONDITION_ID=0xACTIVE_MARKET_CONDITION_ID

RELAYER_URL=https://...
RELAYER_API_KEY=...
RELAYER_API_KEY_ADDRESS=0x...
BUILDER_CODE=0x...

POLYBOT_CLOB_ENDPOINT=https://clob.polymarket.com
POLYBOT_WS_ENDPOINT=wss://ws-subscriptions-clob.polymarket.com
```

Wallet modes:

| Value | Mode | Notes |
|---:|---|---|
| `0` | EOA | Simplest smoke-test mode. |
| `1` | Proxy | Requires or derives a proxy wallet; funder can be supplied. |
| `2` | Gnosis Safe | Requires safe/funder address. |

`POLYMARKET_PRIVATE_KEY` is the canonical name. Legacy `POLYBOT_PRIVATE_KEY` is still accepted with a deprecation warning.

## 7. Live preflight expectations

Before live startup, run:

```powershell
cargo run -p polybot-core -- --setup-check
```

Live preflight must validate:

- V2 live gate is explicitly enabled.
- Wallet key and signature mode are valid.
- Dashboard control key exists.
- Relayer URL, API key, API key address, and builder code are complete.
- Builder code is bytes32 hex.
- V2 market/fee endpoint shape can be verified with `POLYBOT_V2_VERIFY_CONDITION_ID`.
- pUSD, onramp, offramp, and USDC.e collateral addresses are configured.
- Wrap, unwrap, approval, and redeem transaction calldata plans are dry-run validated.

If setup-check fails, stop there. Do not start live trading by bypassing preflight.

## 8. Tiny live smoke test

Use tiny capital first. Suggested operator flow:

```powershell
cargo run -p polybot-core -- --setup-check
cargo run -p polybot-core
```

Then:

- Confirm `/health` reports live mode and healthy local surfaces.
- Open `http://127.0.0.1:8080` and verify the dashboard shows live-gated readiness.
- Send or ingest one tiny test signal only.
- Verify `/positions`, `/executions`, and `/transactions` after the order lifecycle.
- Test pause, resume, and emergency stop with the dashboard control key.
- If Telegram is configured, verify allowlisted command access and alerts.

Do not scale capital until the tiny order path is boring and repeatable.

## 9. Verification commands

Use the workspace quality gate before merging code changes:

```powershell
cargo fmt --check
cargo check --workspace
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

For focused paper-balance validation:

```powershell
cargo test -p polybot-core paper
cargo run -p polybot-core -- --setup-check
```

Redis is optional in v3. Leave `POLYBOT_REDIS_ENABLED=false` for the default Windows-native SQLite-first path unless you intentionally want the extra Redis-backed integrations.
