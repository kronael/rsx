---
status: shipped
---

# Message Definitions

## Table of Contents

- [Overview](#overview)
- [Order States](#order-states)
- [Message Schema](#message-schema)
- [Message Flow Sequences](#message-flow-sequences)
- [Fill Streaming Details](#fill-streaming-details)
- [Completion Signals](#completion-signals)
- [Idempotency & Deduplication](#idempotency--deduplication)
- [Risk Integration](#risk-integration)
- [Alignment with Existing Architecture](#alignment-with-existing-architecture)
- [Cross-References](#cross-references)

---

## Overview

Message semantics for Gateway ↔ Risk ↔ Matching Engine
communication. Messages are raw `#[repr(C)]` fixed records
(WAL wire format) with fixed-point integer representation
for prices and quantities. Streams are multiplexed by
user_id and symbol (no per-user streams).

Transport: casting/UDP (see NETWORK.md). No flow control
(removed in commit `87b223e`); receivers recover via NAK
(in-band) or TCP replication (out-of-band).

## Order States

### State Diagram

```
User submits order
       ↓
   PENDING_RISK_CHECK (Risk validates margin)
       ↓
   PENDING_MATCH (Sent to Matching Engine)
       ↓
    MATCHING (Matching Engine processing)
       ↓
  ┌────┴─────┬──────────┬────────────┬────────────┐
  ↓          ↓          ↓            ↓            ↓
PARTIAL   RESTING    FILLED    RISK_REJECTED  MATCH_FAILED
  ↓          ↓          ↓            ↓            ↓
(more     (done,    (done,       (done,       (done,
fills)    in book)  removed)    rejected)    rejected)
  ↓
FILLED or CANCELLED (if user cancels)
  ↓
(done, removed)
```

### State Descriptions

**PENDING_RISK_CHECK:**
- Risk validating margin, position limits; the gateway has forwarded the order
- Not yet sent to Matching Engine
- User-visible: nothing; the gateway sends no order-accepted ACK

**PENDING_MATCH:**
- Risk check passed, sent to Matching Engine
- Awaiting execution
- User-visible: nothing

**MATCHING:**
- Matching Engine actively processing order
- Transient state (micro/milliseconds)
- Not user-visible (internal only)

**PARTIAL:**
- Order partially filled, still matching
- Example: 100 qty order, 30 filled, 70 remaining
- User receives FILL message(s)
- May transition to FILLED, RESTING, or CANCELLED

**RESTING:**
- Order in orderbook, awaiting counterparty
- Example: Limit buy below market, no immediate match
- User receives ORDER_INSERTED (a client `U` with status 1); the ME writes no
  ORDER_DONE for a resting order
- Remains in orderbook until filled or cancelled

**FILLED:**
- Order completely filled
- Example: 100 qty order, 100 filled
- User receives FILL message(s) + ORDER_DONE(FILLED)
- Removed from orderbook

**CANCELLED:**
- User-requested cancellation, an IOC remainder, or a post-only order that
  would cross
- Remaining qty cancelled (unfilled portion)
- The ME writes ORDER_DONE(CANCELLED) with the `CANCEL_REASON_*` code; a user
  cancel of a resting order is preceded by the book event ORDER_CANCELLED. The
  user receives one status-2 `U` carrying the reason
- Removed from orderbook

**RISK_REJECTED:**
- Insufficient margin, position limits exceeded, user in liquidation, wrong
  shard, symbol not configured, order below the symbol's minimum notional
- Rejected by Risk BEFORE sending to Matching Engine; the gateway checks none
  of these
- User receives ORDER_FAILED with the matching `FailureReason`
  (`INSUFFICIENT_MARGIN`, `USER_IN_LIQUIDATION`, `WRONG_SHARD`,
  `SYMBOL_NOT_FOUND`, `ORDER_TOO_SMALL`)
- Never enters orderbook

**MATCH_FAILED:**
- Validation failed at Matching Engine
- Reasons: invalid tick size, invalid lot size, duplicate order_id, symbol not found
- User receives ORDER_FAILED(reason)
- Never enters orderbook

## Message Schema

Messages are fixed-size `#[repr(C, align(64))]` records sent over
casting/UDP. All data payloads start with the casting prefix:
`seq:u64, ver:u16, kind:u8, _pad:u8, len:u32` (casting.md).

Record layouts are defined in:
- `rsx-messages` (domain wire records: Fill/BBO/Order*/Mark/Liquidation,
  and the order request on each hop)
- `rsx-cast::records` (transport control records: Nak,
  CastHeartbeat, ReplicationRequest, CaughtUpRecord)

### Record Inventory

Domain records (`rsx-messages`), all `#[repr(C, align(64))]`:

| Record | Size | RECORD_* constant | Value |
|--------|------|-------------------|-------|
| `FillRecord` | 128 | `RECORD_FILL` | 0 |
| `BboRecord` | 128 | `RECORD_BBO` | 1 |
| `OrderInsertedRecord` | 64 | `RECORD_ORDER_INSERTED` | 2 |
| `OrderCancelledRecord` | 64 | `RECORD_ORDER_CANCELLED` | 3 |
| `OrderDoneRecord` | 128 | `RECORD_ORDER_DONE` | 4 |
| `ConfigAppliedRecord` | 64 | `RECORD_CONFIG_APPLIED` | 5 |
| `OrderAcceptedRecord` | 128 | `RECORD_ORDER_ACCEPTED` | 7 |
| `MarkPriceRecord` | 64 | `RECORD_MARK_PRICE` | 8 |
| `OrderRequest` (gateway to risk), `OrderMessage` (risk to ME) | 64, 128 | `RECORD_ORDER_REQUEST` | 9 |
| (reserved: order response; nothing writes it) | — | `RECORD_ORDER_RESPONSE` | 10 |
| `CancelRequest` | 64 | `RECORD_CANCEL_REQUEST` | 11 |
| `OrderFailedRecord` | 64 | `RECORD_ORDER_FAILED` | 12 |
| `LiquidationRecord` | 64 | `RECORD_LIQUIDATION` | 13 |

Transport records (`rsx-cast::records`), all
`#[repr(C, align(64))]`:

| Record | Size | RECORD_* constant | Value |
|--------|------|-------------------|-------|
| `CaughtUpRecord` | 128 | `RECORD_CAUGHT_UP` | 6 |
| `Nak` | 64 | `RECORD_NAK` | 0x11 |
| `CastHeartbeat` | 64 | `RECORD_HEARTBEAT` | 0x12 |
| `ReplicationRequest` | 64 | `RECORD_REPLICATION_REQUEST` | 0x13 |

`RECORD_STATUS_MESSAGE = 0x10` was retired in `87b223e`
when flow control was dropped; the constant value stays
reserved.

Sizes/alignments are enforced by `const _: () = assert!(...)`
compile-time checks in each crate. Domain values 0–13 share
the same numeric space as transport value 6 (`RECORD_CAUGHT_UP`);
transport values ≥ 0x10 are disjoint from domain values.

### Per-hop latency timestamps

`FillRecord` and `OrderDoneRecord` carry five `u64` per-hop timestamps each
(`59-latency-observability.md`); both records are 128 bytes:

| Field | Set by | Notes |
|-------|--------|-------|
| `gw_in_ns` | Gateway | Gateway ingress; echoed from `OrderRequest.timestamp_ns` through the order lifecycle. |
| `risk_in_ns` | Risk | Risk's ingress timestamp for the order, carried through the accepted ring and `OrderMessage.risk_in_ns` (`rsx-risk/src/cast_loop.rs:357`). |
| `me_in_ns` | Matching | ME's own ingress timestamp for the order that produced this fill. |
| `match_done_ns` | Matching | Stamped after the match cycle that produced this fill completes. `engine = match_done_ns - me_in_ns`. |
| `gw_out_ns` | Gateway | Stamped just before the gateway turns this record into the outbound client frame. `internal = gw_out_ns - gw_in_ns`. |

A consumer treats `0` as "unset", not as a real timestamp. A
subtraction across hops assumes clock-synced hosts (PTP in
production; trivially true on a single-box dev setup).

`OrderDoneRecord` carries the same five fields. The ME sets the first four
only on the done of an order that never entered the book, and writes 0 on the
done of a resting order (`rsx-matching/src/wal.rs:235`). It leaves `gw_out_ns`
0; the gateway stamps it on the outbound `U`. `OrderInsertedRecord`,
`OrderCancelledRecord` and `OrderFailedRecord` are 64 bytes with no spare
capacity and carry none of them, so the gateway sends zeros in those `U`
frames.

### Fill fee rates

`FillRecord.taker_fee_bps` and `FillRecord.maker_fee_bps` are `i32` basis
points (negative is a rebate). The ME writes both as 0. Risk stamps the rates
it charged (`28-risk.md` § Orderbook Event Ingestion) into the copy it forwards on the
gateway stream, and the gateway computes each receiver's `F` fee with
`rsx_types::calculate_fee`, the function risk charges with.

### CancelReason constants

`OrderDoneRecord.cancel_reason` is a `u8` (0 when `final_status` is FILLED):

| Constant | Value |
|----------|-------|
| `CANCEL_REASON_USER_CANCEL` | 0 |
| `CANCEL_REASON_REDUCE_ONLY` | 1 |
| `CANCEL_REASON_EXPIRY` | 2 |
| `CANCEL_REASON_SYSTEM` | 3 |
| `CANCEL_REASON_POST_ONLY_REJECT` | 4 |
| `CANCEL_REASON_OTHER` | 5 |

The ME writes `CANCEL_REASON_USER_CANCEL` for a user cancel,
`CANCEL_REASON_EXPIRY` for an IOC remainder and
`CANCEL_REASON_POST_ONLY_REJECT` for a post-only order that would cross;
`CANCEL_REASON_REDUCE_ONLY`, `_SYSTEM` and `_OTHER` are defined and no ME
writes them. The gateway puts `cancel_reason` on the client `U` frame for
every ORDER_DONE. The `U` reason slot is one byte read by status: status 3 is
a `FailureReason`, status 2 a `CANCEL_REASON_*`, any other status 0
(`49-webproto.md`).

`OrderCancelledRecord` is the book event: it records that a resting order left
the book (the ME replays from it, marketdata removes the level quantity with
it) and its `reason` is always `CANCEL_REASON_USER_CANCEL`. Risk and the
gateway ignore it; the order's terminal is the ORDER_DONE that follows.

### FailureReason constants

`OrderFailedRecord.reason` is a `u8` (`rsx_types::FailureReason`). The
matching engine writes these codes, not its book-side enum: tick or
non-positive price → `INVALID_TICK_SIZE`, lot or non-positive qty →
`INVALID_LOT_SIZE`, duplicate order id → `DUPLICATE_ORDER_ID`,
reduce-only violation → `REDUCE_ONLY_VIOLATION`, fill-or-kill that cannot
fill in full → `FILL_OR_KILL`.

| Constant | Value |
|----------|-------|
| `INVALID_TICK_SIZE` | 0 |
| `INVALID_LOT_SIZE` | 1 |
| `SYMBOL_NOT_FOUND` | 2 |
| `DUPLICATE_ORDER_ID` | 3 |
| `INSUFFICIENT_MARGIN` | 4 |
| `OVERLOADED` | 5 |
| `INTERNAL_ERROR` | 6 |
| `REDUCE_ONLY_VIOLATION` | 7 |
| `NETWORK_ERROR` | 8 |
| `RATE_LIMIT` | 9 |
| `TIMEOUT` | 10 |
| `USER_IN_LIQUIDATION` | 11 |
| `WRONG_SHARD` | 12 |
| `FILL_OR_KILL` | 13 |
| `ORDER_TOO_SMALL` | 14 |

## Message Flow Sequences

### Fully Filled Order

```
User: "Buy 100 BTC-PERP @ $50,000"

Client:
  1. Convert with the symbol's tick and lot size:
              price = 50000.00 / 0.01 = 5000000 (tick units)
              qty = 100 / 0.001 = 100000 (lot units)
  2. Send N [symbol, side, 5000000, 100000, "cid", tif] over the WebSocket

Gateway:
  1. Check the rate limits, the cid map, the symbol, tick and lot alignment
  2. Assign order_id = UUIDv7::new()
  3. Send: OrderRequest { order_id, user_id=42, symbol=BTC_PERP, side=BUY,
                          price=5000000, qty=100000, timestamp_ns=... } to Risk

Risk:
  1. Validate margin and position limits
  2. Forward NewOrder to Matching Engine

Matching Engine:
  1. Validate tick size: 5000000 % tick_size(5000000) == 0 ✓
  2. Validate lot size: 100000 % lot_size(5000000) == 0 ✓
  3. Match against orderbook:
     - Matches 30 lots @ $49,999.99 (maker order X)
     - Matches 70 lots @ $49,999.98 (maker order Y)
  4. Send fills:
     OrderFill { taker_order_id=order_id, maker_order_id=X,
                 taker_user_id=42, maker_user_id=123,
                 price=4999999, qty=30000, taker_side=BUY, ... }
     OrderFill { taker_order_id=order_id, maker_order_id=Y,
                 taker_user_id=42, maker_user_id=456,
                 price=4999998, qty=70000, taker_side=BUY, ... }
  5. Send completion:
     OrderDone { order_id, final_status=FILLED,
                 filled_qty=100000, remaining_qty=0 }

Risk:
  1. Apply FILL (30 lots) and FILL (70 lots) to the user's position
  2. Forward the fills and ORDER_DONE to the Gateway

Gateway:
  1. Receive FILL (30 lots), send the user an F frame
  2. Receive FILL (70 lots), send the user an F frame
  3. Receive ORDER_DONE, remove from pending tracking, send the user a U
     frame with status 0
```

### Resting Order

```
User: "Buy 100 BTC-PERP @ $48,000" (limit below market)

Gateway:
  1. Send: OrderRequest { ..., price=4800000, qty=100000, ... } to Risk

Risk:
  1. Validate margin and position limits
  2. Forward to Matching Engine

Matching Engine:
  1. Validate ✓
  2. Match against orderbook:
     - Best ask is $50,000 (above buy limit, no match)
     - No fills
  3. Insert into orderbook as resting bid @ $48,000
  4. Send: OrderInserted { order_id, price=4800000, qty=100000, ... }
     (no ORDER_DONE: a resting order is not done)

Gateway:
  1. Receive ORDER_INSERTED, mark the pending entry resting
  2. Send the user a U frame with status 1 (RESTING) and remaining 100000

[Later: Market drops, seller hits the bid]

Matching Engine:
  1. Incoming sell order matches resting buy
  2. Send fill:
     OrderFill { taker_order_id=sell_order_id, maker_order_id=order_id,
                 price=4800000, qty=100000, ... }
  3. Send completion (to original buyer):
     OrderDone { order_id, final_status=FILLED,
                 filled_qty=100000, remaining_qty=0 }

Risk:
  1. Apply the FILL (100 lots) to the user's position

Gateway:
  1. Receive FILL (100 lots), send the user an F frame
  2. Receive ORDER_DONE(FILLED), remove from pending tracking, send the
     user a U frame with status 0
```

### Failed Order (Invalid Tick Size)

```
User: "Buy 1 BTC-PERP" at a price off the tick (tick_size = 10)

Client:
  1. Send N [symbol, side, 5000005, qty, "cid", tif]; prices are tick units

Gateway:
  1. Check: 5000005 % 10 != 0
  2. Send the user E[1008, "price not tick aligned"]; nothing is forwarded
     and no order exists

Matching Engine (if the gateway's tick differs from the engine's):
  1. Validate: 5000005 % tick_size != 0 ✗
  2. Send: OrderFailed { order_id, reason=INVALID_TICK_SIZE }

Gateway:
  1. Receive ORDER_FAILED
  2. Send the user a U frame with status 3 and reason 0 (INVALID_TICK_SIZE)
```

### Cancelled Order

```
User has resting buy order (order_id=X, 100 BTC @ $48,000)

User: "Cancel order X"

Gateway:
  1. Find X in the user's pending orders, send: CancelRequest { order_id=X, user_id=42 }

Risk:
  1. Forward cancel to Matching Engine

Matching Engine:
  1. Lookup order X in FxHashMap
  2. Validate user_id matches (prevent cancel by wrong user)
  3. Remove from orderbook
  4. Send: OrderCancelled { order_id=X, remaining_qty=100000, ... }
  5. Send: OrderDone { order_id=X, final_status=CANCELLED,
                       cancel_reason=USER_CANCEL,
                       filled_qty=0, remaining_qty=100000 }
     (the dedup key of X stays in the dedup set for its 1 hour window)

Risk:
  1. Release the frozen margin of X on ORDER_DONE
  2. Forward ORDER_DONE to the Gateway; ORDER_CANCELLED is not forwarded

Gateway:
  1. Receive ORDER_DONE(CANCELLED, USER_CANCEL)
  2. Send one status-2 `U` frame with reason 0 (`49-webproto.md`)
```

## Fill Streaming Details

### Multiple Fills Per Order

An order can generate 0+ FILL messages:
- 0 fills: Order rests in orderbook immediately (no match)
- 1 fill: Order matches one maker order, fully filled
- N fills: Order matches N maker orders (walk through orderbook)

**Example: Large market taker**
```
Orderbook asks:
  $50,000.00: 10 BTC (maker A)
  $50,000.01: 20 BTC (maker B)
  $50,000.02: 30 BTC (maker C)

User: "Buy 50 BTC @ market" (= buy limit $50,000.02 or higher)

Matching Engine:
  1. Match 10 BTC @ $50,000.00 (maker A) → FILL(qty=10, price=5000000)
  2. Match 20 BTC @ $50,000.01 (maker B) → FILL(qty=20, price=5000001)
  3. Match 20 BTC @ $50,000.02 (maker C) → FILL(qty=20, price=5000002)
  4. ORDER_DONE(FILLED, filled_qty=50, remaining_qty=0)

Gateway receives: 3 FILL messages + 1 ORDER_DONE
User sees: "Filled 50 BTC @ avg $50,000.01"
```

### Fill Atomicity

Each FILL message is atomic (one maker-taker match):
- One maker order + one taker order → one FILL
- Fill price = maker's price (maker sets the price)
- Fill qty = min(maker remaining, taker remaining)

**No partial fills within a FILL message:**
- If maker has 10 BTC, taker has 100 BTC → FILL(qty=10), not FILL(qty=5) + FILL(qty=5)

### Fills Precede ORDER_DONE

Message order:
1. FILL (0+ times)
2. ORDER_DONE or ORDER_FAILED (exactly once)

**Gateway must handle stream correctly:**
```rust
loop {
    match stream.recv() {
        FILL { ... } => { send_f_frame(...); }
        ORDER_DONE { ... } => { remove_from_pending(...); send_u_frame(...); break; }
        ORDER_FAILED { ... } => { remove_from_pending(...); send_u_frame(...); break; }
    }
}
```

**ORDER_DONE signals "no more fills":**
- After ORDER_DONE, Gateway can safely drop the pending entry
- No more FILL messages will arrive for this order_id

## Completion Signals

### ORDER_DONE: Successful Order

**Sent when:**
- Order fully filled (filled_qty = original_qty, remaining_qty = 0)
- Order cancelled (filled_qty = partial, remaining_qty = cancelled): an IOC
  remainder, a post-only order that would cross (never rests, no
  ORDER_CANCELLED), or a user cancel of a resting order

A resting order is not done: the ME writes ORDER_INSERTED for it and ORDER_DONE
only when it later fills or is cancelled. `final_status` 1 (RESTING) is
defined (`rsx_types::FinalStatus`) and no ME writes it.

**Fields:**
- `order_id`: UUIDv7 of order
- `final_status`: FILLED (0) or CANCELLED (2)
- `cancel_reason`: `CANCEL_REASON_*` code, 0 when FILLED
- `filled_qty`: Total qty matched
- `remaining_qty`: Qty cancelled (CANCELLED), 0 (FILLED)

**Exactly one per order:**
- Every successful order gets exactly one ORDER_DONE
- No ORDER_DONE → order still pending (network issue, timeout, or matching)

### ORDER_FAILED: Rejected Order

**Sent when:**
- Validation failed (tick size, lot size, symbol not found)
- Deduplication rejected (duplicate order_id)
- Internal error (matching engine panic, should not happen)

**Fields:**
- `order_id`: UUIDv7 of order
- `reason`: FailureReason enum

**Exactly one per order:**
- Failed orders get exactly one ORDER_FAILED
- No retries, no partial success

### Completion Guarantee

**Invariant:**
- Every NewOrder receives exactly one completion message:
  - ORDER_DONE OR ORDER_FAILED
- Never both, never zero (unless network failure)

**Timeout handling:**
- The gateway waits `RSX_GW_ORDER_TIMEOUT_MS` (10s) for the first response to an order
  (ORDER_INSERTED, ORDER_DONE or ORDER_FAILED). An order that
  has not had one is removed from the pending set and the user gets a `U` with
  status 3 and reason TIMEOUT (10). An order the gateway has seen inserted is
  never swept.
- The order may still be in the matching engine (can't tell, no state reconciliation in v1)

## Idempotency & Deduplication

Two owners, one window. The gateway owns the client order id (`cid`) and the
matching engine owns the order id; both remember for 1 hour, so a `cid`
binding never outlives the engine's memory of the order id it names.

### Gateway: client order id

The gateway mints the order id; a client never sends one. It binds
`(user_id, cid)` to that order id when it forwards an order, in memory, for
1 hour (`rsx-gateway/src/client_ids.rs`). A repeat of the `cid` inside the
window is not forwarded: the sender gets a `U` with status 3 and reason
`DUPLICATE_ORDER_ID` naming the first order. A gateway restart empties the
map. `49-webproto.md` § ACK and retry has the client-visible behaviour; the
target design is `61-cid-idempotency.md` (draft).

### CREATE (New Order)

**Deduplication key:** `(user_id, order_id_hi, order_id_lo)` (the UUIDv7
bytes split into hi/lo u64, keyed per user).

The ME checks the key before it writes ORDER_ACCEPTED
(`rsx-matching/src/order.rs`, `rsx-matching/src/dedup.rs`). A key already in
the set is a duplicate:

- ORDER_FAILED(`DUPLICATE_ORDER_ID`) is written to the WAL and both casts
- the original order's state is unchanged

A client retry never reaches the ME as a repeat: the gateway answers a
repeated `cid` itself and forwards nothing.

### MODIFY (Not Implemented in v1)

**Deduplication strategy (future):**
- Modifies include timestamp (monotonic)
- Matching Engine tracks last modification timestamp per order
- If incoming modify timestamp ≤ last timestamp → ignore (duplicate)
- If incoming modify timestamp > last timestamp → apply (new modification)

### CANCEL

A cancel removes the order from the book (`rsx-matching/src/cancel.rs`). It
does not touch the dedup set, so a cancelled order's id stays rejected as a
duplicate until its window ends. A cancel for an order that is not in the
ME's order index (already filled, already cancelled, never existed) logs and
emits nothing.

### Time-Windowed Deduplication

**Window:** 1 hour (`DEDUP_WINDOW`, `rsx-matching/src/dedup.rs`). Keys are
held in a hash set plus an insertion-ordered queue; a sweep every 10 seconds
(`DEDUP_CLEANUP_INTERVAL`) pops the keys older than the window.

**Restart:** the window survives an ME restart. Every accepted order writes
ORDER_ACCEPTED before it matches, and recovery re-seeds each key younger than
1 hour from the retained WAL, with its remaining time to live
(`rebuild_dedup_window` in `rsx-matching/src/wal.rs`;
`rsx-matching/notes/dedup-persistence.md`).

**Memory:** one set slot and one queue slot per live key, about 65 bytes
before hash-table slack (computed from the entry sizes, not measured). At 1000 orders per second on
one symbol, an hour holds 3.6 million keys, about 230 MB. The set has no
capacity bound; it grows with the order rate times the window.

**Why 1 hour:** the gateway's `cid` binding lives as long, so a resend of a
`cid` after the ME has forgotten the order id cannot reach the ME as a fresh
order.

## Risk Integration

### Risk Validates BEFORE the Matching Engine

**Order submission flow:**
```
User ──N──→ Gateway ──OrderRequest──→ Risk ──OrderMessage──→ Matching Engine
              │                        │
              │ rate limits, cid map,  ├─ Check margin (user has enough collateral?)
              │ symbol, tick, lot,     ├─ Check position limits (user within max position?)
              │ capacity, circuit      ├─ Check the user is not in liquidation
              │                        │
              └─ fails: E[code] to     ├─ If a check fails: ORDER_FAILED(reason) to Gateway
                 the user              └─ If all pass: send to Matching Engine
```

**Why Risk validates:**
- Matching Engine holds no balances, margin, or collateral (only a
  per-symbol net position for the reduce-only clamp) — so it cannot do
  the pre-trade risk check
- Risk checks require user context (margin, positions, collateral); the
  gateway holds none of it and checks none of it
- Each concern has one owner (repo `CLAUDE.md` § Trust boundaries): the gateway
  owns auth, wire parsing and structural validation; Risk owns margin and
  position permission

**Matching Engine does not re-check margin:**
- It enforces structural book invariants only: positive and tick-aligned
  price, positive and lot-aligned quantity, duplicate order id
- An order that reaches it has passed Risk

### Fills Update Positions → Recalculate Risk

**After fill received:**
```
Risk receives FILL(user_id=42, side=BUY, qty=100)
  ↓
Update position: user.long_qty += 100
  ↓
Recalculate margin:
  position_value = long_qty * mark_price - short_qty * mark_price
  initial_margin = position_value * initial_margin_rate
  maintenance_margin = position_value * maintenance_margin_rate
  ↓
Check if equity < maintenance_margin:
  equity = collateral + unrealized_pnl
  if equity < maintenance_margin:
      trigger liquidation (send market orders to close position)
```

**Risk checks happen:**
- **Before order:** Ensure user can open position (pre-check)
- **After fill:** Ensure position doesn't violate maintenance margin (post-check)

## Alignment with Existing Architecture

### Fixed-Point Price/Qty

**From ORDERBOOK.md section 1:**
```rust
pub struct Price(pub i64);  // Price in smallest tick units
pub struct Qty(pub i64);    // Qty in smallest lot units
```

**Wire protocol matches:**
```rust
pub struct OrderRequest {
    pub price: i64,  // Same: i64, tick units
    pub qty: i64,    // Same: i64, lot units
    // ...
}
```

**No conversion in the exchange:**
- The client converts human units to tick and lot units at its display
  boundary; the order channel carries `i64` (`49-webproto.md`)
- The gateway checks alignment and never converts or parses a float
- Matching Engine operates on i64 directly
- No float arithmetic in matching engine

### Single-Threaded Matching

**From ORDERBOOK.md:**
- Single-threaded event loop
- No locks, no mutexes
- O(1) operations

**Wire protocol supports:**
- Synchronous request/response (matching is serial)
- Responses come back in order processed (not necessarily order sent)
- Gateway handles out-of-order responses (per-user queue searched newest first, `29-rpc.md`)

### Event Generation

**From ORDERBOOK.md section 6:**
```rust
enum Event {
    Fill { maker_order_id, taker_order_id, price, qty, ... },
    OrderInserted { order_id, price, qty, ... },
    OrderCancelled { order_id, remaining_qty, ... },
}
```

**Wire protocol translates:**
- Event::Fill → FILL record
- Event::OrderInserted → ORDER_INSERTED
- Event::OrderCancelled → ORDER_CANCELLED (the book event of a user cancel)
- Event::OrderDone → ORDER_DONE (FILLED, or CANCELLED with its
  `CANCEL_REASON_*`)
- Event::OrderFailed → ORDER_FAILED

### Zero-Allocation Principle

**v1 (raw #[repr(C)] over casting/UDP):**
- Minimal allocation (UDP send buffers only)
- WAL wire format: zero-copy on read path

**Future (raw structs over SMRB):**
- Zero allocation (pre-allocated ring buffer)
- Eliminates extra transport overhead for co-located processes

## Cross-References

- **ORDERBOOK.md**: Price/Qty types, tick/lot size validation, matching algorithm
- **RPC.md**: Async handling, pending tracking, LIFO VecDeque optimization
- **NETWORK.md**: Component communication, stream lifecycle, topology
- **blog/picking-a-wire-format.md**: Wire format decision history
- **SMRB.md**: Future transport layer (raw structs over shared memory)
