# SuperFast PolyBot v3 Windows Runbook

## 1. Prerequisites

- Windows PowerShell
- Rust toolchain installed (`rustup`, `cargo`)
- A `config.toml` file in the repo root

## 2. Minimal `.env` for simulation boot

Create a `.env` file in the repo root with:

```env
POLYBOT_SIMULATION=true
POLYBOT_EXECUTION_MODE=simulation
POLYBOT_LOG_LEVEL=info
POLYBOT_SQLITE_PATH=./polybot.db
POLYBOT_REDIS_ENABLED=false
POLYBOT_REDIS_URL=redis://127.0.0.1:6379
```

## 3. Live/shadow setup variables

Add these when you want live or shadow mode:

```env
POLYBOT_PRIVATE_KEY=0xYOUR_PRIVATE_KEY
POLYBOT_SIGNATURE_TYPE=0
POLYBOT_CLOB_ENDPOINT=https://clob.polymarket.com
POLYBOT_WS_ENDPOINT=wss://ws-subscriptions-clob.polymarket.com
POLYBOT_CLOB_CREDENTIALS_PATH=.\.polybot\clob_credentials.json
```

Notes:

- `POLYBOT_SIGNATURE_TYPE=0` = EOA
- `POLYBOT_SIGNATURE_TYPE=1` = Proxy
- `POLYBOT_SIGNATURE_TYPE=2` = GnosisSafe (requires `POLYBOT_FUNDER_ADDRESS`)
- Alias env names also work for compatibility: `POLYMARKET_PRIVATE_KEY`, `CLOB_API_URL`, `WS_CLOB_URL`, `FUNDER_ADDRESS`, `POLYGON_CHAIN_ID`

## 4. Preflight validation command

Run the startup checks without starting the full bot:

```powershell
cargo run -p polybot-core -- --setup-check
```

This validates:

- config parsing
- Polygon RPC connectivity
- wallet mode/signer setup
- CLOB API credential derivation/reuse
- first-run approval visibility for live/shadow mode

If approval checks fail, complete the Polymarket first-run USDC/CTF approval flow, then rerun the same command.

## 5. Start the bot

```powershell
cargo run -p polybot-core
```

## 6. Local health verification commands

After boot, verify the local operator surfaces:

```powershell
Invoke-WebRequest http://127.0.0.1:8080/health | Select-Object -ExpandProperty Content
Invoke-WebRequest http://127.0.0.1:8080/metrics | Select-Object -ExpandProperty Content
Invoke-WebRequest "http://127.0.0.1:8080/signals?limit=5" | Select-Object -ExpandProperty Content
Invoke-WebRequest http://127.0.0.1:8080/positions | Select-Object -ExpandProperty Content
```

## 7. Verification commands

```powershell
cargo test
cargo check
```

## 8. Expected credential persistence

On successful live/shadow authentication, derived CLOB credentials are stored at:

```text
.\.polybot\clob_credentials.json
```

That path is gitignored and can be overridden with `POLYBOT_CLOB_CREDENTIALS_PATH`.

Redis is optional in v3. Leave `POLYBOT_REDIS_ENABLED=false` for the default Windows-native SQLite-first path unless you intentionally want the extra Redis-backed integrations.
