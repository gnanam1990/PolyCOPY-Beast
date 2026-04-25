# CLOB V2 Migration Status

**Last updated:** 2026-04-25
**Branch:** `main`
**Merged PR range:** #7 through #14

## PRD

`SuperFast_PolyBot_v3_2_CLOB_V2_PRD.md` is the v3.2 CLOB V2 spec.

## Current Milestone

The active milestone is `paper-ready on main`.

The project now has a complete local paper loop:

- SQLite schema and domain types for V2 transactions, fees, copied lots, positions, and daily stats.
- V2-shaped order payload/signing helpers, relayer client, simulated relayer lifecycle, and transaction storage.
- Explicit paper capital through `[paper].starting_balance_usd` and `POLYBOT_PAPER_STARTING_BALANCE_USD`.
- Deterministic paper fills through `[paper].fixed_entry_price` and `POLYBOT_PAPER_FIXED_ENTRY_PRICE`.
- pUSD available/reserved accounting, realized/unrealized PnL, and dashboard portfolio value.
- Served local command dashboard with health, positions, signals, executions, V2 transactions, and authenticated controls.
- Live mode blocked behind explicit V2 gate, relayer/builder config, collateral config, endpoint verification, wallet setup, and setup-check.

## Completed Work

| Area | Status | Notes |
|---|---|---|
| Phase 1 schema/types | Complete | Migrations, V2 transaction fields, fee columns, and common domain types are merged. |
| Phase 2 config cleanup | Complete | Relayer, builder, collateral, category caps, and env parsing are merged. |
| Phase 3 V1 cleanup | Complete | Old RPC/gas assumptions were removed from the active startup path. |
| Minimal V2 layer | Complete for paper and gated live | The project uses an in-repo Rust implementation instead of waiting for a Rust SDK. |
| Relayer simulation | Complete | Paper mode exercises V2 transaction states without submitting live orders. |
| Paper balances | Complete | Starting balance is explicit and defaults to `$1,000`. |
| Dashboard | Complete for local operation | The served dashboard is the primary operator surface on port `8080`. |
| Branch cleanup | Complete | Historical merged branches were removed; `main` is the source of truth. |

## Verified Checks

Most recent merged verification:

- `cargo test --workspace`: pass
- `cargo clippy --workspace -- -D warnings`: pass
- `cargo check --workspace`: pass
- `trunk build --release` from `polybot-dashboard`: pass
- `cargo run -p polybot-core -- --setup-check`: pass with `mode=Simulation simulation_preflight=true`

Focused verification for the paper-balance PR:

- `cargo test -p polybot-core paper`: pass
- `cargo run -p polybot-core -- --setup-check`: pass in simulation mode

## What Is Still Pending

These are operator-gated items, not normal code backlog:

- Add real live secrets: `POLYMARKET_PRIVATE_KEY`, complete `RELAYER_*`, valid `BUILDER_CODE`, and dashboard control key.
- Set `POLYBOT_V2_VERIFY_CONDITION_ID` to an active market condition ID before live preflight.
- Fund the wallet with tiny smoke-test capital.
- Run `cargo run -p polybot-core -- --setup-check` in live mode and do not continue unless it passes.
- Submit one tiny live order and verify `/positions`, `/executions`, `/transactions`, dashboard controls, and relayer state.
- Configure a real Telegram bot token and allowlisted user IDs, then verify commands and alerts.

## SDK Decision

There is no official Polymarket Rust CLOB V2 SDK in the repo. The migration proceeded with a minimal in-repo Rust V2 layer using the Rust Ethereum ecosystem and `reqwest` rather than depending on an unverified community crate.

Keep `polymarket-client-sdk 0.4` only for compatible V1/Gamma/Data API paths that remain useful.

## Product Readiness

| Slice | Readiness | Comment |
|---|---:|---|
| Paper trading | 90% | Usable for local simulation and operator rehearsal. |
| Dashboard | 85% | Good enough for local command-center operation; visual polish can keep improving. |
| Live trading | 70% | Serious gated path exists, but real-money trust requires the tiny live smoke test. |
| Telegram | 75% | Code path exists; final readiness requires a real token and allowlisted user verification. |

Honest next step: run the tiny live smoke test with expendable capital only after secrets and funding are configured.
