# SuperFast PolyBot v3.2 — CLOB V2 Edition

**Project Name:** SuperFast PolyBot v3.2
**Edition:** CLOB V2 / Solo Developer / Windows Native / Gasless
**Date:** April 24, 2026
**Status:** Final — Pre-Launch (V2 Go-Live: April 28, 2026)
**Target:** Windows Native + Sustainable Copy Trading on Polymarket CLOB V2

---

## 1. Executive Summary

SuperFast PolyBot v3.2 is a **self-hosted, gasless copy-trading system** built for **Polymarket CLOB V2**, launching April 28, 2026.

This PRD is a ground-up rewrite for the V2 protocol. V2 changes everything: the collateral token is now **pUSD** (not USDC.e), builder authentication uses a single **bytes32 `builderCode`** (not API key triples), orders are signed against **EIP-712 domain version "2"** with a new struct, fees are **match-time** (not order-time), and all on-chain transactions are **gasless** via the Relayer Client.

This bot does not hold POL for gas. It wraps USDC.e into pUSD via the Collateral Onramp, signs V2 orders with its builder code, and submits them gaslessly through Polymarket's relayer.

### Key Design Decisions

- **CLOB V2 native:** Built exclusively for the V2 protocol. No V1 backward compatibility.
- **Gasless execution:** All on-chain actions (wrap, approve, trade, redeem) go through the Relayer Client.
- **Maker-first strategy:** Default to GTC (maker) orders. Taker (FOK) only when the market's `feeSchedule.takerFee` is below threshold.
- **Match-time fee awareness:** Fees are read from market metadata (`feeSchedule`) at signal time, not calculated locally.
- **pUSD collateral:** Bot wraps USDC.e → pUSD on startup. All trading balances are tracked in pUSD.
- **Single builder code:** One `BUILDER_CODE` (bytes32) replaces the old API key + secret + passphrase.
- **Async relayer flow:** Orders return a `transactionID` immediately; the bot polls `GET /transaction` for on-chain confirmation.
- **Realistic solo scope:** SQLite only, no Redis, no Docker. Telegram control + lightweight web dashboard.
- **Dual-source signals:** Data API polling (2s) + WebSocket market channel (sub-100ms) with tx-hash dedup.

---

## 2. CLOB V2 Changes (April 28, 2026 Go-Live)

### 2.1 What Changed from V1 → V2

| Area | V1 | V2 | Bot Impact |
|---|---|---|---|
| **SDK** | `rs-clob-client` | `rs-clob-client-v2` | Must upgrade crate |
| **Collateral** | USDC.e | **pUSD** (Polymarket USD) | Wrap USDC.e → pUSD before trading |
| **Order struct** | `feeRateBps`, `nonce`, `taker` included | **Removed** all three; adds `timestamp` (ms), `metadata`, `builder` | Order signing logic rewritten |
| **Builder auth** | API key + secret + passphrase + HMAC headers | Single `builderCode` (bytes32) per order | `.env` simplified; `builder-signing-sdk` deleted |
| **EIP-712 domain** | Version `"1"` | Version `"2"` for Exchange orders | Signing breaks without update |
| **Fee logic** | Embedded in order at creation (`feeRateBps`) | Set at **match time** via `feeSchedule` object | Bot reads fees from market metadata |
| **Relayer submit** | Returns `transactionHash` synchronously | Returns `{transactionID, state: "STATE_NEW"}` asynchronously | Must poll `GET /transaction` for confirmation |
| **Pagination** | Offset-based `GET /markets` | Cursor-based `GET /markets/keyset` | More efficient market discovery |
| **Test endpoint** | N/A | `https://clob-v2.polymarket.com` | Pre-launch testing available |
| **Gas** | User pays POL gas | **Relayer pays gas** (gasless) | Bot holds zero POL |
| **Auto-redeem** | Manual or custom | **Native via relayer** with Builder API | Bot auto-redeems resolved positions |

### 2.2 pUSD (Polymarket USD)

pUSD is the new settlement token for CLOB V2. It is minted by wrapping USDC.e through the **Collateral Onramp** contract.

**Flow:**
```
User USDC.e → Collateral Onramp → pUSD (1:1)
```

- pUSD is used for all trading, settlement, and redemption.
- The bot must wrap its USDC.e balance into pUSD on startup (or check existing pUSD balance).
- When a market resolves, the bot redeems pUSD back to USDC.e (or keeps it in pUSD for the next trade).

### 2.3 Builder Code Authentication

V2 eliminates the API key triple. Instead:

1. Go to Polymarket Settings → Builder Profile
2. Generate a **Builder Code** (32-byte hex string, e.g., `0xabc123...`)
3. Include this `builderCode` in **every order** you sign
4. The relayer recognizes your builder code and attributes volume/rebates to your profile

**No HMAC headers. No API secret. No passphrase.** Just the builder code in the order struct.

### 2.4 Match-Time Fees

V2 removes `feeRateBps` from the order. Instead, each market has a `feeSchedule` object:

```json
{
  "feeSchedule": {
    "takerFee": 12500,    // 125 bps (1.25%) for takers
    "makerFee": 0,        // 0 bps for makers
    "rebate": 2500        // 25 bps rebate to makers when taken
  }
}
```

- **Taker fee** is charged at match time based on the market's `feeSchedule`.
- **Maker fee** is always 0.
- **Maker rebate** is credited when resting liquidity is taken.
- The bot reads `feeSchedule` from `GET /markets` or `GET /markets/{condition_id}` before deciding GTC vs FOK.

### 2.5 Async Relayer Submission

When the bot submits an order to the relayer:

```json
// POST /order (via relayer)
{
  "transactionID": "txn_abc123...",
  "state": "STATE_NEW"
}
```

The bot must then poll:
```
GET /transaction?transactionID=txn_abc123...
```

States:
- `STATE_NEW` — Just submitted
- `STATE_PENDING` — In relayer queue
- `STATE_SUBMITTED` — On-chain tx sent
- `STATE_SUCCESS` — Mined and confirmed
- `STATE_FAILED` — Reverted or error

The bot tracks `transactionID` in SQLite and polls until terminal state (`SUCCESS` or `FAILED`).

### 2.6 Gasless Everything

Via the Relayer Client + Builder API:

| Action | V1 (Self) | V2 (Relayer) |
|---|---|---|
| Wrap USDC.e → pUSD | Self-signed tx + POL gas | **Gasless via relayer** |
| Approve pUSD spender | Self-signed tx + POL gas | **Gasless via relayer** |
| Place CLOB order | Self-signed tx + POL gas | **Gasless via relayer** |
| Cancel order | Self-signed tx + POL gas | **Gasless via relayer** |
| Redeem resolved position | Self-signed tx + POL gas | **Gasless via relayer** |
| Transfer pUSD | Self-signed tx + POL gas | **Gasless via relayer** |

The bot holds **zero POL**. All gas is paid by Polymarket's relayer.

---

## 3. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         SuperFast PolyBot v3.2                              │
│                              CLOB V2 Edition                                │
│                                                                             │
│  ┌─────────────────────┐      ┌─────────────────────────────────────────┐   │
│  │   Signal Ingestion  │      │         Risk & Fee Engine               │   │
│  │                     │      │                                         │   │
│  │  WS Market Channel  ├─────►│  base_size = target_size × multiplier   │   │
│  │  Data API (2s poll) │      │  fee_check = feeSchedule.takerFee       │   │
│  │  Tx-hash dedup      │      │  drawdown_guard                         │   │
│  └──────────┬──────────┘      └──────────────────┬──────────────────────┘   │
│             │                                    │                           │
│             ▼                                    ▼                           │
│  ┌─────────────────────┐      ┌─────────────────────────────────────────┐   │
│  │   Wallet Tracker    │      │      CLOB V2 Execution Engine           │   │
│  │                     │      │                                         │   │
│  │  Multi-target       │      │  Fetch price → read feeSchedule         │   │
│  │  Real metrics only  │      │  → build V2 order (timestamp, builder)  │   │
│  │  Category filter    │      │  → sign EIP-712 v2 → relayer submit     │   │
│  └─────────────────────┘      │  → poll transactionID → confirm fill    │   │
│                               └──────────────────┬──────────────────────┘   │
│                                                  │                           │
│             ┌────────────────────────────────────┤                           │
│             ▼                                    ▼                           │
│  ┌─────────────────────┐      ┌─────────────────────────────────────────┐   │
│  │  Lightweight Web UI │      │         SQLite (polybot.db)             │   │
│  │                     │      │                                         │   │
│  │  Live PnL, Positions│      │  signals / trades / positions           │   │
│  │  System Health      │      │  targets / daily_stats / config         │   │
│  │  Relayer Queue      │      │  transactions (async tracking)          │   │
│  └─────────────────────┘      └─────────────────────────────────────────┘   │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │  Telegram Bot  (chat ID whitelist + 2-step confirmation)             │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │  Relayer Client  (gasless wrap, approve, trade, redeem, transfer)    │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────────┘
```

| Component | Technology |
|---|---|
| Language | Rust 1.75+ + Tokio |
| Storage | SQLite (`polybot.db`) via `sqlx` |
| Signal Ingestion | Polymarket Data API + WebSocket market channel |
| Execution | Official `rs-clob-client-v2` |
| Price Feed | CLOB V2 WebSocket `wss://ws-subscriptions-clob.polymarket.com/ws/market` |
| Relayer | Polymarket Relayer Client (gasless) |
| Collateral | pUSD via Collateral Onramp (wrap USDC.e → pUSD) |
| UI | Lightweight Axum server + vanilla HTML/JS |
| Control | Telegram Bot (chat ID whitelist) |
| Deployment | Native `cargo run --release` on Windows |

---

## 4. Polymarket CLOB V2 API Layer

### 4.1 APIs

| API | Base URL | Auth | Purpose |
|---|---|---|---|
| Gamma API | `https://gamma-api.polymarket.com` | No | Market discovery, metadata, prices, condition IDs |
| CLOB V2 API | `https://clob-v2.polymarket.com` | Builder Code + Relayer | Order placement, cancellation, orderbook, balances |
| Data API | `https://data-api.polymarket.com` | No | Wallet activity, positions, historical trades |
| Relayer API | `https://relayer-v2.polymarket.com` | Relayer API Key | Gasless transaction submission |

**Note:** On April 28, 2026, `clob-v2.polymarket.com` becomes the production endpoint. Until then, use it for testing.

### 4.2 Rate Limits (V2 — Verified)

Polymarket uses Cloudflare throttling. Excess requests are queued and delayed; `429` only appears if the queue is saturated.

| Endpoint | Burst Limit | Sustained Limit |
|---|---|---|
| General REST | 15,000 / 10 s | — |
| CLOB V2 General | 9,000 / 10 s | — |
| `POST /order` (via relayer) | 3,500 / 10 s | 36,000 / 10 min |
| `DELETE /order` | 3,000 / 10 s | 30,000 / 10 min |
| `POST /orders` (batch) | 1,000 / 10 s | 15,000 / 10 min |
| Gamma API | 4,000 / 10 s | — |
| Data API | 1,000 / 10 s | — |
| Relayer API | 2,000 / 10 s | — |
| WebSocket | **500 instruments / connection** | 5 concurrent connections / IP |

**Retry strategy:** Exponential backoff with jitter. Start at 1s, double each retry, cap at 60s. Add ±20% random jitter.

### 4.3 WebSocket Endpoints (V2)

| Channel | URL | Use |
|---|---|---|
| Market channel | `wss://ws-subscriptions-clob.polymarket.com/ws/market` | Real-time orderbook, price updates, trades |
| User channel | `wss://ws-subscriptions-clob.polymarket.com/ws/user` | Fill confirmations for our own orders |

Subscribe format (V2):
```json
{
  "type": "market",
  "assets_id": ["<token_id_1>", "<token_id_2>", "..."]
}
```

**Note:** One connection can subscribe to up to **500 token IDs**.

### 4.4 Cursor Pagination (V2)

V2 replaces offset pagination with cursor-based keyset pagination for efficiency:

```
GET /markets/keyset?cursor=<cursor>&limit=100
GET /events/keyset?cursor=<cursor>&limit=100
```

Response includes `next_cursor` for subsequent requests. The bot uses this for market discovery and wallet activity scanning.

---

## 5. Authentication & Wallet Setup (V2)

### 5.1 Wallet Type

**Recommended:** EOA (`signature_type = 0`). Simplest setup; private key in `.env`.

The V2 SDK auto-derives the Polymarket proxy wallet address via `CREATE2` from your EOA.

### 5.2 Builder Code

The builder code is the **only** authentication credential needed for order signing in V2.

**Setup:**
1. Log into Polymarket → Settings → Builder Profile
2. Generate a new Builder Code (32-byte hex string)
3. Copy it to `.env` as `BUILDER_CODE`
4. Include it in every order's `builder` field

```rust
// In every V2 order
builder: "0x<your-32-byte-builder-code>",
```

**No API key. No secret. No passphrase. No HMAC.**

### 5.3 Relayer API Key (Gasless)

For gasless transaction submission, obtain a Relayer API Key from Polymarket.

**Headers required for every relayer request:**
```
RELAYER_API_KEY: <your-relayer-api-key>
RELAYER_API_KEY_ADDRESS: <your-eoa-address>
```

### 5.4 First-Run Setup (V2)

On first run, the bot performs these **gasless** actions via the relayer:

1. **Wrap USDC.e → pUSD** via Collateral Onramp
   ```
   POST /relayer/wrap
   {
     "amount": <usdc_amount>,
     "token": "USDC.e"
   }
   ```

2. **Approve pUSD spender** (CTF Exchange contract) — gasless
   ```
   POST /relayer/approve
   {
     "token": "pUSD",
     "spender": "<ctf_exchange_address>"
   }
   ```

3. **Deploy proxy wallet** (if not already deployed) — gasless
   ```
   POST /relayer/deploy
   ```

All three are one-time setup steps. The bot detects missing state and triggers them automatically.

### 5.5 V2 SDK Initialization

```rust
use polymarket_client_v2::Client;

let client = Client::new("https://clob-v2.polymarket.com", Config::default())?
    .with_relayer("https://relayer-v2.polymarket.com", relayer_api_key, relayer_api_key_address)
    .with_builder_code("0x<builder_code>")
    .authenticate(&signer)
    .await?;
```

---

## 6. Environment Variables (Complete `.env` for V2)

```env
# === Core Identity ===
POLYMARKET_PRIVATE_KEY=0xYOUR_EOA_PRIVATE_KEY

# === Polymarket Endpoints (V2) ===
CLOB_API_URL=https://clob-v2.polymarket.com
GAMMA_API_URL=https://gamma-api.polymarket.com
DATA_API_URL=https://data-api.polymarket.com
WS_CLOB_URL=wss://ws-subscriptions-clob.polymarket.com/ws/market
WS_USER_URL=wss://ws-subscriptions-clob.polymarket.com/ws/user

# === Relayer (Gasless) ===
RELAYER_URL=https://relayer-v2.polymarket.com
RELAYER_API_KEY=YOUR_RELAYER_API_KEY
RELAYER_API_KEY_ADDRESS=0xYOUR_EOA_ADDRESS

# === Builder (V2) ===
BUILDER_CODE=0xYOUR_32_BYTE_BUILDER_CODE

# === Collateral ===
COLLATERAL_TOKEN=pUSD
COLLATERAL_ONRAMP_ADDRESS=0x_COLLATERAL_ONRAMP_CONTRACT_ADDRESS
USDC_E_ADDRESS=0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174

# === Signal Ingestion (Target Wallets) ===
TARGET_WALLETS=0xWALLET1,0xWALLET2,0xWALLET3
POLL_INTERVAL_MS=2000
USE_WEBSOCKET=true
SIGNAL_MAX_AGE_SECS=30

# === Order Execution ===
DEFAULT_ORDER_TYPE=GTC              # GTC (maker, zero fee) or FOK (taker, fee applies)
FOK_MAX_FEE_BPS=50                  # Only use FOK if feeSchedule.takerFee ≤ 50 bps
SLIPPAGE_TOLERANCE=0.02             # 2% max price deviation
PRICE_BUFFER=0.005                  # 0.5% buffer for GTC pricing
POSITION_MULTIPLIER=0.1             # Copy at 10% of target's size

# === Risk Limits ===
MAX_TRADE_SIZE_USDC=150.0
MIN_TRADE_SIZE_USDC=5.0
MAX_CONCURRENT_POSITIONS=20
MAX_DAILY_LOSS_PCT=5.0
MAX_DAILY_VOLUME_USDC=0             # 0 = disabled
MAX_CONSECUTIVE_LOSSES=5
LOSS_COOLDOWN_SECS=3600
MIN_USDC_BALANCE=20.0

# === Category Caps ===
MAX_POSITION_POLITICS_USDC=250.0
MAX_POSITION_CRYPTO_USDC=150.0
MAX_POSITION_SPORTS_USDC=200.0
MAX_POSITION_OTHER_USDC=100.0

# === Mode ===
SIMULATION_MODE=true
LOG_LEVEL=info

# === REMOVED FROM V1 (no longer needed) ===
# BUILDER_API_KEY, BUILDER_API_SECRET, BUILDER_API_PASSPHRASE
# POLYGON_RPC_URL, POLYGON_CHAIN_ID, MIN_PRIORITY_FEE_GWEI, MIN_MAX_FEE_GWEI
# (All gas is handled by the relayer)
```

---

## 7. Signal Ingestion Architecture (Dual-Source)

### 7.1 Primary Source — Data API Polling

```
Loop every POLL_INTERVAL_MS (default 2000 ms):
  GET https://data-api.polymarket.com/activity
    ?user=<target_wallet>
    &type=TRADE
    &limit=50
    &sortBy=TIMESTAMP
    &sortDirection=DESC
    &start=<last_seen_timestamp>

  For each trade returned:
    → Hash-dedup: skip if tx_hash already in signals table
    → Age filter: skip if timestamp < (now - SIGNAL_MAX_AGE_SECS)
    → Enqueue to signal_channel
```

### 7.2 Secondary Source — WebSocket Market Channel

When `USE_WEBSOCKET=true`, subscribe to the market channel for all tokens held by target wallets.

```
Connect: wss://ws-subscriptions-clob.polymarket.com/ws/market
Subscribe to all token_ids the targets hold (batch up to 500 per connection)
On trade event → hash-dedup → enqueue to signal_channel
```

### 7.3 Deduplication

Both sources feed the same `signal_channel`. A `HashSet<TxHash>` in memory (with SQLite persistence for crash recovery) ensures each trade is processed exactly once.

### 7.4 Signal Staleness Guard

A signal is **rejected** if:
- `timestamp` is older than `SIGNAL_MAX_AGE_SECS` (default 30s)
- Market `end_date` has passed (resolved)
- Market `redeemable = true` (settled on-chain)
- Same `(target_wallet, token_id, side)` is already an open position (anti-duplication)

---

## 8. Signal JSON Schema (Real Fields Only)

```json
{
  "signal_id": "uuid-v4",
  "source": "websocket" | "polling" | "manual",
  "tx_hash": "0x...",
  "timestamp": "2026-04-24T13:45:22.123Z",
  "target_wallet": "0x...",
  "market_id": "clob-market-id",
  "token_id": "erc1155-token-id",
  "side": "BUY" | "SELL",
  "outcome": "YES" | "NO",
  "target_size_usdc": 500.0,
  "target_price": 0.62,
  "category": "politics" | "crypto" | "sports" | "other",
  "fee_schedule": {
    "takerFee": 12500,
    "makerFee": 0,
    "rebate": 2500
  }
}
```

**Field sources:**
- `tx_hash`, `timestamp`, `size`, `price`, `side`: from Data API `/activity`
- `token_id`, `market_id`: from Data API response
- `outcome`: derived from `token_id` via Gamma API mapping
- `category`: from Gamma API market metadata
- `fee_schedule`: from `GET /markets/{condition_id}` or cached market metadata

**Note:** `fee_rate_bps` is removed. V2 uses match-time fees via `feeSchedule`.

---

## 9. Fee-Aware Order Execution Flow (V2)

```
Step 1 — Signal received from signal_channel
         ↓
Step 2 — Staleness check (age, resolved, duplicate)
         ↓ PASS
Step 3 — Read feeSchedule from market metadata
         taker_fee_bps = feeSchedule.takerFee
         maker_fee_bps = feeSchedule.makerFee
         rebate_bps = feeSchedule.rebate
         ↓
Step 4 — Risk engine: compute final_size
         base = target_size × POSITION_MULTIPLIER
         final = clamp(base, MIN_SIZE, category_cap)
         ↓ final_size >= MIN_TRADE_SIZE_USDC
Step 5 — Fetch current best price (parallel with Step 3-4)
         GET /price?token_id=<id>&side=BUY
         ↓
Step 6 — Slippage check
         |current_price - signal_price| / signal_price <= SLIPPAGE_TOLERANCE
         ↓ PASS
Step 7 — DECISION: Maker vs Taker
         ├─ IF taker_fee_bps <= FOK_MAX_FEE_BPS AND edge_justified:
         │   → Build FOK order (taker, immediate fill)
         │   → price = current_price + PRICE_BUFFER
         │   → builder = BUILDER_CODE
         │   → timestamp = current_time_ms()
         │   → NO feeRateBps (V2 removed it)
         │
         └─ ELSE:
             → Build GTC order (maker, zero fee, possible rebate)
             → price = signal_price ± PRICE_BUFFER (improve by 1 tick)
             → builder = BUILDER_CODE
             → timestamp = current_time_ms()
             → NO feeRateBps
         ↓
Step 8 — Sign order with EIP-712 v2 via rs-clob-client-v2
         Domain: { name: "Polymarket CTF Exchange", version: "2", ... }
         ↓
Step 9 — Submit to Relayer (gasless)
         POST /order (via relayer)
         → Returns { transactionID, state: "STATE_NEW" }
         ↓
Step 10 — Poll transaction state
          Loop: GET /transaction?transactionID=<id>
          Until state ∈ { STATE_SUCCESS, STATE_FAILED }
          ↓
Step 10a — STATE_SUCCESS
           → Fetch transactionHash from response
           → Write trades table (status = FILLED)
           → Update positions table
           → Update daily_stats (volume, fees_paid, rebates_earned, pnl)
           ↓
Step 10b — STATE_FAILED
           → Write trades table (status = FAILED)
           → Log error; optionally retry as GTC
           ↓
Step 10c — Timeout (> 60s in non-terminal state)
           → Mark as PENDING_TIMEOUT
           → Alert via Telegram
```

### 9.1 Order Types (V2)

| Type | Behaviour | Fee | When to Use |
|---|---|---|---|
| **GTC** (Good Till Cancelled) | Rests in book until filled or cancelled | **Zero** + possible rebate | **Default.** Use unless taker fee is very low. |
| **FOK** (Fill or Kill) | Must fill 100% immediately or cancel | `feeSchedule.takerFee` at match time | **Selective.** Only when `takerFee ≤ FOK_MAX_FEE_BPS`. |

### 9.2 V2 Order Struct

```rust
struct Order {
    salt: u64,              // Random nonce
    maker: Address,         // Bot's proxy wallet address
    signer: Address,        // Bot's EOA address
    taker: Address,         // Zero address (open order)
    tokenId: u256,          // ERC-1155 token ID
    makerAmount: u256,      // pUSD amount (in wei)
    takerAmount: u256,      // Token amount (in wei)
    expiration: u256,       // Unix timestamp (0 = no expiry)
    nonce: u256,            // Order nonce
    feeRateBps: u64,        // DEPRECATED in V2 — set to 0
    side: Side,             // BUY or SELL
    signatureType: u8,      // 0 = EOA
    signature: Bytes,       // EIP-712 v2 signature

    // V2 NEW FIELDS:
    timestamp: u64,         // Current time in milliseconds
    metadata: String,       // Optional metadata (JSON string)
    builder: String,        // Builder code (bytes32 hex)
}
```

**Critical:** `feeRateBps` is deprecated but still present in the struct for backward compatibility. Set it to `0`. The actual fee is determined by the market's `feeSchedule` at match time.

### 9.3 EIP-712 V2 Domain

```rust
EIP712Domain {
    name: "Polymarket CTF Exchange",
    version: "2",           // CHANGED from "1"
    chainId: 137,
    verifyingContract: "0x<exchange_contract_address>"
}
```

Signing against version `"1"` will fail in V2.

---

## 10. Risk & Dynamic Sizing Engine

### 10.1 Base Size

```rust
base_size = target_trade_size_usdc × POSITION_MULTIPLIER
```

### 10.2 Performance Multiplier

```rust
performance_multiplier = f(win_rate_30d, avg_trade_size, recent_streak)
```

| Win Rate (30d) | Multiplier |
|---|---|
| < 40% | 0.00 |
| 40–50% | 0.50 |
| 50–55% | 0.75 |
| 55–60% | 1.00 |
| 60–70% | 1.25 |
| > 70% | 1.50 |

| Recent Streak | Adjustment |
|---|---|
| 3+ consecutive losses | × 0.50 |
| 3+ consecutive wins | × 1.00 |

### 10.3 Drawdown Multiplier

| Daily Drawdown | Multiplier |
|---|---|
| 0–5% | 1.00 |
| 5–10% | 0.75 |
| 10–15% | 0.50 |
| 15–20% | 0.25 |
| > 20% | 0.00 (full pause) |

### 10.4 Final Size Formula

```rust
final_size = base_size
           × performance_multiplier
           × drawdown_multiplier

final_size = clamp(final_size, MIN_TRADE_SIZE_USDC, category_max_position)
```

### 10.5 Hard Limits

| Limit | Default | Behaviour |
|---|---|---|
| Daily loss limit | 5% of pUSD balance | Auto-pause |
| Max concurrent positions | 20 | Drop new signals |
| Consecutive loss circuit breaker | 5 | Pause for `LOSS_COOLDOWN_SECS` |
| Min pUSD balance | $20 | Auto-pause; Telegram alert |
| Micro-trade filter | < $1.00 notional | Reject |
| Politics cap | $250/market | |
| Crypto cap | $150/market | |
| Sports cap | $200/market | |
| Other cap | $100/market | |

### 10.6 Anti-Duplication Rule

The bot never enters the same `token_id` from two different target wallets. First signal "owns" the position.

---

## 11. Target Wallet Tracking (Realistic Metrics)

### 11.1 Computable Metrics

| Metric | Source | Calculation |
|---|---|---|
| ROI (30d) | Data API `/activity` | `(end_balance - start_balance) / start_balance` |
| Win Rate | Data API `/activity` | `% closed positions with realized_pnl > 0` |
| Trade Frequency | Data API `/activity` | `count(trades) / 30 days` |
| Avg Trade Size | Data API `/activity` | `mean(size_usdc)` |
| Category Breakdown | Gamma API + activity | `% trades per category` |
| Current Streak | Data API `/activity` | Consecutive wins/losses |

### 11.2 Wallet Scoring Command

Telegram: `/wallet score [address]`

```
Wallet: 0x... (Label: "Whale_A")
ROI 30d: +12.4%
Win Rate: 58.3%
Frequency: 4.2 trades/day
Avg Size: $340
Top Category: Politics (62%)
Current Streak: W2
Status: ACTIVE
```

---

## 12. SQLite Database Schema

### 12.1 `signals`

```sql
CREATE TABLE signals (
    id            TEXT PRIMARY KEY,
    source        TEXT NOT NULL,
    tx_hash       TEXT UNIQUE,
    received_at   TEXT NOT NULL,
    target_wallet TEXT NOT NULL,
    market_id     TEXT NOT NULL,
    token_id      TEXT NOT NULL,
    side          TEXT NOT NULL,
    outcome       TEXT NOT NULL,
    target_price  REAL NOT NULL,
    target_size   REAL NOT NULL,
    taker_fee_bps INTEGER NOT NULL,    -- from feeSchedule.takerFee
    maker_fee_bps INTEGER NOT NULL,    -- from feeSchedule.makerFee
    rebate_bps    INTEGER NOT NULL,    -- from feeSchedule.rebate
    category      TEXT NOT NULL,
    status        TEXT NOT NULL
);
```

### 12.2 `trades`

```sql
CREATE TABLE trades (
    id               TEXT PRIMARY KEY,
    signal_id        TEXT NOT NULL REFERENCES signals(id),
    transaction_id   TEXT,             -- V2: relayer transactionID
    transaction_hash TEXT,             -- V2: on-chain tx hash (from polling)
    placed_at        TEXT NOT NULL,
    filled_at        TEXT,
    market_id        TEXT NOT NULL,
    token_id         TEXT NOT NULL,
    side             TEXT NOT NULL,
    order_type       TEXT NOT NULL,
    requested_size   REAL NOT NULL,
    filled_size      REAL,
    requested_price  REAL NOT NULL,
    fill_price       REAL,
    taker_fee_bps    INTEGER NOT NULL DEFAULT 0,
    fee_paid_usdc    REAL DEFAULT 0,
    rebate_usdc      REAL DEFAULT 0,
    relayer_state    TEXT,             -- STATE_NEW, STATE_PENDING, etc.
    status           TEXT NOT NULL,    -- pending, filled, rejected, failed
    retry_count      INTEGER DEFAULT 0,
    error_msg        TEXT
);
```

### 12.3 `transactions` (V2 New Table)

```sql
CREATE TABLE transactions (
    transaction_id   TEXT PRIMARY KEY,
    trade_id         TEXT REFERENCES trades(id),
    type             TEXT NOT NULL,    -- order, cancel, wrap, approve, redeem
    state            TEXT NOT NULL,    -- STATE_NEW, STATE_PENDING, STATE_SUBMITTED, STATE_SUCCESS, STATE_FAILED
    submitted_at     TEXT NOT NULL,
    confirmed_at     TEXT,
    transaction_hash TEXT,
    error_msg        TEXT
);
```

### 12.4 `positions`

```sql
CREATE TABLE positions (
    token_id        TEXT PRIMARY KEY,
    market_id       TEXT NOT NULL,
    outcome         TEXT NOT NULL,
    category        TEXT NOT NULL,
    size_usdc       REAL NOT NULL,
    avg_entry_price REAL NOT NULL,
    current_price   REAL,
    unrealized_pnl  REAL,
    fees_paid       REAL DEFAULT 0,
    rebates_earned  REAL DEFAULT 0,
    opened_at       TEXT NOT NULL,
    last_updated    TEXT NOT NULL,
    owned_by_wallet TEXT NOT NULL,
    status          TEXT NOT NULL
);
```

### 12.5 `targets`

```sql
CREATE TABLE targets (
    wallet_address  TEXT PRIMARY KEY,
    label           TEXT,
    added_at        TEXT NOT NULL,
    active          INTEGER NOT NULL DEFAULT 1,
    roi_30d         REAL,
    win_rate        REAL,
    trade_frequency REAL,
    avg_trade_size  REAL,
    top_category    TEXT,
    current_streak  INTEGER DEFAULT 0,
    last_scored_at  TEXT,
    categories      TEXT,
    notes           TEXT
);
```

### 12.6 `daily_stats`

```sql
CREATE TABLE daily_stats (
    date              TEXT PRIMARY KEY,
    starting_balance  REAL NOT NULL,
    realized_pnl      REAL DEFAULT 0,
    unrealized_pnl    REAL DEFAULT 0,
    volume_traded     REAL DEFAULT 0,
    fees_paid         REAL DEFAULT 0,
    rebates_earned    REAL DEFAULT 0,
    trades_placed     INTEGER DEFAULT 0,
    trades_filled     INTEGER DEFAULT 0,
    trades_rejected   INTEGER DEFAULT 0,
    drawdown_pct      REAL DEFAULT 0,
    paused_at         TEXT,
    notes             TEXT
);
```

### 12.7 `config`

```sql
CREATE TABLE config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

---

## 13. Error Handling & Resilience

### 13.1 Error Categories

| Error | Trigger | Response |
|---|---|---|
| API 429 | Rate limited | Exponential backoff + jitter |
| API 5xx | Server error | Retry up to 3×, then FAILED |
| FOK not matched | No liquidity | Retry as GTC at improved price |
| Partial fill (GTC) | Maker partial fill | Track with partial fill flag |
| Relayer timeout | >60s in non-terminal state | Mark PENDING_TIMEOUT; alert Telegram |
| Relayer STATE_FAILED | On-chain revert | Log error; retry logic based on type |
| WS disconnect | Network drop | Auto-reconnect; resubscribe all tokens |
| pUSD balance low | < MIN_USDC_BALANCE | Auto-pause; Telegram alert |
| Daily loss limit | Drawdown ≥ 5% | Auto-pause; require `/resume` |
| Wrap failure | Collateral Onramp error | Alert; halt until resolved |

### 13.2 Relayer Polling Logic

```rust
const RELAYER_POLL_INTERVAL_MS: u64 = 2_000;
const RELAYER_MAX_POLL_TIME_SECS: u64 = 120;

async fn poll_relayer_transaction(transaction_id: &str) -> Result<TransactionState, BotError> {
    let start = Instant::now();
    loop {
        let resp = relayer_client
            .get(format!("/transaction?transactionID={}", transaction_id))
            .send().await?;

        let state = resp.json::<TransactionResponse>().await?.state;

        match state {
            TransactionState::Success => return Ok(state),
            TransactionState::Failed => return Ok(state),
            _ => {
                if start.elapsed().as_secs() > RELAYER_MAX_POLL_TIME_SECS {
                    return Err(BotError::RelayerTimeout);
                }
                tokio::time::sleep(Duration::from_millis(RELAYER_POLL_INTERVAL_MS)).await;
            }
        }
    }
}
```

### 13.3 State Reconciliation

Every 30 seconds:
1. Fetch open positions from CLOB V2 API (`GET /positions`)
2. Compare against SQLite `positions` table
3. Resolve mismatches
4. Update `current_price` and `unrealized_pnl` from live orderbook
5. Check for resolved markets → trigger auto-redeem via relayer
6. Log reconciliation timestamp

---

## 14. Functional Requirements

### Must Have

- [ ] Full Simulation Mode with virtual pUSD balance tracking
- [ ] Live Mode toggle (`SIMULATION_MODE=false` required)
- [ ] Dual-source signal ingestion (WebSocket + Data API polling)
- [ ] Target wallet tracker with configurable wallet list
- [ ] Transaction hash deduplication across all sources
- [ ] Signal staleness guard (age, resolution, duplicate position)
- [ ] **Match-time fee awareness:** Read `feeSchedule` from market metadata
- [ ] **V2 order signing:** EIP-712 domain version "2", `timestamp`, `builder` field
- [ ] **Gasless execution:** All trades via Relayer Client
- [ ] **Async transaction tracking:** Poll `GET /transaction` for confirmation
- [ ] **pUSD collateral management:** Wrap USDC.e → pUSD on startup
- [ ] **Auto-redeem:** Resolved positions redeemed gaslessly via relayer
- [ ] Maker-first strategy: GTC default, FOK only when `takerFee ≤ threshold`
- [ ] Fee/rebate tracking in SQLite
- [ ] Risk engine with performance multipliers
- [ ] Anti-duplication rule
- [ ] CLOB V2 order placement with slippage check
- [ ] Automatic retry with exponential backoff + jitter
- [ ] Circuit breaker (consecutive losses)
- [ ] Daily loss limit auto-pause
- [ ] Lightweight web dashboard
- [ ] Telegram bot with whitelisted chat IDs
- [ ] Full SQLite persistence
- [ ] State reconciliation loop (30s)
- [ ] First-run gasless setup (wrap, approve, deploy)

### Nice to Have

- [ ] Target wallet on-demand scoring
- [ ] Stop-loss via WebSocket orderbook monitoring
- [ ] Take-profit auto-exit
- [ ] Order aggregation window
- [ ] pUSD → USDC.e unwrap on shutdown

---

## 15. User Interfaces

### 15.1 Web Dashboard

Runs on `http://localhost:8080` via Axum.

| Panel | Metrics |
|---|---|
| Status Bar | Mode (SIM/LIVE), pUSD balance, daily PnL, fees paid, rebates earned, drawdown % |
| Open Positions | Token, outcome, size, entry, current price, unrealized PnL, fees, rebates |
| Relayer Queue | Pending transactionIDs, states, time in queue |
| System Health | WS status, API latency, SQLite OK, last reconciliation, maker % vs taker % |
| Signal Feed | Last 20 signals with source, wallet, side, size, takerFee, order type chosen |
| Execution Log | Last 20 trades with order type, fill price, fee paid, relayer latency |
| Daily Stats | PnL chart, volume, fee/rebate breakdown, win/loss ratio |

### 15.2 Dashboard Event Stream

`ws://localhost:8081`:

| Event | Payload |
|---|---|
| `signal_received` | Signal ID, wallet, market, side, size, takerFee, chosen_order_type |
| `trade_placed` | Trade ID, order type, price, size, transactionID |
| `trade_filled` | Trade ID, fill price, fee paid, rebate earned, relayer latency |
| `trade_failed` | Trade ID, error, relayer state |
| `position_updated` | Token ID, new price, unrealized PnL |
| `relayer_update` | transactionID, state, time_in_queue |
| `system_alert` | Level, message |
| `daily_stats` | Balance, PnL, fees, rebates, drawdown — every 60s |

### 15.3 Telegram Commands

| Command | Description | Confirm |
|---|---|---|
| `/status` | Mode, pUSD balance, daily PnL, fees, drawdown | No |
| `/positions` | Open positions with PnL | No |
| `/signals` | Last 10 signals | No |
| `/wallets` | Tracked wallets | No |
| `/wallet add [addr]` | Add target | No |
| `/wallet remove [addr]` | Remove target | Yes |
| `/wallet score [addr]` | Score wallet | No |
| `/wrap [amount]` | Wrap USDC.e → pUSD | Yes |
| `/redeem [token_id]` | Redeem resolved position | Yes |
| `/pause` | Pause signals | No |
| `/resume` | Resume | No |
| `/mode sim` | Sim mode | Yes → `/confirm` |
| `/mode live` | Live mode | Yes → `/confirm` |
| `/emergency_stop` | Cancel all + pause | Yes → `/confirm` |
| `/report daily` | Daily report | No |
| `/report weekly` | Weekly report | No |

**Security:** Only `TELEGRAM_ALLOWED_CHAT_IDS` receive responses. `/confirm` required for destructive actions (expires in 60s).

---

## 16. Non-Functional Requirements

| Requirement | Target | Notes |
|---|---|---|
| Signal-to-order latency | < 800ms (taker), uncritical (maker) | Maker orders don't race |
| Relayer confirmation time | < 30s typical | Poll every 2s |
| Reconciliation interval | 30s | Adaptive 10–60s |
| SQLite write latency | < 10ms | WAL mode |
| Dashboard refresh | Real-time via WS | No polling |
| Uptime | Auto-restart on panic | Windows Task Scheduler or NSSM |
| Private key security | Only in `.env`, never logged | `secrecy::SecretString` |
| Windows native | No Redis, no Docker | `cargo run --release` |
| Simulation safety | 100% no-op | Verified by unit tests |
| Rust edition | 2021, stable 1.75+ | |

---

## 17. Implementation Phases (Solo-Dev Realistic)

| Phase | Scope | Duration |
|---|---|---|
| **Phase 1** | Foundation: SQLite schema (incl. `transactions` table), `.env` loader, simulation shell | 2–3 days |
| **Phase 2** | Collateral: pUSD wrap/unwrap via Collateral Onramp, balance tracking | 2–3 days |
| **Phase 3** | Signal ingestion: Data API poller, WebSocket connector, dedup, staleness guard | 3–4 days |
| **Phase 4** | V2 order engine: EIP-712 v2 signing, `builderCode`, `timestamp`, order struct | 3–4 days |
| **Phase 5** | Relayer integration: Gasless submit, async polling, `transactions` tracking | 3–4 days |
| **Phase 6** | Risk engine: Sizing, performance multipliers, hard limits, anti-duplication | 2–3 days |
| **Phase 7** | Fee awareness: Read `feeSchedule`, maker/taker decision, fee/rebate tracking | 2–3 days |
| **Phase 8** | Telegram bot: Whitelist, commands, wrap/redeem commands, 2-step confirmation | 2–3 days |
| **Phase 9** | Web dashboard: Status, positions, relayer queue, signal feed, execution log | 3–4 days |
| **Phase 10** | Reconciliation & auto-redeem: Position sync, PnL, orphan detection, resolved market redemption | 3–4 days |
| **Phase 11** | Live deployment: Flip `SIMULATION_MODE=false`, $100–200, monitor 1 week | Ongoing |

**Total:** 4–5 weeks of focused solo development.

---

## 18. Fee Economics Reference (V2)

### Match-Time Fees

V2 fees are determined by the market's `feeSchedule` at match time, not by the order:

```json
{
  "feeSchedule": {
    "takerFee": 12500,    // 125 bps
    "makerFee": 0,        // 0 bps
    "rebate": 2500        // 25 bps rebate to maker
  }
}
```

**Units:** All values are in **basis points × 100** (i.e., `12500` = 125 bps = 1.25%).

### Fee-Aware Decision Matrix

| feeSchedule.takerFee | Recommended Order Type | Why |
|---|---|---|
| > 100 bps (10000) | **GTC only** | Fee destroys edge |
| 50–100 bps (5000–10000) | **GTC default**, FOK only if urgent | Expensive taker fee |
| 20–50 bps (2000–5000) | **GTC default**, FOK acceptable | Moderate fee |
| 5–20 bps (500–2000) | **FOK acceptable** if time-sensitive | Low fee |
| < 5 bps (< 500) | **FOK preferred** if signal is fast | Negligible fee |

### Maker Rebate

- Makers pay **0 bps**.
- Makers earn `feeSchedule.rebate` when their resting order is taken.
- Rebate is credited in pUSD at match time.

---

## 19. V2 Migration Checklist

If migrating from a V1 bot, verify each item:

- [ ] Upgrade `rs-clob-client` → `rs-clob-client-v2`
- [ ] Change EIP-712 domain version `"1"` → `"2"`
- [ ] Remove `feeRateBps` from order logic (set to 0 in struct)
- [ ] Add `timestamp` (ms) and `builder` (builderCode) to every order
- [ ] Remove API key/secret/passphrase; replace with single `BUILDER_CODE`
- [ ] Add Relayer Client for gasless submission
- [ ] Implement async `transactionID` polling
- [ ] Add pUSD wrap flow via Collateral Onramp
- [ ] Change balance tracking from USDC.e to pUSD
- [ ] Read fees from `feeSchedule` instead of local calculation
- [ ] Add `transactions` table for async relayer state tracking
- [ ] Update pagination from offset to cursor (`/markets/keyset`)
- [ ] Add auto-redeem for resolved positions via relayer
- [ ] Remove all Polygon RPC / gas configuration (relayer handles gas)

---

## 20. Risk Disclaimer

This software executes real financial transactions on the Polygon blockchain via Polymarket CLOB V2.

- **Dynamic taker fees** can exceed 1% per trade. Blind copy-trading will lose money.
- **Maker orders** are not guaranteed to fill. Unfilled GTC orders generate no profit.
- The `SIMULATION_MODE`, daily loss limit, circuit breaker, and minimum balance guard reduce unintended exposure but do not eliminate risk.
- **CLOB V2 is new.** Smart contract risk, relayer downtime, and API changes are possible.
- Start with the minimum viable capital ($100–200) and validate all behaviour in simulation mode before going live.
- This PRD reflects Polymarket CLOB V2 as of April 24, 2026. Protocol parameters are subject to change.
