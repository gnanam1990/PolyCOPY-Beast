# V3.2 Simulation-Complete Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the CLOB V2 simulation-complete milestone: green tests, V2 order/relayer simulation, virtual pUSD accounting, fee-aware routing, and V2 operator visibility while live submission remains blocked.

**Architecture:** Keep the existing scanner -> risk -> execution -> state -> dashboard/Telegram pipeline. Add small V2-focused execution modules and pure accounting helpers, then thread simulated transaction lifecycle through the existing state and operator surfaces. Use `OrderType::Limit` as the maker/GTC representation for this milestone.

**Tech Stack:** Rust, Tokio, Serde, Rusqlite/SQLite, Axum, Leptos dashboard, existing Polymarket SDK for non-V2-compatible read paths.

---

## Scope Check

This plan covers multiple slices, but they are not independent products. The transaction lifecycle is the central seam: V2 order shape, simulated relayer, virtual pUSD reservation, fee routing, SQLite persistence, dashboard, Telegram, and live-mode gates all depend on the same simulated order outcome. Keep the work in the order below so each slice compiles and validates before the next one.

## File Structure

**Create:**

- `polybot-core/src/execution/v2_order.rs` - V2 order payload types, builder-code bytes32 parsing, timestamp helpers, CLOB side mapping.
- `polybot-core/src/execution/v2_signing.rs` - EIP-712 v2 domain representation and stable signing payload serialization for simulation/readiness checks.
- `polybot-core/src/execution/v2_relayer.rs` - relayer submit/poll request and response models, relayer state mapping, transaction-record conversion.
- `polybot-core/src/execution/v2_sim.rs` - deterministic simulated relayer transport.
- `polybot-core/src/execution/v2_flow.rs` - pure orchestration helpers that convert an `Order` plus relayer outcome into `Trade` and `TransactionRecord`.
- `polybot-core/src/state/pusd.rs` - virtual pUSD account reservation/release/fill helper.

**Modify:**

- `polybot-common/src/types.rs` - restore v1 Trade JSON compatibility by defaulting missing `source_wallet`; keep V2 fields defaulted.
- `polybot-core/src/execution/mod.rs` - register new modules and route simulation execution through the V2 simulated lifecycle.
- `polybot-core/src/execution/order_builder.rs` - add fee-aware V2 order type selection helpers.
- `polybot-core/src/state/mod.rs` - persist transaction rows for simulated V2 trades and update daily fee/rebate stats.
- `polybot-core/src/state/sqlite.rs` - repair stale fixtures and expose non-terminal transactions for operator surfaces.
- `polybot-core/src/metrics.rs` - add virtual pUSD, reserved pUSD, fees, and rebates snapshots.
- `polybot-core/src/health.rs` - expose V2 simulation fields and `/transactions`.
- `polybot-dashboard/src/data.rs` - add V2 health fields and transaction DTOs.
- `polybot-dashboard/src/app.rs` - display V2 simulation state.
- `polybot-core/src/telegram_bot/commands.rs` - include V2 simulation state in `/status`.
- `polybot-core/src/setup.rs` - block live mode with clear V2 readiness reasons.
- `README.md` and `docs/superpowers/V3_2_MIGRATION_STATUS.md` - document the simulation-complete milestone and live-readiness follow-up.

---

### Task 1: Restore the Test Baseline

**Files:**
- Modify: `polybot-common/src/types.rs`
- Modify: `polybot-core/src/state/sqlite.rs`
- Test: `polybot-common/src/types.rs`
- Test: `polybot-core/src/state/sqlite.rs`

- [ ] **Step 1: Run the current failing tests to capture RED**

Run:

```powershell
cargo test --workspace
```

Expected: FAIL before running `polybot-core` tests with missing `fee_schedule` on a `Signal` fixture and missing V2 `Trade` fields in `polybot-core/src/state/sqlite.rs`.

- [ ] **Step 2: Restore old Trade JSON compatibility**

In `polybot-common/src/types.rs`, add this helper near the other default helpers:

```rust
fn default_source_wallet() -> String {
    String::new()
}
```

Then change the `Trade` struct field from:

```rust
pub source_wallet: String,
```

to:

```rust
#[serde(default = "default_source_wallet")]
pub source_wallet: String,
```

This preserves old stored JSON payloads while new runtime trades still carry the copied wallet.

- [ ] **Step 3: Repair the stale Signal fixture**

In `polybot-core/src/state/sqlite.rs`, inside `get_signal_round_trip_preserves_direction`, add this field after `suggested_size_usdc: None,`:

```rust
fee_schedule: None,
```

- [ ] **Step 4: Repair the stale Trade fixture**

In `polybot-core/src/state/sqlite.rs`, inside `latest_trades_round_trip_preserves_source_wallet_and_direction`, add these fields after `simulated: true,`:

```rust
transaction_id: None,
transaction_hash: None,
relayer_state: None,
taker_fee_bps: 0,
fee_paid_usdc: Decimal::ZERO,
rebate_usdc: Decimal::ZERO,
retry_count: 0,
error_msg: None,
```

- [ ] **Step 5: Run package tests**

Run:

```powershell
cargo test -p polybot-common
cargo test -p polybot-core get_signal_round_trip_preserves_direction
cargo test -p polybot-core latest_trades_round_trip_preserves_source_wallet_and_direction
```

Expected: PASS for `polybot-common` and both focused `polybot-core` tests.

- [ ] **Step 6: Run the workspace test suite**

Run:

```powershell
cargo test --workspace
```

Expected: PASS or reveal the next real runtime failure after fixture repair. If another compile error appears from the same V2-field drift, fix only that fixture with explicit default values matching the struct definitions.

- [ ] **Step 7: Commit**

Run:

```powershell
git add -- polybot-common/src/types.rs polybot-core/src/state/sqlite.rs
git commit -m "test: restore v3.2 workspace baseline"
```

---

### Task 2: Add V2 Order Payload Types

**Files:**
- Create: `polybot-core/src/execution/v2_order.rs`
- Modify: `polybot-core/src/execution/mod.rs`
- Test: `polybot-core/src/execution/v2_order.rs`

- [ ] **Step 1: Register the future module with a failing import**

In `polybot-core/src/execution/mod.rs`, add:

```rust
pub mod v2_order;
```

Run:

```powershell
cargo test -p polybot-core builder_code_accepts_bytes32_hex
```

Expected: FAIL because `polybot-core/src/execution/v2_order.rs` does not exist.

- [ ] **Step 2: Create the V2 order module**

Create `polybot-core/src/execution/v2_order.rs` with this content:

```rust
use polybot_common::errors::PolybotError;
use polybot_common::types::{FeeSchedule, OrderType, TradeDirection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuilderCode(pub [u8; 32]);

impl BuilderCode {
    pub fn as_hex(self) -> String {
        let mut out = String::with_capacity(66);
        out.push_str("0x");
        for byte in self.0 {
            out.push_str(&format!("{:02x}", byte));
        }
        out
    }
}

pub fn parse_builder_code(raw: &str) -> Result<BuilderCode, PolybotError> {
    let value = raw.trim();
    let Some(hex) = value.strip_prefix("0x") else {
        return Err(PolybotError::Config(
            "BUILDER_CODE must be 0x-prefixed bytes32 hex".to_string(),
        ));
    };
    if hex.len() != 64 {
        return Err(PolybotError::Config(format!(
            "BUILDER_CODE must be 32 bytes, got {} hex chars",
            hex.len()
        )));
    }

    let mut out = [0u8; 32];
    for idx in 0..32 {
        let start = idx * 2;
        out[idx] = decode_hex_byte(&hex[start..start + 2])?;
    }
    Ok(BuilderCode(out))
}

fn decode_hex_byte(raw: &str) -> Result<u8, PolybotError> {
    u8::from_str_radix(raw, 16).map_err(|_| {
        PolybotError::Config(format!("BUILDER_CODE contains non-hex byte '{}'", raw))
    })
}

pub fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn clob_side(direction: TradeDirection) -> u8 {
    match direction {
        TradeDirection::Buy => 0,
        TradeDirection::Sell => 1,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct V2OrderPayload {
    pub salt: u64,
    pub maker: String,
    pub signer: String,
    pub token_id: String,
    pub maker_amount: String,
    pub taker_amount: String,
    pub side: u8,
    pub signature_type: u8,
    pub timestamp_ms: u64,
    pub metadata: BuilderCode,
    pub builder: BuilderCode,
    pub order_type: OrderType,
    pub fee_schedule: Option<FeeSchedule>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_code_accepts_bytes32_hex() {
        let raw = "0x00000000000000000000000000000000000000000000000000000000deadbeef";
        let parsed = parse_builder_code(raw).unwrap();
        assert_eq!(parsed.as_hex(), raw);
    }

    #[test]
    fn builder_code_rejects_wrong_length() {
        let err = parse_builder_code("0xdeadbeef").unwrap_err();
        assert!(format!("{}", err).contains("32 bytes"));
    }

    #[test]
    fn builder_code_rejects_non_hex() {
        let err = parse_builder_code(
            "0xzz00000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(format!("{}", err).contains("non-hex"));
    }

    #[test]
    fn clob_side_maps_buy_sell() {
        assert_eq!(clob_side(TradeDirection::Buy), 0);
        assert_eq!(clob_side(TradeDirection::Sell), 1);
    }
}
```

- [ ] **Step 3: Run focused V2 order tests**

Run:

```powershell
cargo test -p polybot-core v2_order
```

Expected: PASS.

- [ ] **Step 4: Commit**

Run:

```powershell
git add -- polybot-core/src/execution/mod.rs polybot-core/src/execution/v2_order.rs
git commit -m "feat: add v2 order payload primitives"
```

---

### Task 3: Add EIP-712 V2 Domain Serialization

**Files:**
- Create: `polybot-core/src/execution/v2_signing.rs`
- Modify: `polybot-core/src/execution/mod.rs`
- Test: `polybot-core/src/execution/v2_signing.rs`

- [ ] **Step 1: Register the signing module**

In `polybot-core/src/execution/mod.rs`, add:

```rust
pub mod v2_signing;
```

Run:

```powershell
cargo test -p polybot-core eip712_domain_uses_version_2
```

Expected: FAIL because `v2_signing.rs` does not exist.

- [ ] **Step 2: Create the signing boundary module**

Create `polybot-core/src/execution/v2_signing.rs` with this content:

```rust
use polybot_common::errors::PolybotError;
use serde::{Deserialize, Serialize};

use super::v2_order::V2OrderPayload;

pub const EIP712_DOMAIN_NAME: &str = "Polymarket CTF Exchange";
pub const EIP712_DOMAIN_VERSION: &str = "2";
pub const POLYGON_CHAIN_ID: u64 = 137;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Eip712DomainV2 {
    pub name: String,
    pub version: String,
    pub chain_id: u64,
    pub verifying_contract: String,
}

pub fn exchange_domain(verifying_contract: &str) -> Result<Eip712DomainV2, PolybotError> {
    validate_address(verifying_contract)?;
    Ok(Eip712DomainV2 {
        name: EIP712_DOMAIN_NAME.to_string(),
        version: EIP712_DOMAIN_VERSION.to_string(),
        chain_id: POLYGON_CHAIN_ID,
        verifying_contract: verifying_contract.to_string(),
    })
}

pub fn validate_address(address: &str) -> Result<(), PolybotError> {
    let valid = address.starts_with("0x")
        && address.len() == 42
        && address[2..].chars().all(|ch| ch.is_ascii_hexdigit());
    if valid {
        Ok(())
    } else {
        Err(PolybotError::Config(format!(
            "invalid EIP-712 verifying contract address: {}",
            address
        )))
    }
}

pub fn signing_payload_json(
    domain: &Eip712DomainV2,
    order: &V2OrderPayload,
) -> serde_json::Value {
    serde_json::json!({
        "domain": domain,
        "primaryType": "Order",
        "message": {
            "salt": order.salt,
            "maker": order.maker,
            "signer": order.signer,
            "tokenId": order.token_id,
            "makerAmount": order.maker_amount,
            "takerAmount": order.taker_amount,
            "side": order.side,
            "signatureType": order.signature_type,
            "timestamp": order.timestamp_ms,
            "metadata": order.metadata.as_hex(),
            "builder": order.builder.as_hex(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::v2_order::{parse_builder_code, V2OrderPayload};
    use polybot_common::types::OrderType;

    #[test]
    fn eip712_domain_uses_version_2() {
        let domain = exchange_domain("0xE111180000d2663C0091e4f400237545B87B996B").unwrap();
        assert_eq!(domain.name, "Polymarket CTF Exchange");
        assert_eq!(domain.version, "2");
        assert_eq!(domain.chain_id, 137);
    }

    #[test]
    fn invalid_verifying_contract_is_rejected() {
        let err = exchange_domain("0xabc").unwrap_err();
        assert!(format!("{}", err).contains("invalid EIP-712"));
    }

    #[test]
    fn signing_payload_contains_builder_and_timestamp() {
        let builder =
            parse_builder_code("0x00000000000000000000000000000000000000000000000000000000deadbeef")
                .unwrap();
        let order = V2OrderPayload {
            salt: 42,
            maker: "0x0000000000000000000000000000000000000001".to_string(),
            signer: "0x0000000000000000000000000000000000000001".to_string(),
            token_id: "123".to_string(),
            maker_amount: "1000000".to_string(),
            taker_amount: "1000000".to_string(),
            side: 0,
            signature_type: 0,
            timestamp_ms: 1_771_000_000_000,
            metadata: builder,
            builder,
            order_type: OrderType::Limit,
            fee_schedule: None,
        };
        let domain = exchange_domain("0xE111180000d2663C0091e4f400237545B87B996B").unwrap();
        let payload = signing_payload_json(&domain, &order);
        assert_eq!(payload["primaryType"], "Order");
        assert_eq!(payload["message"]["timestamp"], 1_771_000_000_000u64);
        assert_eq!(
            payload["message"]["builder"],
            "0x00000000000000000000000000000000000000000000000000000000deadbeef"
        );
    }
}
```

- [ ] **Step 3: Run focused signing tests**

Run:

```powershell
cargo test -p polybot-core v2_signing
```

Expected: PASS.

- [ ] **Step 4: Commit**

Run:

```powershell
git add -- polybot-core/src/execution/mod.rs polybot-core/src/execution/v2_signing.rs
git commit -m "feat: add v2 signing payload boundary"
```

---

### Task 4: Add Relayer Models and Simulated Relayer

**Files:**
- Create: `polybot-core/src/execution/v2_relayer.rs`
- Create: `polybot-core/src/execution/v2_sim.rs`
- Modify: `polybot-core/src/execution/mod.rs`
- Test: `polybot-core/src/execution/v2_relayer.rs`
- Test: `polybot-core/src/execution/v2_sim.rs`

- [ ] **Step 1: Register modules**

In `polybot-core/src/execution/mod.rs`, add:

```rust
pub mod v2_relayer;
pub mod v2_sim;
```

Run:

```powershell
cargo test -p polybot-core relayer_state_parses_terminal_and_non_terminal_values
```

Expected: FAIL because the modules do not exist.

- [ ] **Step 2: Create relayer models**

Create `polybot-core/src/execution/v2_relayer.rs` with this content:

```rust
use chrono::Utc;
use polybot_common::errors::PolybotError;
use polybot_common::types::{TransactionKind, TransactionRecord, TransactionState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayerSubmitRequest {
    pub signed_order: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayerSubmitResponse {
    #[serde(rename = "transactionID")]
    pub transaction_id: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayerTransactionResponse {
    #[serde(rename = "transactionID")]
    pub transaction_id: String,
    pub state: String,
    #[serde(default)]
    pub transaction_hash: Option<String>,
    #[serde(default)]
    pub error_msg: Option<String>,
}

pub fn map_transaction_state(raw: &str) -> Result<TransactionState, PolybotError> {
    match raw {
        "STATE_NEW" => Ok(TransactionState::New),
        "STATE_PENDING" => Ok(TransactionState::Pending),
        "STATE_SUBMITTED" => Ok(TransactionState::Submitted),
        "STATE_SUCCESS" => Ok(TransactionState::Success),
        "STATE_FAILED" => Ok(TransactionState::Failed),
        other => Err(PolybotError::Execution(format!(
            "unknown relayer transaction state: {}",
            other
        ))),
    }
}

pub fn transaction_record_from_submit(
    response: &RelayerSubmitResponse,
    trade_id: Option<String>,
) -> Result<TransactionRecord, PolybotError> {
    Ok(TransactionRecord {
        transaction_id: response.transaction_id.clone(),
        trade_id,
        kind: TransactionKind::Order,
        state: map_transaction_state(&response.state)?,
        submitted_at: Utc::now(),
        confirmed_at: None,
        transaction_hash: None,
        error_msg: None,
    })
}

pub fn apply_poll_response(
    record: &mut TransactionRecord,
    response: &RelayerTransactionResponse,
) -> Result<(), PolybotError> {
    record.state = map_transaction_state(&response.state)?;
    record.transaction_hash = response.transaction_hash.clone();
    record.error_msg = response.error_msg.clone();
    if record.state.is_terminal() {
        record.confirmed_at = Some(Utc::now());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relayer_state_parses_terminal_and_non_terminal_values() {
        assert!(!map_transaction_state("STATE_NEW").unwrap().is_terminal());
        assert!(!map_transaction_state("STATE_PENDING").unwrap().is_terminal());
        assert!(map_transaction_state("STATE_SUCCESS").unwrap().is_terminal());
        assert!(map_transaction_state("STATE_FAILED").unwrap().is_terminal());
    }

    #[test]
    fn submit_response_creates_transaction_record() {
        let response = RelayerSubmitResponse {
            transaction_id: "txn_abc".to_string(),
            state: "STATE_NEW".to_string(),
        };
        let record = transaction_record_from_submit(&response, Some("trade-1".to_string())).unwrap();
        assert_eq!(record.transaction_id, "txn_abc");
        assert_eq!(record.trade_id.as_deref(), Some("trade-1"));
        assert_eq!(record.state, TransactionState::New);
    }

    #[test]
    fn poll_response_marks_terminal_confirmation() {
        let response = RelayerSubmitResponse {
            transaction_id: "txn_abc".to_string(),
            state: "STATE_NEW".to_string(),
        };
        let mut record = transaction_record_from_submit(&response, Some("trade-1".to_string())).unwrap();
        let poll = RelayerTransactionResponse {
            transaction_id: "txn_abc".to_string(),
            state: "STATE_SUCCESS".to_string(),
            transaction_hash: Some("0xabc".to_string()),
            error_msg: None,
        };
        apply_poll_response(&mut record, &poll).unwrap();
        assert_eq!(record.state, TransactionState::Success);
        assert_eq!(record.transaction_hash.as_deref(), Some("0xabc"));
        assert!(record.confirmed_at.is_some());
    }
}
```

- [ ] **Step 3: Create the deterministic simulated relayer**

Create `polybot-core/src/execution/v2_sim.rs` with this content:

```rust
use polybot_common::errors::PolybotError;
use polybot_common::types::TransactionState;

use super::v2_relayer::{RelayerSubmitResponse, RelayerTransactionResponse};

#[derive(Debug, Clone)]
pub struct SimulatedRelayer {
    transaction_id: String,
    states: Vec<TransactionState>,
    poll_index: usize,
}

impl SimulatedRelayer {
    pub fn success(transaction_id: impl Into<String>) -> Self {
        Self::scripted(
            transaction_id,
            vec![
                TransactionState::New,
                TransactionState::Pending,
                TransactionState::Submitted,
                TransactionState::Success,
            ],
        )
    }

    pub fn failed(transaction_id: impl Into<String>) -> Self {
        Self::scripted(
            transaction_id,
            vec![
                TransactionState::New,
                TransactionState::Pending,
                TransactionState::Submitted,
                TransactionState::Failed,
            ],
        )
    }

    pub fn scripted(transaction_id: impl Into<String>, states: Vec<TransactionState>) -> Self {
        Self {
            transaction_id: transaction_id.into(),
            states,
            poll_index: 0,
        }
    }

    pub fn submit(&self) -> Result<RelayerSubmitResponse, PolybotError> {
        let Some(first) = self.states.first() else {
            return Err(PolybotError::Execution(
                "simulated relayer requires at least one state".to_string(),
            ));
        };
        Ok(RelayerSubmitResponse {
            transaction_id: self.transaction_id.clone(),
            state: first.as_sqlite_str().to_string(),
        })
    }

    pub fn poll(&mut self) -> Result<RelayerTransactionResponse, PolybotError> {
        if self.states.is_empty() {
            return Err(PolybotError::Execution(
                "simulated relayer requires at least one state".to_string(),
            ));
        }
        self.poll_index = (self.poll_index + 1).min(self.states.len() - 1);
        let state = self.states[self.poll_index];
        Ok(RelayerTransactionResponse {
            transaction_id: self.transaction_id.clone(),
            state: state.as_sqlite_str().to_string(),
            transaction_hash: state
                .is_terminal()
                .then(|| format!("0xsim{}", self.transaction_id.replace('-', ""))),
            error_msg: (state == TransactionState::Failed)
                .then(|| "simulated relayer failure".to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulated_relayer_advances_to_success() {
        let mut relayer = SimulatedRelayer::success("sim-txn-1");
        let submit = relayer.submit().unwrap();
        assert_eq!(submit.state, "STATE_NEW");

        assert_eq!(relayer.poll().unwrap().state, "STATE_PENDING");
        assert_eq!(relayer.poll().unwrap().state, "STATE_SUBMITTED");
        let final_state = relayer.poll().unwrap();
        assert_eq!(final_state.state, "STATE_SUCCESS");
        assert!(final_state.transaction_hash.is_some());
    }

    #[test]
    fn simulated_relayer_can_fail() {
        let mut relayer = SimulatedRelayer::failed("sim-txn-2");
        let _ = relayer.submit().unwrap();
        let _ = relayer.poll().unwrap();
        let _ = relayer.poll().unwrap();
        let final_state = relayer.poll().unwrap();
        assert_eq!(final_state.state, "STATE_FAILED");
        assert_eq!(final_state.error_msg.as_deref(), Some("simulated relayer failure"));
    }
}
```

- [ ] **Step 4: Run relayer tests**

Run:

```powershell
cargo test -p polybot-core v2_relayer
cargo test -p polybot-core v2_sim
```

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```powershell
git add -- polybot-core/src/execution/mod.rs polybot-core/src/execution/v2_relayer.rs polybot-core/src/execution/v2_sim.rs
git commit -m "feat: add simulated v2 relayer lifecycle"
```

---

### Task 5: Add Virtual pUSD Accounting and Metrics

**Files:**
- Create: `polybot-core/src/state/pusd.rs`
- Modify: `polybot-core/src/state/mod.rs`
- Modify: `polybot-core/src/metrics.rs`
- Test: `polybot-core/src/state/pusd.rs`
- Test: `polybot-core/src/metrics.rs`

- [ ] **Step 1: Register the pUSD module**

In `polybot-core/src/state/mod.rs`, add:

```rust
pub mod pusd;
```

Run:

```powershell
cargo test -p polybot-core virtual_pusd_reserve_release_and_fill
```

Expected: FAIL because `pusd.rs` does not exist.

- [ ] **Step 2: Create virtual pUSD account helper**

Create `polybot-core/src/state/pusd.rs` with this content:

```rust
use polybot_common::errors::PolybotError;
use rust_decimal::Decimal;

#[derive(Debug, Clone, PartialEq)]
pub struct VirtualPusdAccount {
    available: Decimal,
    reserved: Decimal,
    fees_paid: Decimal,
    rebates_earned: Decimal,
}

impl VirtualPusdAccount {
    pub fn new(starting_balance: Decimal) -> Self {
        Self {
            available: starting_balance.max(Decimal::ZERO),
            reserved: Decimal::ZERO,
            fees_paid: Decimal::ZERO,
            rebates_earned: Decimal::ZERO,
        }
    }

    pub fn available(&self) -> Decimal {
        self.available
    }

    pub fn reserved(&self) -> Decimal {
        self.reserved
    }

    pub fn fees_paid(&self) -> Decimal {
        self.fees_paid
    }

    pub fn rebates_earned(&self) -> Decimal {
        self.rebates_earned
    }

    pub fn reserve(&mut self, amount: Decimal) -> Result<(), PolybotError> {
        if amount <= Decimal::ZERO {
            return Err(PolybotError::Risk("pUSD reserve amount must be positive".to_string()));
        }
        if self.available < amount {
            return Err(PolybotError::Risk(format!(
                "insufficient virtual pUSD: available={}, required={}",
                self.available, amount
            )));
        }
        self.available -= amount;
        self.reserved += amount;
        Ok(())
    }

    pub fn release(&mut self, amount: Decimal) -> Result<(), PolybotError> {
        if amount <= Decimal::ZERO {
            return Err(PolybotError::Risk("pUSD release amount must be positive".to_string()));
        }
        let released = amount.min(self.reserved);
        self.reserved -= released;
        self.available += released;
        Ok(())
    }

    pub fn fill(&mut self, reserved_spend: Decimal, fee: Decimal, rebate: Decimal) -> Result<(), PolybotError> {
        if reserved_spend <= Decimal::ZERO {
            return Err(PolybotError::Risk("pUSD fill amount must be positive".to_string()));
        }
        if self.reserved < reserved_spend {
            return Err(PolybotError::Risk(format!(
                "insufficient reserved pUSD: reserved={}, fill={}",
                self.reserved, reserved_spend
            )));
        }
        self.reserved -= reserved_spend;
        self.fees_paid += fee.max(Decimal::ZERO);
        self.rebates_earned += rebate.max(Decimal::ZERO);
        self.available += rebate.max(Decimal::ZERO);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn virtual_pusd_reserve_release_and_fill() {
        let mut account = VirtualPusdAccount::new(dec!(100));
        account.reserve(dec!(25)).unwrap();
        assert_eq!(account.available(), dec!(75));
        assert_eq!(account.reserved(), dec!(25));

        account.release(dec!(5)).unwrap();
        assert_eq!(account.available(), dec!(80));
        assert_eq!(account.reserved(), dec!(20));

        account.fill(dec!(20), dec!(0.10), dec!(0.02)).unwrap();
        assert_eq!(account.available(), dec!(80.02));
        assert_eq!(account.reserved(), dec!(0));
        assert_eq!(account.fees_paid(), dec!(0.10));
        assert_eq!(account.rebates_earned(), dec!(0.02));
    }

    #[test]
    fn virtual_pusd_rejects_insufficient_balance() {
        let mut account = VirtualPusdAccount::new(dec!(10));
        let err = account.reserve(dec!(11)).unwrap_err();
        assert!(format!("{}", err).contains("insufficient virtual pUSD"));
    }
}
```

- [ ] **Step 3: Add V2 metric snapshots**

In `polybot-core/src/metrics.rs`, add these fields to `Metrics` after `daily_pnl_cents`:

```rust
pub virtual_pusd_cents: AtomicI64,
pub reserved_pusd_cents: AtomicI64,
pub fees_paid_cents: AtomicI64,
pub rebates_earned_cents: AtomicI64,
```

Initialize them in `Metrics::new()`:

```rust
virtual_pusd_cents: AtomicI64::new(0),
reserved_pusd_cents: AtomicI64::new(0),
fees_paid_cents: AtomicI64::new(0),
rebates_earned_cents: AtomicI64::new(0),
```

Add these methods to `impl Metrics`:

```rust
pub fn update_v2_accounting(
    &self,
    virtual_pusd: rust_decimal::Decimal,
    reserved_pusd: rust_decimal::Decimal,
    fees_paid: rust_decimal::Decimal,
    rebates_earned: rust_decimal::Decimal,
) {
    self.virtual_pusd_cents.store(decimal_to_cents(virtual_pusd), Ordering::Relaxed);
    self.reserved_pusd_cents.store(decimal_to_cents(reserved_pusd), Ordering::Relaxed);
    self.fees_paid_cents.store(decimal_to_cents(fees_paid), Ordering::Relaxed);
    self.rebates_earned_cents.store(decimal_to_cents(rebates_earned), Ordering::Relaxed);
}

pub fn virtual_pusd(&self) -> f64 {
    self.virtual_pusd_cents.load(Ordering::Relaxed) as f64 / 100.0
}

pub fn reserved_pusd(&self) -> f64 {
    self.reserved_pusd_cents.load(Ordering::Relaxed) as f64 / 100.0
}

pub fn fees_paid(&self) -> f64 {
    self.fees_paid_cents.load(Ordering::Relaxed) as f64 / 100.0
}

pub fn rebates_earned(&self) -> f64 {
    self.rebates_earned_cents.load(Ordering::Relaxed) as f64 / 100.0
}
```

Add this private helper near the top of `metrics.rs`:

```rust
fn decimal_to_cents(value: rust_decimal::Decimal) -> i64 {
    use rust_decimal::prelude::ToPrimitive;
    (value * rust_decimal::Decimal::new(100, 0))
        .round()
        .to_i64()
        .unwrap_or(0)
}
```

- [ ] **Step 4: Add metrics test**

In `polybot-core/src/metrics.rs`, add this test:

```rust
#[test]
fn v2_accounting_metrics_round_trip() {
    let m = Metrics::new();
    m.update_v2_accounting(
        rust_decimal::Decimal::new(12345, 2),
        rust_decimal::Decimal::new(2500, 2),
        rust_decimal::Decimal::new(15, 2),
        rust_decimal::Decimal::new(7, 2),
    );
    assert!((m.virtual_pusd() - 123.45).abs() < 0.01);
    assert!((m.reserved_pusd() - 25.00).abs() < 0.01);
    assert!((m.fees_paid() - 0.15).abs() < 0.01);
    assert!((m.rebates_earned() - 0.07).abs() < 0.01);
}
```

- [ ] **Step 5: Run tests**

Run:

```powershell
cargo test -p polybot-core virtual_pusd
cargo test -p polybot-core v2_accounting_metrics_round_trip
```

Expected: PASS.

- [ ] **Step 6: Commit**

Run:

```powershell
git add -- polybot-core/src/state/mod.rs polybot-core/src/state/pusd.rs polybot-core/src/metrics.rs
git commit -m "feat: add virtual pusd accounting"
```

---

### Task 6: Add Fee-Aware V2 Routing

**Files:**
- Modify: `polybot-core/src/execution/order_builder.rs`
- Test: `polybot-core/src/execution/order_builder.rs`

- [ ] **Step 1: Add failing fee routing tests**

In `polybot-core/src/execution/order_builder.rs`, inside the existing test module, add:

```rust
#[test]
fn v2_routing_defaults_to_limit_when_fee_schedule_missing() {
    let decision = test_decision(dec!(2), dec!(2));
    assert_eq!(select_v2_order_type(&decision, None, 50), OrderType::Limit);
}

#[test]
fn v2_routing_allows_fok_under_fee_threshold() {
    let decision = test_decision(dec!(2), dec!(2));
    let fee = FeeSchedule {
        taker_fee_bps: 2500,
        maker_fee_bps: 0,
        rebate_bps: 500,
    };
    assert_eq!(select_v2_order_type(&decision, Some(fee), 50), OrderType::Fok);
}

#[test]
fn v2_routing_uses_limit_when_taker_fee_exceeds_threshold() {
    let decision = test_decision(dec!(2), dec!(2));
    let fee = FeeSchedule {
        taker_fee_bps: 12500,
        maker_fee_bps: 0,
        rebate_bps: 500,
    };
    assert_eq!(select_v2_order_type(&decision, Some(fee), 50), OrderType::Limit);
}
```

Run:

```powershell
cargo test -p polybot-core v2_routing
```

Expected: FAIL because `select_v2_order_type` does not exist.

- [ ] **Step 2: Add the V2 routing helper**

In `polybot-core/src/execution/order_builder.rs`, below `select_order_type`, add:

```rust
pub fn select_v2_order_type(
    decision: &RiskDecision,
    fee_schedule: Option<FeeSchedule>,
    fok_max_fee_bps: u32,
) -> OrderType {
    let combined = decision.confidence_multiplier * decision.secret_level_multiplier;
    let taker_fee_allowed = fee_schedule
        .map(|fees| fees.taker_bps_true() <= fok_max_fee_bps)
        .unwrap_or(false);

    if combined >= rust_decimal_macros::dec!(1.5) && taker_fee_allowed {
        OrderType::Fok
    } else {
        OrderType::Limit
    }
}
```

Ensure `FeeSchedule` is available from the existing `use polybot_common::types::*;` import.

- [ ] **Step 3: Run focused tests**

Run:

```powershell
cargo test -p polybot-core v2_routing
```

Expected: PASS.

- [ ] **Step 4: Commit**

Run:

```powershell
git add -- polybot-core/src/execution/order_builder.rs
git commit -m "feat: add fee-aware v2 order routing"
```

---

### Task 7: Convert Simulated Relayer Outcomes Into Trades

**Files:**
- Create: `polybot-core/src/execution/v2_flow.rs`
- Modify: `polybot-core/src/execution/mod.rs`
- Test: `polybot-core/src/execution/v2_flow.rs`

- [ ] **Step 1: Register the flow module**

In `polybot-core/src/execution/mod.rs`, add:

```rust
pub mod v2_flow;
```

Run:

```powershell
cargo test -p polybot-core simulated_v2_success_creates_filled_trade_and_transaction
```

Expected: FAIL because `v2_flow.rs` does not exist.

- [ ] **Step 2: Create V2 flow helper**

Create `polybot-core/src/execution/v2_flow.rs` with this content:

```rust
use chrono::Utc;
use polybot_common::errors::PolybotError;
use polybot_common::types::{Trade, TradeStatus, TransactionRecord, TransactionState};
use rust_decimal::Decimal;

use super::order_builder::Order;
use super::v2_relayer::{apply_poll_response, transaction_record_from_submit};
use super::v2_sim::SimulatedRelayer;

#[derive(Debug, Clone)]
pub struct V2ExecutionOutcome {
    pub trade: Trade,
    pub transaction: TransactionRecord,
}

pub fn simulate_v2_success(order: &Order) -> Result<V2ExecutionOutcome, PolybotError> {
    simulate_with_relayer(order, SimulatedRelayer::success(format!("sim-{}", order.signal_id)))
}

pub fn simulate_with_relayer(
    order: &Order,
    mut relayer: SimulatedRelayer,
) -> Result<V2ExecutionOutcome, PolybotError> {
    let submit = relayer.submit()?;
    let mut transaction = transaction_record_from_submit(&submit, None)?;

    while !transaction.state.is_terminal() {
        let poll = relayer.poll()?;
        apply_poll_response(&mut transaction, &poll)?;
    }

    let trade_status = match transaction.state {
        TransactionState::Success => TradeStatus::Filled,
        TransactionState::Failed => TradeStatus::Failed(
            transaction
                .error_msg
                .clone()
                .unwrap_or_else(|| "simulated relayer failed".to_string()),
        ),
        _ => TradeStatus::Pending,
    };

    let filled_size = if matches!(trade_status, TradeStatus::Filled) {
        order.size
    } else {
        Decimal::ZERO
    };

    let trade_id = uuid::Uuid::new_v4().to_string();
    transaction.trade_id = Some(trade_id.clone());

    Ok(V2ExecutionOutcome {
        trade: Trade {
            id: trade_id,
            signal_id: order.signal_id.clone(),
            source_wallet: order.source_wallet.clone(),
            market_id: order.market_id.clone(),
            category: order.category,
            side: order.side,
            direction: order.direction,
            price: order.price,
            size: order.size,
            size_usd: order.size_usd,
            filled_size,
            order_type: order.order_type,
            status: trade_status,
            placed_at: Utc::now(),
            filled_at: transaction.state.is_terminal().then(Utc::now),
            simulated: true,
            transaction_id: Some(transaction.transaction_id.clone()),
            transaction_hash: transaction.transaction_hash.clone(),
            relayer_state: Some(transaction.state),
            taker_fee_bps: 0,
            fee_paid_usdc: Decimal::ZERO,
            rebate_usdc: Decimal::ZERO,
            retry_count: 0,
            error_msg: transaction.error_msg.clone(),
        },
        transaction,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::order_builder::{build_order, Order};
    use crate::execution::v2_sim::SimulatedRelayer;
    use polybot_common::types::{
        Category, Decision, OrderType, RiskDecision, Side, TradeDirection, TradeStatus,
        TransactionState,
    };
    use rust_decimal_macros::dec;

    fn test_order() -> Order {
        let decision = RiskDecision {
            signal_id: "sig-1".to_string(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "market-1".to_string(),
            side: Side::Yes,
            direction: TradeDirection::Buy,
            category: Category::Politics,
            position_size_usd: dec!(50),
            target_size_tokens: None,
            confidence_multiplier: dec!(1),
            secret_level_multiplier: dec!(1),
            drawdown_factor: dec!(1),
            blocked: false,
            manual_review: false,
            decision: Decision::Execute,
        };
        let ctx = crate::execution::clob_client::MarketContext {
            token_id: "token-1".to_string(),
            tick_size: dec!(0.01),
            min_order_size: dec!(1),
            neg_risk: false,
        };
        let mut order = build_order(&decision, &ctx, dec!(0.50), dec!(50));
        order.order_type = OrderType::Limit;
        order
    }

    #[test]
    fn simulated_v2_success_creates_filled_trade_and_transaction() {
        let outcome = simulate_v2_success(&test_order()).unwrap();
        assert_eq!(outcome.trade.status, TradeStatus::Filled);
        assert!(outcome.trade.simulated);
        assert!(outcome.trade.transaction_id.is_some());
        assert_eq!(outcome.transaction.state, TransactionState::Success);
        assert_eq!(outcome.transaction.trade_id.as_deref(), Some(outcome.trade.id.as_str()));
    }

    #[test]
    fn simulated_v2_failure_creates_failed_trade_and_transaction() {
        let outcome = simulate_with_relayer(&test_order(), SimulatedRelayer::failed("sim-fail")).unwrap();
        assert!(matches!(outcome.trade.status, TradeStatus::Failed(_)));
        assert_eq!(outcome.trade.filled_size, Decimal::ZERO);
        assert_eq!(outcome.transaction.state, TransactionState::Failed);
    }
}
```

- [ ] **Step 3: Run focused flow tests**

Run:

```powershell
cargo test -p polybot-core simulated_v2
```

Expected: PASS.

- [ ] **Step 4: Commit**

Run:

```powershell
git add -- polybot-core/src/execution/mod.rs polybot-core/src/execution/v2_flow.rs
git commit -m "feat: map simulated v2 relayer outcomes"
```

---

### Task 8: Persist V2 Transaction Lifecycle in State Manager

**Files:**
- Modify: `polybot-core/src/state/mod.rs`
- Modify: `polybot-core/src/state/sqlite.rs`
- Test: `polybot-core/src/state/mod.rs`
- Test: `polybot-core/src/state/sqlite.rs`

- [ ] **Step 1: Add SQLite helper for relayer queue**

In `polybot-core/src/state/sqlite.rs`, add this method near `list_non_terminal_transactions`:

```rust
pub fn latest_transactions(
    &self,
    limit: usize,
) -> Result<Vec<polybot_common::types::TransactionRecord>, PolybotError> {
    let mut stmt = self
        .conn
        .prepare(
            "SELECT transaction_id, trade_id, type, state, submitted_at, confirmed_at, transaction_hash, error_msg
             FROM transactions
             ORDER BY submitted_at DESC
             LIMIT ?1",
        )
        .map_err(|e| PolybotError::State(format!("Failed to prepare latest transactions: {}", e)))?;

    let rows = stmt
        .query_map([limit as i64], |row| {
            Ok(transaction_record_from_row(
                row.get(0)?,
                row.get(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        })
        .map_err(|e| PolybotError::State(format!("Failed to query latest transactions: {}", e)))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| PolybotError::State(format!("Failed to collect latest transactions: {}", e)))?
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
}
```

Run:

```powershell
cargo test -p polybot-core latest_transactions
```

Expected: FAIL if a helper function signature needs adjustment. Fix the call to reuse the existing row-mapping helper already present in `sqlite.rs`.

- [ ] **Step 2: Add transaction persistence helper**

In `polybot-core/src/state/mod.rs`, add this helper above `run_state_manager`:

```rust
fn persist_transaction_from_trade(
    store: &sqlite::SqliteStore,
    trade: &Trade,
) -> Result<(), PolybotError> {
    let Some(transaction_id) = trade.transaction_id.clone() else {
        return Ok(());
    };

    let state = trade
        .relayer_state
        .unwrap_or(polybot_common::types::TransactionState::New);
    let record = polybot_common::types::TransactionRecord {
        transaction_id,
        trade_id: Some(trade.id.clone()),
        kind: polybot_common::types::TransactionKind::Order,
        state,
        submitted_at: trade.placed_at,
        confirmed_at: state.is_terminal().then(|| trade.filled_at.unwrap_or_else(Utc::now)),
        transaction_hash: trade.transaction_hash.clone(),
        error_msg: trade.error_msg.clone(),
    };
    store.insert_transaction(&record)
}
```

Then, inside `run_in_memory`, immediately after `store.insert_trade(&trade)`, add:

```rust
if let Err(e) = persist_transaction_from_trade(&store, &trade) {
    tracing::error!(error = %e, "Failed to persist V2 transaction to SQLite");
}
```

- [ ] **Step 3: Add state-manager test**

In `polybot-core/src/state/mod.rs`, add a test that sends a simulated V2 trade with a transaction:

```rust
#[tokio::test]
async fn run_in_memory_persists_v2_transaction_from_trade() {
    let sqlite_path = std::env::temp_dir().join(format!("polybot-v2-txn-{}.db", uuid::Uuid::new_v4()));
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let metrics = Arc::new(Metrics::new());
    let positions = Arc::new(Mutex::new(positions::PositionManager::new()));
    let market_prices = Arc::new(RwLock::new(HashMap::new()));
    let config = AppConfig::default();

    let mut trade = test_trade("market-v2", polybot_common::types::Side::Yes, polybot_common::types::Category::Politics);
    trade.transaction_id = Some("sim-txn-1".to_string());
    trade.relayer_state = Some(polybot_common::types::TransactionState::Success);
    trade.transaction_hash = Some("0xsim".to_string());

    tx.send(trade).await.unwrap();
    drop(tx);

    run_in_memory(
        rx,
        metrics,
        positions,
        market_prices,
        Some(sqlite_path.to_str().unwrap()),
        &config,
    )
    .await
    .unwrap();

    let store = sqlite::SqliteStore::open(&sqlite_path).unwrap();
    let txns = store.latest_transactions(10).unwrap();
    assert_eq!(txns.len(), 1);
    assert_eq!(txns[0].transaction_id, "sim-txn-1");
    assert_eq!(txns[0].state, polybot_common::types::TransactionState::Success);
}
```

- [ ] **Step 4: Run state persistence tests**

Run:

```powershell
cargo test -p polybot-core latest_transactions
cargo test -p polybot-core run_in_memory_persists_v2_transaction_from_trade
```

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```powershell
git add -- polybot-core/src/state/mod.rs polybot-core/src/state/sqlite.rs
git commit -m "feat: persist v2 simulated transactions"
```

---

### Task 9: Route Simulation Execution Through V2 Simulated Lifecycle

**Files:**
- Modify: `polybot-core/src/execution/mod.rs`
- Modify: `polybot-core/src/execution/order_builder.rs`
- Test: `polybot-core/src/execution/mod.rs`

- [ ] **Step 1: Add a helper to make the simulation branch small**

In `polybot-core/src/execution/mod.rs`, add this helper above `run_execution_engine`:

```rust
fn simulation_transaction_id(signal_id: &str) -> String {
    format!("sim-{}", signal_id)
}
```

Add this test at the bottom of the existing test module:

```rust
#[test]
fn simulation_transaction_id_is_stable_for_signal() {
    assert_eq!(super::simulation_transaction_id("abc"), "sim-abc");
}
```

Run:

```powershell
cargo test -p polybot-core simulation_transaction_id_is_stable_for_signal
```

Expected: PASS.

- [ ] **Step 2: Replace immediate simulation trade creation**

In `polybot-core/src/execution/mod.rs`, in the `ExecutionMode::Simulation` arm, replace:

```rust
let trade = order_builder::create_simulated_trade(&decision, &order);
```

with:

```rust
let outcome = v2_flow::simulate_with_relayer(
    &order,
    v2_sim::SimulatedRelayer::success(simulation_transaction_id(&decision.signal_id)),
)?;
let trade = outcome.trade;
metrics.broadcast_event("relayer_update", serde_json::json!({
    "transaction_id": outcome.transaction.transaction_id,
    "state": outcome.transaction.state.as_sqlite_str(),
    "mode": "simulation",
}));
```

Keep the existing state-channel send logic unchanged so the state manager persists the trade and transaction.

- [ ] **Step 3: Run focused execution tests**

Run:

```powershell
cargo test -p polybot-core simulation_mode_uses_fully_offline_transport
cargo test -p polybot-core simulation_transaction_id_is_stable_for_signal
cargo test -p polybot-core simulated_v2_success_creates_filled_trade_and_transaction
```

Expected: PASS.

- [ ] **Step 4: Run workspace check**

Run:

```powershell
cargo check --workspace
```

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```powershell
git add -- polybot-core/src/execution/mod.rs
git commit -m "feat: route simulation through v2 lifecycle"
```

---

### Task 10: Expose V2 State in Health and Transactions API

**Files:**
- Modify: `polybot-core/src/health.rs`
- Test: `polybot-core/src/health.rs`

- [ ] **Step 1: Add V2 fields to health response**

In `polybot-core/src/health.rs`, add these fields to `HealthResponse` after `balance_usd`:

```rust
pub virtual_pusd: String,
pub reserved_pusd: String,
pub fees_paid: String,
pub rebates_earned: String,
pub live_disabled_reason: Option<String>,
```

In `health_check`, add these assignments inside `HealthResponse`:

```rust
virtual_pusd: format!("{:.2}", metrics.virtual_pusd()),
reserved_pusd: format!("{:.2}", metrics.reserved_pusd()),
fees_paid: format!("{:.2}", metrics.fees_paid()),
rebates_earned: format!("{:.2}", metrics.rebates_earned()),
live_disabled_reason: state
    .simulation_mode
    .then(|| "simulation-complete milestone keeps live submission disabled".to_string()),
```

- [ ] **Step 2: Add transactions handler**

In `polybot-core/src/health.rs`, add:

```rust
#[derive(Deserialize)]
pub struct TransactionsQuery {
    pub limit: Option<usize>,
}

pub async fn transactions_handler(
    State(state): State<Arc<HealthState>>,
    Query(query): Query<TransactionsQuery>,
) -> Json<Vec<polybot_common::types::TransactionRecord>> {
    let limit = query.limit.unwrap_or(20);
    match SqliteStore::open(std::path::Path::new(&state.sqlite_path)) {
        Ok(store) => Json(store.latest_transactions(limit).unwrap_or_default()),
        Err(_) => Json(Vec::new()),
    }
}
```

Register the route in `create_health_router`:

```rust
.route("/transactions", get(transactions_handler))
```

- [ ] **Step 3: Add health test assertions**

In the existing `health_check_includes_balance_and_drawdown_fields` test, add:

```rust
assert_eq!(response.virtual_pusd, "0.00");
assert_eq!(response.reserved_pusd, "0.00");
assert_eq!(response.fees_paid, "0.00");
assert_eq!(response.rebates_earned, "0.00");
assert!(response.live_disabled_reason.is_some());
```

In `health_router_exposes_executions_and_control_routes`, add:

```rust
assert!(dbg.contains("/transactions"));
```

- [ ] **Step 4: Run health tests**

Run:

```powershell
cargo test -p polybot-core health_check_includes_balance_and_drawdown_fields
cargo test -p polybot-core health_router_exposes_executions_and_control_routes
```

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```powershell
git add -- polybot-core/src/health.rs
git commit -m "feat: expose v2 simulation health"
```

---

### Task 11: Add Dashboard V2 Readouts

**Files:**
- Modify: `polybot-dashboard/src/data.rs`
- Modify: `polybot-dashboard/src/app.rs`
- Modify: `polybot-dashboard/Trunk.toml`
- Test: `polybot-dashboard/src/data.rs`

- [ ] **Step 1: Add dashboard DTO fields**

In `polybot-dashboard/src/data.rs`, add these fields to `HealthData`:

```rust
pub virtual_pusd: String,
pub reserved_pusd: String,
pub fees_paid: String,
pub rebates_earned: String,
pub live_disabled_reason: Option<String>,
```

Add this DTO:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionData {
    pub transaction_id: String,
    pub trade_id: Option<String>,
    pub kind: String,
    pub state: String,
    pub submitted_at: String,
    pub confirmed_at: Option<String>,
    pub transaction_hash: Option<String>,
    pub error_msg: Option<String>,
}
```

Add this fetcher:

```rust
pub async fn fetch_transactions(limit: usize) -> Result<Vec<TransactionData>, String> {
    let url = format!("{}?limit={}", api_path("/transactions"), limit);
    gloo_net::http::Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("Transactions fetch error: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Transactions parse error: {}", e))
}
```

- [ ] **Step 2: Add Trunk proxy**

In `polybot-dashboard/Trunk.toml`, add:

```toml
[[proxy]]
rewrite = "/transactions"
backend = "http://localhost:8080/transactions"
```

- [ ] **Step 3: Render V2 cards and relayer queue**

In `polybot-dashboard/src/app.rs`, create a resource near the other resources:

```rust
let transactions_res = Resource::new(
    move || refresh.get(),
    async move { data::fetch_transactions(10).await.unwrap_or_default() }
);
let transactions_sig = Signal::derive(move || transactions_res.get().as_deref().cloned().unwrap_or_default());
```

Add a card near the stats grid:

```rust
<div class="stat-card">
    <div class="card-label">"Virtual pUSD"</div>
    <div class="card-value">{move || health.get().map(|h| format!("${}", h.virtual_pusd)).unwrap_or_else(|| "-".into())}</div>
    <div class="card-subtitle">{move || health.get().map(|h| format!("reserved ${}", h.reserved_pusd)).unwrap_or_else(|| "reserved -".into())}</div>
</div>
```

Add a compact relayer queue table in the dashboard view:

```rust
<div class="panel-card">
    <div class="panel-header">
        <span class="card-title">"Relayer Queue"</span>
    </div>
    <div class="table-wrap">
        <table>
            <thead>
                <tr>
                    <th>"Transaction"</th>
                    <th>"State"</th>
                    <th>"Hash"</th>
                </tr>
            </thead>
            <tbody>
                <For
                    each=move || transactions_sig.get()
                    key=|txn| txn.transaction_id.clone()
                    children=move |txn| view! {
                        <tr>
                            <td>{txn.transaction_id}</td>
                            <td>{txn.state}</td>
                            <td>{txn.transaction_hash.unwrap_or_else(|| "-".into())}</td>
                        </tr>
                    }
                />
            </tbody>
        </table>
    </div>
</div>
```

- [ ] **Step 4: Build dashboard**

Run:

```powershell
cd polybot-dashboard
trunk build --release
```

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```powershell
git add -- polybot-dashboard/src/data.rs polybot-dashboard/src/app.rs polybot-dashboard/Trunk.toml
git commit -m "feat: show v2 simulation state in dashboard"
```

---

### Task 12: Add Telegram V2 Status Readout

**Files:**
- Modify: `polybot-core/src/telegram_bot/commands.rs`
- Test: `polybot-core/src/telegram_bot/commands.rs`

- [ ] **Step 1: Add formatting helper**

In `polybot-core/src/telegram_bot/commands.rs`, add this helper near `fallback_positions_message`:

```rust
fn format_v2_status(metrics: &Metrics, live_disabled_reason: Option<&str>) -> String {
    let reason = live_disabled_reason
        .map(|value| format!("\nLive gate: {}", value))
        .unwrap_or_default();
    format!(
        "V2 Simulation\npUSD: ${:.2}\nReserved: ${:.2}\nFees: ${:.2}\nRebates: ${:.2}{}",
        metrics.virtual_pusd(),
        metrics.reserved_pusd(),
        metrics.fees_paid(),
        metrics.rebates_earned(),
        reason
    )
}
```

Add this test:

```rust
#[test]
fn v2_status_message_includes_balances_and_gate() {
    let metrics = Metrics::new();
    metrics.update_v2_accounting(
        rust_decimal::Decimal::new(10000, 2),
        rust_decimal::Decimal::new(2500, 2),
        rust_decimal::Decimal::new(10, 2),
        rust_decimal::Decimal::new(5, 2),
    );
    let msg = format_v2_status(&metrics, Some("simulation only"));
    assert!(msg.contains("pUSD: $100.00"));
    assert!(msg.contains("Reserved: $25.00"));
    assert!(msg.contains("Live gate: simulation only"));
}
```

Run:

```powershell
cargo test -p polybot-core v2_status_message_includes_balances_and_gate
```

Expected: PASS.

- [ ] **Step 2: Include V2 status in `/status`**

In the `Command::Status` branch, before `bot.send_message`, add:

```rust
let v2_status = format_v2_status(
    &metrics,
    config
        .system
        .simulation
        .then_some("simulation-complete milestone keeps live submission disabled"),
);
```

Then change the status format string from:

```rust
"SuperFast PolyBot v3\nMode: {}\nStatus: {}\nUptime: {}\nPositions: {}\nSignals: {}\nFollowed wallets: {}\nWS: {}\nRPC: {}{}"
```

to:

```rust
"SuperFast PolyBot v3.2\nMode: {}\nStatus: {}\nUptime: {}\nPositions: {}\nSignals: {}\nFollowed wallets: {}\nWS: {}\nRPC: {}\n{}{}"
```

and pass `v2_status` before `pending_mode`.

- [ ] **Step 3: Run Telegram tests**

Run:

```powershell
cargo test -p polybot-core telegram_bot::commands
```

Expected: PASS.

- [ ] **Step 4: Commit**

Run:

```powershell
git add -- polybot-core/src/telegram_bot/commands.rs
git commit -m "feat: add v2 simulation telegram status"
```

---

### Task 13: Add Live-Mode Safety Gate

**Files:**
- Modify: `polybot-core/src/setup.rs`
- Test: `polybot-core/src/setup.rs`

- [ ] **Step 1: Add gate helper**

In `polybot-core/src/setup.rs`, add this helper:

```rust
fn live_v2_enabled() -> bool {
    std::env::var("POLYBOT_ENABLE_LIVE_V2")
        .map(|value| value == "true" || value == "1")
        .unwrap_or(false)
}
```

Add this test module content under the existing test module:

```rust
#[test]
fn live_v2_gate_defaults_to_disabled() {
    std::env::remove_var("POLYBOT_ENABLE_LIVE_V2");
    assert!(!super::live_v2_enabled());
}

#[test]
fn live_v2_gate_accepts_true() {
    std::env::set_var("POLYBOT_ENABLE_LIVE_V2", "true");
    assert!(super::live_v2_enabled());
    std::env::remove_var("POLYBOT_ENABLE_LIVE_V2");
}
```

Run:

```powershell
cargo test -p polybot-core live_v2_gate
```

Expected: PASS.

- [ ] **Step 2: Block live preflight unless explicitly enabled**

In `run_startup_preflight`, immediately after the simulation-mode early return, add:

```rust
if matches!(config.system.execution_mode, ExecutionMode::Live) && !live_v2_enabled() {
    return Err(PolybotError::Config(
        "Live CLOB V2 submission is disabled for the simulation-complete milestone. Set POLYBOT_ENABLE_LIVE_V2=true only after V2 endpoint verification, pUSD wrap/approve/redeem support, dashboard control auth, and full workspace tests are green.".to_string(),
    ));
}
```

- [ ] **Step 3: Run setup tests**

Run:

```powershell
cargo test -p polybot-core setup
```

Expected: PASS.

- [ ] **Step 4: Commit**

Run:

```powershell
git add -- polybot-core/src/setup.rs
git commit -m "feat: gate live v2 submission"
```

---

### Task 14: Document Simulation-Complete Status

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/V3_2_MIGRATION_STATUS.md`

- [ ] **Step 1: Add README status block**

In `README.md`, near the top after the overview paragraph, add:

```markdown
## V3.2 Status

The current V3.2 target is simulation-complete CLOB V2. Simulation exercises the V2-shaped order, relayer transaction lifecycle, virtual pUSD accounting, fee-aware routing, and operator visibility without submitting live orders.

Live CLOB V2 submission remains blocked until endpoint response shapes, pUSD wrap/approve/redeem, dashboard control authentication, and full verification are complete.
```

- [ ] **Step 2: Update migration status**

In `docs/superpowers/V3_2_MIGRATION_STATUS.md`, add a section after the phase table:

```markdown
## Current Milestone

The active milestone is `v3.2 simulation-complete`.

Scope:
- restore green workspace tests
- run simulation through a V2-shaped relayer transaction lifecycle
- add virtual pUSD accounting
- add fee-aware maker/FOK routing
- expose V2 state in dashboard and Telegram
- keep live submission blocked behind an explicit safety gate
```

- [ ] **Step 3: Commit docs**

Run:

```powershell
git add -- README.md docs/superpowers/V3_2_MIGRATION_STATUS.md
git commit -m "docs: describe v3.2 simulation-complete milestone"
```

---

### Task 15: Final Verification

**Files:**
- No code edits unless verification reveals a specific failure.

- [ ] **Step 1: Run workspace tests**

Run:

```powershell
cargo test --workspace
```

Expected: PASS.

- [ ] **Step 2: Run clippy**

Run:

```powershell
cargo clippy --workspace -- -D warnings
```

Expected: PASS.

- [ ] **Step 3: Run workspace check**

Run:

```powershell
cargo check --workspace
```

Expected: PASS.

- [ ] **Step 4: Run dashboard production build**

Run:

```powershell
cd polybot-dashboard
trunk build --release
```

Expected: PASS.

- [ ] **Step 5: Run simulation setup check**

Run:

```powershell
cargo run -p polybot-core -- --setup-check
```

Expected: PASS in simulation mode with a log summary containing `simulation_preflight=true`.

- [ ] **Step 6: Commit final verification note if docs changed**

If verification results are added to `docs/superpowers/V3_2_MIGRATION_STATUS.md`, commit:

```powershell
git add -- docs/superpowers/V3_2_MIGRATION_STATUS.md
git commit -m "docs: record v3.2 simulation verification"
```

If no files changed, do not create an empty commit.
