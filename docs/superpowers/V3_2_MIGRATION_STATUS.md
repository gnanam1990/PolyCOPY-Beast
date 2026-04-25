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

### PRD vs real V2 API discrepancies — VERIFIED 2026-04-25

After probing live `clob-v2.polymarket.com/markets` and reading `Polymarket/py-clob-client-v2` source:

**The PRD is substantially fictional regarding V2 internals.** Decision (2026-04-25): **land Phase 1-3 as-is, reconcile in Phase 4.** The fictional infrastructure stays in the schema/types as dead weight; Phase 4 ignores it and ports from the real Python SDK.

#### What the PRD got wrong

| PRD claim | Reality | Phase 1-3 artifact now dead |
|---|---|---|
| `feeSchedule = {takerFee, makerFee, rebate}` flat values | Polynomial: `platform_fee = amount * rate * (price * (1-price))^exponent / price` + separate `builder_fee_rate`. API exposes `maker_base_fee`/`taker_base_fee` integers. | `polybot_common::types::FeeSchedule` shape is wrong (struct exists, not yet consumed) |
| Async relayer flow with `transactionID` polling and `STATE_*` lifecycle | **No relayer indirection.** V2 posts directly to `/order` synchronously. | `transactions` table (migration v2), `TransactionState`, `TransactionKind`, `TransactionRecord`, `Trade.{transaction_id, transaction_hash, relayer_state, retry_count}`, all CRUD helpers, all 3 idempotency tests — all dead |
| `BUILDER_CODE` replaces API keys | API keys still required (`/auth/api-key`, HMAC). Builder code is an additional order field for fee attribution. | `RelayerConfig` (Phase 2) is fictional — no separate relayer URL exists |
| `metadata: String`, `builder: String` (Rust types) | Both `bytes32` in EIP-712, hex strings in JSON wire | Field types in any future order builder must be bytes32-shaped |
| Order types: GTC, FOK | Real: GTC, FOK, GTD, FAK. `postOnly` is a separate boolean flag. | `OrderType::PostOnly` should be a flag; missing `Gtd` |
| pUSD wrapping via Collateral Onramp | Not visible in V2 SDK; SDK still references USDC | `CollateralConfig.onramp_address` likely unused |

#### What the PRD got right (verified)

- ✅ EIP-712 domain version `"2"`
- ✅ Removed from order struct: `taker`, `nonce`, `feeRateBps`
- ✅ Added to order struct: `timestamp` (ms), `metadata`, `builder`
- ✅ Standard Exchange contract: `0xE111180000d2663C0091e4f400237545B87B996B`
- ✅ Neg Risk Exchange contract: `0xe2222d279d744050d28e00520010520000310F59`
- ✅ V2 cutover date: 2026-04-28

#### Verified V2 wire format (from `py-clob-client-v2/order_builder/builder.py::order_to_json_v2`)

```json
{
  "order": {
    "salt": <int>,
    "maker": "0x...",
    "signer": "0x...",
    "tokenId": "<string>",
    "makerAmount": "<string>",
    "takerAmount": "<string>",
    "side": "BUY" | "SELL",
    "expiration": "<string>",
    "signatureType": <int>,
    "timestamp": "<string ms>",
    "metadata": "<bytes32 hex>",
    "builder": "<bytes32 hex>",
    "signature": "0x..."
  },
  "owner": "<api-key>",
  "orderType": "GTC" | "FOK" | "GTD" | "FAK",
  "deferExec": false,
  "postOnly": false
}
```

#### Verified V2 fee model (from `py-clob-client-v2/tests/test_fee_calculations.py`)

```
platform_fee_usd = amount_usd * fee_rate * (price * (1 - price))^fee_exponent / price
builder_fee_usd  = amount_usd * builder_taker_fee_rate
total_fee_usd    = platform_fee_usd + builder_fee_usd
```

Per-market data needed:
- `base_fee` (int, returned by `GET /fee-rate?token_id=…`)
- `fee_exponent` (float, from same endpoint or cached `FeeInfo`)
- `taker_only` (bool)
- Builder fee rate (separate cache, fetched per-builder-code)

Higher near 50¢, lower at extremes — designed for prediction markets.

#### Phase 4 mandate (revised)

**Port from `Polymarket/py-clob-client-v2`** rather than implement from PRD:

1. Mirror `OrderV2` / `SignedOrderV2` / `order_to_json_v2` exactly (bytes32 shape preserved)
2. Implement polynomial fee calculator in Rust (`base_fee` + `exponent` + `taker_only`)
3. Keep API key auth (HMAC L2) — V2 didn't kill it
4. Add `builder_code` to every signed order
5. POST `/order` synchronously — no relayer state machine, no `transactions` table writes
6. **Ignore** the dead infrastructure from Phase 1-2 (`transactions` table, `TransactionState`, `RelayerConfig`, `FeeSchedule`); Phase 5+ may rip it out as cleanup

Estimated scope: **smaller** than originally projected. ~400-500 LOC + tests (no relayer client, no transaction poller, no pUSD wrap).

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

### Dead infrastructure to clean up (Phase 5+ when convenient)

Per the 2026-04-25 PRD-vs-reality audit, the following Phase 1-2 additions are dead and should eventually be removed (kept for now per "land as-is, reconcile in Phase 4" decision):

**polybot-common/src/types.rs:**
- `FeeSchedule` struct (wrong shape)
- `TransactionState` enum (no async lifecycle exists)
- `TransactionKind` enum
- `TransactionRecord` struct
- `Trade.transaction_id`, `Trade.transaction_hash`, `Trade.relayer_state`, `Trade.retry_count` fields
- `Signal.fee_schedule` field (or rewrite payload to real shape)

**polybot-core/src/state/sqlite.rs:**
- `transactions` table (migration v2)
- `idx_transactions_state`, `idx_transactions_trade_id` indexes
- `insert_transaction`, `update_transaction_state`, `get_transaction`, `list_non_terminal_transactions` methods
- `row_to_transaction_record` helper
- 3 transactions CRUD tests
- `trades.transaction_id`, `trades.transaction_hash`, `trades.relayer_state`, `trades.retry_count` columns

**polybot-core/src/config.rs:**
- `RelayerConfig` struct + `Option<RelayerConfig>` on `AppConfig`
- All `RELAYER_*` env parsing
- 3 relayer-related tests
- `CollateralConfig.onramp_address` field (likely; verify before deleting)
- `COLLATERAL_ONRAMP_ADDRESS` env parsing

**Approach:** rather than a destructive cleanup migration (which loses any V1 historical writes to those columns), prefer leaving columns in place and just stop writing to them. Schema cleanup can be a far-future cosmetic pass.

## Out-of-scope (per PRD §17)

Every plan's "Out-of-scope reminders" section lists what was intentionally deferred. When writing subsequent plans, check that file first to avoid re-litigating.
