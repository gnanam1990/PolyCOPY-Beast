# CLOB V2 Migration — Status

**Last updated:** 2026-04-24
**Branch:** `feat/v3.2-clob-v2-phase1` (spans Phases 1–3; name is historical)
**Commits above main:** 35

## PRD

`SuperFast_PolyBot_v3_2_CLOB_V2_PRD.md` (root of repo) — v3.2 spec, V2 go-live 2026-04-28.

## Phases

| Phase | Scope | Plan | Status |
|---|---|---|---|
| **1** | SQLite schema + polybot-common types foundation (migration runner, `transactions` table, fee columns, `FeeSchedule` / `TransactionState` / `TransactionRecord` types, Signal/Trade struct extensions) | `docs/superpowers/plans/2026-04-24-v3.2-clob-v2-phase1-schema-foundation.md` | ✅ Complete, reviewed |
| **2** | V2 config scaffolding (`RelayerConfig`, `CollateralConfig`, `BuilderConfig`, `fok_max_fee_bps`, category caps) + 3 Phase-1 review follow-ups (FK pragma, `insert_trade` V2 cols, `FeeSchedule::*_bps_true` rename) + `serial_test` for env tests | `docs/superpowers/plans/2026-04-24-v3.2-clob-v2-phase2-config-cleanup.md` | ✅ Complete, reviewed |
| **3** | V1 RPC/gas deletion (`rpc_pool.rs`, `rpc_endpoints` config, Polygon chain-id preflight) + `POLYBOT_PRIVATE_KEY` → `POLYMARKET_PRIVATE_KEY` rename with alias | `docs/superpowers/plans/2026-04-24-v3.2-clob-v2-phase3-v1-cleanup.md` | ✅ Complete, reviewed |
| **4** | V2 SDK integration + order builder rewrite — **BLOCKED pending SDK decision** | _not yet written_ | ⏸️ |
| 5–11 | Relayer client, risk engine rewrite, auto-redeem, Telegram additions, dashboard WS stream, e2e sim | _not yet written_ | ⏸️ |

## Current Milestone

The active milestone is `v3.2 simulation-complete`.

Scope:
- restore green workspace tests
- run simulation through a V2-shaped relayer transaction lifecycle
- add virtual pUSD accounting
- add fee-aware maker/FOK routing
- expose V2 state in dashboard and Telegram
- keep live submission blocked behind an explicit safety gate

## Final Verification for Simulation-Complete

Run on 2026-04-25:
- `cargo test --workspace`: PASS (35 `polybot-common`, 276 `polybot-core`, dashboard/doc-test targets clean)
- `cargo clippy --workspace -- -D warnings`: PASS
- `cargo check --workspace`: PASS
- `trunk build --release` from `polybot-dashboard`: PASS
- `cargo run -p polybot-core -- --setup-check`: PASS, `mode=Simulation simulation_preflight=true`

## Key quality signals at end of Phase 3

- `cargo test --workspace`: **231 passed** (33 polybot-common + 198 polybot-core + 0 dashboard), 0 failed
- `cargo clippy --workspace -- -D warnings`: clean
- `cargo run --release -p polybot-core -- --setup-check`: starts in simulation mode, `mode=Simulation simulation_preflight=true`
- Both `POLYMARKET_PRIVATE_KEY=…` and `POLYBOT_PRIVATE_KEY=…` (with deprecation warning) work
- Zero `rpc_pool` / `rpc_endpoints` / `POLYGON_RPC` hits in `polybot-core/src`

## Critical finding from Phase 4 SDK probe (2026-04-24)

**There is no official Polymarket Rust V2 SDK.** The migration docs at <https://docs.polymarket.com/v2-migration> list only TypeScript (`@polymarket/clob-client-v2`) and Python (`py-clob-client-v2`). A "unified" future SDK is mentioned with no Rust timeline.

The PRD's references to `rs-clob-client-v2` / `polymarket_client_v2` as Cargo dependencies were aspirational — those crates do not exist.

### Community options evaluated

| Crate / repo | Verdict |
|---|---|
| `polymarket-client-sdk 0.4.4` (current dep) | Official, V1 only |
| `Polymarket/rs-clob-client 0.3.1` | Official, V1 only |
| `polyfill2` | Community fork claims V2 support; page fetch blocked, unverified |
| `sproot/polymarket-sdk` | "1 Commit" — too immature |
| `imangoMah/polymarket-rs-sdk` | Claims parity with TS SDK incl. EIP-712 + builder relayer; unverified depth |

### PRD vs real V2 API discrepancies

**Must reconcile before Phase 4 consumes V2-specific types:**

1. **V2 Order struct** — official docs (verbatim):
   ```
   Order(uint256 salt, address maker, address signer, uint256 tokenId,
         uint256 makerAmount, uint256 takerAmount, uint8 side,
         uint8 signatureType, uint256 timestamp,
         bytes32 metadata, bytes32 builder)
   ```
   PRD showed `metadata: String`, `builder: String`, and `feeRateBps` as a retained deprecated field — docs say those types are `bytes32` and `feeRateBps` is **removed** from the struct entirely.

2. **Fee schedule shape** — official docs say `getClobMarketInfo().info.fd = { r: rate, e: exponent, to: takerOnly }`. Our Phase 1 `FeeSchedule` type uses PRD's `{takerFee, makerFee, rebate}` shape. **Phase 4 must verify against a live `GET /markets` response** and rewrite `polybot-common::types::FeeSchedule` if needed. Blast radius is small — no consumer wired yet.

### Verified V2 contract addresses (from docs)

- Standard Exchange: `0xE111180000d2663C0091e4f400237545B87B996B`
- Neg Risk Exchange: `0xe2222d279d744050d28e00520010520000310F59`
- EIP-712 Domain: `{name: "Polymarket CTF Exchange", version: "2", chainId: 137, verifyingContract: <one of above>}`

## Recommendation for Phase 4

**Roll our own minimal V2 layer** using `alloy` (Rust Ethereum ecosystem) + `reqwest`:

- EIP-712 v2 signing: `alloy-primitives` + `alloy-sol-types`, ~200 LOC
- Relayer HTTP client: reqwest wrapper for POST /order + GET /transaction, ~150 LOC
- Transaction poller: wraps `list_non_terminal_transactions` (added Phase 1), ~100 LOC
- Market info fetcher: ~100 LOC
- Real-shape `FeeSchedule` rewrite: ~50 LOC

**Estimated:** ~600–700 LOC + tests. Keep `polymarket-client-sdk 0.4` for V1 Gamma/Data calls that remain compatible.

**Why not `polyfill2`:** Unverified solo-maintainer crate for live-trading infrastructure.
**Why not wait:** No Polymarket timeline; V2 goes live 2026-04-28.

## Phase 4+ deferred follow-ups

From Phase 1/2/3 reviewer notes — absorb when scope naturally includes them:

- Prune orphaned `PolybotError::RpcPool` variant in `polybot-common/src/errors.rs`
- Rename `metrics.set_rpc_healthy()` — now semantically misleading post-V2 gasless
- Delete CLOB credential persistence (`clob_credentials.json` write/read) in `clob_client.rs` when V2 SDK replaces V1 auth flow
- Remove `#![allow(dead_code)]` from `main.rs` once bigger deletions settle
- Consider demoting partial-`RELAYER_*` warn to `debug!` or `Once`-guarded when reload semantics defined
- Evaluate folding minimal preflight into `ClobClient::from_env`

## Out-of-scope (per PRD §17)

Every plan's "Out-of-scope reminders" section lists what was intentionally deferred. When writing subsequent plans, check that file first to avoid re-litigating.
