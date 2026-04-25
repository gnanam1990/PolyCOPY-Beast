# V3.2 Simulation-Complete Design

**Date:** 2026-04-25
**Scope:** Finish SuperFast PolyBot v3.2 to a simulation-complete CLOB V2 milestone before enabling live relayer submission.

## Goal

Bring the project from a strong V3/V3.2 foundation to a CLOB V2 simulation-complete system. The bot should exercise the production-shaped V2 data model, order-routing decisions, transaction lifecycle, pUSD accounting, dashboard state, and Telegram/operator controls without submitting live orders.

This milestone deliberately stops before real-money live submission. Live mode should remain blocked until the simulation-complete path is green, externally verified against V2 API response shapes, and protected by explicit setup checks.

## Current State

The repository already has a working Rust workspace with:

- shared domain types in `polybot-common`
- scanner, risk, execution, state, Telegram, and health/dashboard modules in `polybot-core`
- Leptos dashboard in `polybot-dashboard`
- SQLite tables for signals, trades, positions, daily stats, targets, copied lots, and V2 transactions
- V3.2 config scaffolding for relayer, collateral, builder code, fee caps, and category caps
- Data API polling, market WebSocket fast path, deduplication, risk limits, mirrored exits, reconciliation, and dashboard endpoints

Current verification status:

- `cargo check --workspace` passes
- `cargo clippy --workspace -- -D warnings` passes
- `cargo test --workspace` is not green because recent V2 struct additions left test fixtures behind

## Recommended Approach

Use a staged "simulation-complete first" approach.

### Why this approach

- It keeps the existing pipeline intact: scanner -> dedup -> risk -> execution -> state -> operator surfaces.
- It lets us build the V2 execution state machine once and run it in simulation before live submission.
- It avoids touching real funds while pUSD, fee schedules, relayer states, and auto-redeem behavior are still being validated.
- It gives a clean readiness gate: live mode can only be enabled after tests, setup checks, and simulated V2 flows pass.

### Alternatives rejected

- **Direct live V2 path:** Faster to place a test order, but too risky because relayer polling, pUSD accounting, fees, and cancellation/redeem behavior are still incomplete.
- **Full PRD perfect pass before any milestone:** Cleaner on paper, but too large for one safe implementation cycle and harder to debug.

## Milestone Boundaries

### In scope

- Restore green workspace tests.
- Add a minimal CLOB V2 execution core with order payload types, builder-code validation, timestamp handling, and EIP-712 domain version `"2"` structure.
- Add a relayer client abstraction with submit/poll response models and a simulation transport.
- Use the SQLite `transactions` table as the source of truth for async transaction lifecycle.
- Add virtual pUSD balance tracking for simulation.
- Add fee-aware routing decisions using market fee schedule data when available.
- Default to maker-first GTC planning; allow FOK only below configured taker-fee threshold.
- Track fee/rebate fields in trade and daily stats paths.
- Surface V2 state in dashboard/health data: pUSD balance, relayer queue, fees, rebates, and chosen order type.
- Add Telegram/operator readouts for V2 status; destructive wrap/redeem/live controls stay confirm-gated.
- Keep live submission disabled behind explicit setup checks.

### Out of scope for this milestone

- Real pUSD wrap/unwrap transactions.
- Real live relayer order submission on production funds.
- Fully automated live deployment.
- Nice-to-have strategy features such as stop-loss, take-profit, order aggregation, and wallet scoring.
- Replacing every legacy naming artifact if it does not affect simulation-complete correctness.

## Architecture

### V2 execution layer

Add a small V2 execution layer under `polybot-core/src/execution` rather than replacing the full runtime.

Recommended files:

- `v2_order.rs`: CLOB V2 order payload, builder code parsing, timestamp and bytes32 helpers.
- `v2_signing.rs`: EIP-712 domain version `"2"` and signing adapter boundaries.
- `v2_relayer.rs`: relayer submit/poll request and response models, state mapping, and transport trait.
- `v2_sim.rs`: deterministic simulated relayer transport for tests and simulation mode.

The existing execution engine should call this layer through a narrow facade. Simulation should run the same transaction-state logic as live, but with a fake relayer transport and no network submit.

### Transaction lifecycle

Every V2 order attempt should have a transaction lifecycle:

1. Build order plan from risk decision and market context.
2. Choose GTC or FOK based on fee policy.
3. Create a transaction record in SQLite when the simulated or real relayer accepts the request.
4. Poll or advance state through `STATE_NEW`, `STATE_PENDING`, `STATE_SUBMITTED`, then terminal `STATE_SUCCESS` or `STATE_FAILED`.
5. Only update positions and copied lots from terminal successful fills.
6. Persist failures with error message and alert metadata.

In simulation mode, the lifecycle should be deterministic and configurable enough to test success, failure, and delayed/pending cases.

### pUSD accounting

Simulation should introduce virtual pUSD accounting:

- starting virtual pUSD balance comes from config or the existing portfolio reference balance
- order plans reserve pUSD before simulated submit
- failed or cancelled transactions release reserved pUSD
- successful fills convert reserved pUSD into position exposure
- daily stats report pUSD-denominated volume, fees, rebates, and PnL

Real on-chain wrap/approve/redeem remains disabled until the next live-readiness milestone.

### Fee-aware routing

The risk/execution boundary should carry enough market metadata to choose order type:

- If fee schedule is missing, choose conservative GTC in simulation.
- If taker fee true bps is `0` or below `FOK_MAX_FEE_BPS`, FOK may be used for high-confidence immediate-copy cases.
- If taker fee exceeds threshold, route as GTC maker.
- Persist chosen order type, taker fee, fee paid, and rebate estimate.

The existing `FeeSchedule` shape must be verified against live/test V2 market responses before live readiness. Until verified, keep parsing isolated so the shape can change without touching the rest of the pipeline.

### Dashboard and Telegram

Dashboard should show enough V2 state for operator confidence:

- mode and live-disabled reason
- virtual pUSD balance and reserved pUSD
- relayer queue with transaction IDs and states
- fee paid and rebate totals
- signal feed with chosen order type and fee schedule
- execution log with relayer latency/state

Telegram should keep the existing whitelist/confirmation model and add V2 status language. Wrap, redeem, and live mode commands may be present as disabled or confirm-gated stubs until real relayer actions exist.

## Safety Gates

Live submission must remain blocked unless all of these are true:

- `cargo test --workspace` passes
- `cargo clippy --workspace -- -D warnings` passes
- setup check validates builder code, relayer config, collateral config, and endpoint mode
- V2 fee schedule shape has been verified against an actual endpoint response
- operator explicitly sets live execution mode
- live submission uses the V2 relayer path, not V1 credential flow

The dashboard control routes should be secured before live mode is allowed, because pause/resume/emergency-stop currently expose powerful actions.

## Testing Strategy

Use test-driven slices for each risky subsystem.

Required test groups:

- **Fixture repair:** update old tests for V2 `Signal` and `Trade` fields or add serde defaults where backward compatibility is intended.
- **V2 order/signing:** builder code validation, timestamp generation, EIP-712 domain version, stable payload serialization.
- **Relayer simulation:** state mapping, submit response persistence, terminal success/failure, delayed non-terminal states.
- **SQLite lifecycle:** insert/update/list transaction rows, trade linkage, fee/rebate persistence.
- **pUSD accounting:** reserve, release, fill, insufficient balance, daily stats.
- **Fee routing:** GTC default, FOK under threshold, missing fee schedule fallback.
- **Runtime integration:** scanner -> risk -> execution -> state in simulation creates the expected transaction/trade/position records.
- **Operator surfaces:** health/dashboard JSON includes V2 fields without breaking old consumers.

Verification commands for milestone completion:

```powershell
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo check --workspace
```

Dashboard build verification should be added once UI changes land:

```powershell
cd polybot-dashboard
trunk build --release
```

## Implementation Slices

1. Restore green tests and compatibility decisions.
2. Add V2 order/signing types behind tests.
3. Add relayer transport abstraction plus simulated relayer.
4. Persist transaction lifecycle in the runtime path.
5. Add virtual pUSD accounting.
6. Add fee-aware GTC/FOK routing.
7. Update dashboard and health responses for V2 simulation state.
8. Update Telegram/operator text and disabled stubs for live-only V2 actions.
9. Add setup-check gates and live-mode block reasons.
10. Run full verification and document remaining live-readiness work.

## Acceptance Criteria

The simulation-complete milestone is done when:

- full workspace tests and clippy pass
- simulation mode can process a representative V2 copy-trade signal through transaction lifecycle to a persisted trade and position
- simulated relayer success, failure, and pending states are covered by tests
- virtual pUSD balance, reserved pUSD, fees, rebates, and transaction states are visible through operator surfaces
- live submission remains blocked with a clear reason
- remaining live-readiness work is isolated to real relayer submit, real pUSD wrap/approve/redeem, endpoint verification, and production safety hardening

## Open Follow-Up For Live Readiness

After this milestone, the next spec should cover live-readiness only:

- verify actual CLOB V2 and relayer response shapes
- implement real pUSD wrap/approve/redeem via relayer
- enable real relayer order submission
- add cancellation support
- secure dashboard control endpoints
- perform low-capital monitored live rollout
