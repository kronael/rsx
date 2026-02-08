# Wire Protocol & Message Definitions

## Overview

RSX uses gRPC bidirectional streaming for Gateway ↔ Risk ↔ Matching Engine
communication (v1). Messages are defined in Protocol Buffers (protobuf) with
fixed-point integer representation for prices and quantities. Streams are
multiplexed by user_id and symbol (no per-user streams).

**Operational note:** HTTP/2 flow control and keepalive policies can affect
long-lived streams. Configure consistently across gateway, risk, and matcher.

**Future evolution:**
- v1: gRPC + protobuf (balance of ergonomics and performance)
- No v2 planned (see FUTURE.md)

## Order States

### State Diagram

```
User submits order
       ↓
   PENDING_RISK_CHECK (Gateway validates margin)
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
- Gateway validating margin, position limits
- Not yet sent to Matching Engine
- User-visible: "Order submitted, checking risk"

**PENDING_MATCH:**
- Risk check passed, sent to Matching Engine
- Awaiting execution
- User-visible: "Order pending execution"

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
- User receives ORDER_DONE(RESTING)
- Remains in orderbook until filled or cancelled

**FILLED:**
- Order completely filled
- Example: 100 qty order, 100 filled
- User receives FILL message(s) + ORDER_DONE(FILLED)
- Removed from orderbook

**CANCELLED:**
- User-requested cancellation
- Remaining qty cancelled (unfilled portion)
- User receives ORDER_CANCELLED
- Removed from orderbook

**RISK_REJECTED:**
- Insufficient margin, position limits exceeded
- Rejected by Gateway BEFORE sending to Matching Engine
- User receives ORDER_FAILED(RISK_REJECTED)
- Never enters orderbook

**MATCH_FAILED:**
- Validation failed at Matching Engine
- Reasons: invalid tick size, invalid lot size, duplicate order_id, symbol not found
- User receives ORDER_FAILED(reason)
- Never enters orderbook

## gRPC Service Definition

### Protocol Buffers Schema

```proto
syntax = "proto3";

package rsx.matching;

service MatchingEngine {
    // Bidirectional stream: Gateway sends orders, Matching Engine sends fills
    rpc OrderStream(stream GatewayMessage) returns (stream MatcherMessage);
}

service RiskGateway {
    // Bidirectional stream: Gateway sends orders, Risk sends updates/fills/errors
    rpc Stream(stream GatewayToRisk) returns (stream RiskToGateway);
}

// Gateway → Matching Engine
message GatewayMessage {
    oneof msg {
        NewOrder new_order = 1;
        CancelOrder cancel_order = 2;
        // ModifyOrder modify_order = 3;  // future
    }
}

message NewOrder {
    bytes order_id = 1;      // UUIDv7 (16 bytes)
    uint32 user_id = 2;
    uint32 symbol = 3;
    Side side = 4;
    int64 price = 5;         // Fixed-point: price in smallest tick units
    int64 qty = 6;           // Fixed-point: qty in smallest lot units
    uint64 timestamp_ns = 7; // Nanosecond epoch (for latency tracking)
    bool reduce_only = 8;   // ME enforces: clamp to position
}

message CancelOrder {
    bytes order_id = 1;      // UUIDv7 of order to cancel
    uint32 user_id = 2;      // For validation (must match original order)
}

// Gateway → Risk
message GatewayToRisk {
    oneof msg {
        RiskNewOrder new_order = 1;
        RiskCancelOrder cancel_order = 2;
    }
}

message RiskNewOrder {
    bytes order_id = 1;          // UUIDv7 (16 bytes)
    uint64 client_order_id = 2;
    uint32 user_id = 3;
    uint32 symbol_id = 4;
    Side side = 5;
    int64 price = 6;
    int64 qty = 7;
    TimeInForce tif = 8;
    uint64 timestamp_ns = 9;
    bool reduce_only = 10;      // pass to ME
    bool is_liquidation = 11;   // skip margin check at risk
}

message RiskCancelOrder {
    uint32 user_id = 1;
    uint32 symbol_id = 2;
    oneof key {
        bytes order_id = 3;
        uint64 client_order_id = 4;
    }
    uint64 timestamp_ns = 5;
}

enum Side {
    BUY = 0;
    SELL = 1;
}

enum TimeInForce {
    GTC = 0;
    IOC = 1;
    FOK = 2;
}

// Matching Engine → Gateway
message MatcherMessage {
    oneof msg {
        OrderFill fill = 1;
        OrderDone done = 2;
        OrderFailed failed = 3;
        ConfigApplied config_applied = 4;
    }
}

// Risk → Gateway
message RiskToGateway {
    oneof msg {
        RiskOrderUpdate order_update = 1;
        OrderFill fill = 2;
        StreamError error = 3;
        ConfigApplied config_applied = 4;
    }
}

message RiskOrderUpdate {
    bytes order_id = 1;
    uint64 client_order_id = 2;
    uint32 user_id = 3;
    uint32 symbol_id = 4;
    OrderStatus status = 5;
    int64 filled_qty = 6;
    int64 remaining_qty = 7;
    FailureReason reason = 8;  // set only when status == FAILED
    string details = 9;
}

message StreamError {
    uint32 code = 1;
    string msg = 2;
}

message OrderFill {
    bytes taker_order_id = 1;     // UUIDv7 of aggressor (user who submitted order)
    bytes maker_order_id = 2;     // UUIDv7 of resting order (matched against)
    uint32 taker_user_id = 3;
    uint32 maker_user_id = 4;
    int64 price = 5;              // Fill price (maker's price)
    int64 qty = 6;                // Fill qty
    Side taker_side = 7;          // Side of aggressor
    uint64 timestamp_ns = 8;      // Fill timestamp
    int64 taker_fee = 9;          // fee charged to taker (>= 0)
    int64 maker_fee = 10;         // fee to maker (negative = rebate)
}

message OrderDone {
    bytes order_id = 1;
    FinalStatus final_status = 2;
    int64 filled_qty = 3;         // Total filled qty
    int64 remaining_qty = 4;      // Remaining qty (resting or cancelled)
}

enum FinalStatus {
    FILLED = 0;      // Completely filled
    RESTING = 1;     // Partially filled or unfilled, now in orderbook
    CANCELLED = 2;   // User-cancelled (via CancelOrder)
}

enum OrderStatus {
    FILLED = 0;
    RESTING = 1;
    CANCELLED = 2;
    FAILED = 3;
}

message OrderFailed {
    bytes order_id = 1;
    FailureReason reason = 2;
    string details = 3;  // Human-readable error message
}

message ConfigApplied {
    uint32 symbol_id = 1;
    uint64 config_version = 2;
    uint64 effective_at_ms = 3;
    uint64 applied_at_ns = 4;
}

// Tip sync used by risk main → risk replica (matching replica usage TBD)
message TipSync {
    uint32 symbol_id = 1;
    uint64 seq_no = 2;         // last fully applied seq for this symbol
    uint64 timestamp_ns = 3;
}

enum FailureReason {
    INVALID_TICK_SIZE = 0;       // Price doesn't align to tick size
    INVALID_LOT_SIZE = 1;        // Qty doesn't align to lot size
    SYMBOL_NOT_FOUND = 2;        // Symbol doesn't exist
    DUPLICATE_ORDER_ID = 3;      // Order ID already exists (idempotency check)
    INSUFFICIENT_MARGIN = 4;     // Risk check failed (should be Gateway's job, but double-check)
    OVERLOADED = 5;              // Ingress backpressure at gateway
    INTERNAL_ERROR = 6;          // Matching engine error (should not happen)
    REDUCE_ONLY_VIOLATION = 7;   // No position to reduce
}
```

### Field Encodings

**order_id (bytes, 16B):**
- UUIDv7 encoded as 16-byte binary (not string)
- String encoding (36 chars) wastes wire space
- Binary encoding: 16 bytes vs 36 bytes (2.25x smaller)

**price, qty (int64):**
- Fixed-point integer (reference ORDERBOOK.md section 1)
- Example: BTC-PERP tick_size=0.01 USD, price=$50,000.00 → `price=5000000`
- Example: BTC-PERP lot_size=0.001 BTC, qty=1.5 BTC → `qty=1500`
- Conversion at Gateway (human-readable → fixed-point)
- NO floating point (non-deterministic, precision errors)

**timestamp_ns (uint64):**
- Nanosecond epoch (Unix timestamp * 1e9 + nanos)
- For latency tracking (order submission → fill)
- Matching Engine uses monotonic clock (not wall clock)

**user_id, symbol (uint32):**
- Internal IDs (not strings)
- Gateway maintains string → ID mapping (symbol name → symbol ID)

## Message Flow Sequences

### Fully Filled Order

```
User: "Buy 100 BTC-PERP @ $50,000"

Gateway:
  1. Assign order_id = UUIDv7::new()
  2. Convert: price = 50000.00 / 0.01 = 5000000 (tick units)
              qty = 100 / 0.001 = 100000 (lot units)
  3. Send: NewOrder { order_id, user_id=42, symbol=BTC_PERP, side=BUY,
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

Gateway:
  1. Receive FILL (30 lots), update user position: +30 BTC
  2. Receive FILL (70 lots), update user position: +70 BTC
  3. Receive ORDER_DONE, remove from pending tracking
  4. Forward fills to user: "Filled 100 BTC @ avg $49,999.985"
```

### Partially Filled, Resting Order

```
User: "Buy 100 BTC-PERP @ $48,000" (limit below market)

Gateway:
  1. Send: NewOrder { ..., price=4800000, qty=100000, ... }

Risk:
  1. Validate margin and position limits
  2. Forward to Matching Engine

Matching Engine:
  1. Validate ✓
  2. Match against orderbook:
     - Best ask is $50,000 (above buy limit, no match)
     - No fills
  3. Insert into orderbook as resting bid @ $48,000
  4. Send completion:
     OrderDone { order_id, final_status=RESTING,
                 filled_qty=0, remaining_qty=100000 }

Gateway:
  1. Receive ORDER_DONE(RESTING)
  2. User: "Order resting in orderbook, waiting for match"

[Later: Market drops, seller hits the bid]

Matching Engine:
  1. Incoming sell order matches resting buy
  2. Send fill:
     OrderFill { taker_order_id=sell_order_id, maker_order_id=order_id,
                 price=4800000, qty=100000, ... }
  3. Send completion (to original buyer):
     OrderDone { order_id, final_status=FILLED,
                 filled_qty=100000, remaining_qty=0 }

Gateway:
  1. Receive FILL (100 lots), update user position: +100 BTC
  2. User: "Filled 100 BTC @ $48,000"
```

### Failed Order (Invalid Tick Size)

```
User: "Buy 1 BTC-PERP @ $50,000.005" (invalid, tick_size=$0.01)

Gateway:
  1. Convert: price = 50000.005 / 0.01 = 5000000.5 (fractional ticks, invalid)
  2. Gateway SHOULD reject here (pre-validation)
  3. But if missed, send: NewOrder { ..., price=5000000, ... }
     (user entered 5000000.5, rounded to 5000000 by int conversion — BAD)

Better Gateway validation:
  if (user_price / tick_size) != floor(user_price / tick_size) {
      return ORDER_FAILED(INVALID_TICK_SIZE);
  }

Risk:
  1. Validate margin and position limits
  2. Forward to Matching Engine

Matching Engine (if Gateway missed validation):
  1. Validate: 5000000 % tick_size(5000000) != 0 ✗
  2. Send: OrderFailed { order_id, reason=INVALID_TICK_SIZE,
                         details="Price 5000000 not aligned to tick 1000 at this level" }

Gateway:
  1. Receive ORDER_FAILED
  2. User: "Order rejected: Invalid tick size"
```

### Cancelled Order

```
User has resting buy order (order_id=X, 100 BTC @ $48,000)

User: "Cancel order X"

Gateway:
  1. Send: CancelOrder { order_id=X, user_id=42 }

Risk:
  1. Forward cancel to Matching Engine

Matching Engine:
  1. Lookup order X in FxHashMap
  2. Validate user_id matches (prevent cancel by wrong user)
  3. Remove from orderbook
  4. Mark as CANCELED in FxHashMap (keep for dedup, 5min)
  5. Add to pruning queue: (order_id=X, timestamp=now())
  6. Send: OrderDone { order_id=X, final_status=CANCELLED,
                       filled_qty=0, remaining_qty=100000 }

Gateway:
  1. Receive ORDER_DONE(CANCELLED)
  2. User: "Order cancelled, 100 BTC remaining (unfilled)"
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
        FILL { ... } => { update_position(...); }
        ORDER_DONE { ... } => { remove_from_pending(...); break; }
        ORDER_FAILED { ... } => { notify_user(...); break; }
    }
}
```

**ORDER_DONE signals "no more fills":**
- After ORDER_DONE, Gateway can safely finalize order state
- No more FILL messages will arrive for this order_id

## Completion Signals

### ORDER_DONE: Successful Order

**Sent when:**
- Order fully filled (filled_qty = original_qty, remaining_qty = 0)
- Order partially filled, now resting (filled_qty > 0, remaining_qty > 0)
- Order unfilled, now resting (filled_qty = 0, remaining_qty = original_qty)
- Order cancelled by user (filled_qty = partial, remaining_qty = cancelled)

**Fields:**
- `order_id`: UUIDv7 of order
- `final_status`: FILLED, RESTING, or CANCELLED
- `filled_qty`: Total qty matched
- `remaining_qty`: Qty still in orderbook (RESTING) or cancelled (CANCELLED)

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
- `details`: Human-readable error message (for logging, debugging)

**Exactly one per order:**
- Failed orders get exactly one ORDER_FAILED
- No retries, no partial success

### Completion Guarantee

**Invariant:**
- Every NewOrder receives exactly one completion message:
  - ORDER_DONE OR ORDER_FAILED
- Never both, never zero (unless network failure)

**Timeout handling:**
- If Gateway doesn't receive completion within timeout (10s) → assume network failure
- Gateway returns error to user: "Order submission failed, please retry"
- Order may still be in matching engine (can't tell, no state reconciliation in v1)

## Idempotency & Deduplication

### CREATE (New Order)

**Deduplication key:** order_id (UUIDv7)

**Matching Engine:**
```rust
fn handle_new_order(&mut self, order: NewOrder) -> Result<()> {
    // Check if order_id already exists
    if self.active_orders.contains_key(&order.order_id) {
        return Err(ORDER_FAILED(DUPLICATE_ORDER_ID));
    }

    // Insert into FxHashMap
    let handle = self.orderbook.insert_order(order);
    self.active_orders.insert(order.order_id, handle);

    Ok(())
}
```

**If duplicate:**
- ORDER_FAILED(DUPLICATE_ORDER_ID)
- Original order state unchanged
- User must retry with new order_id (if intended)

### MODIFY (Not Implemented in v1)

**Deduplication strategy (future):**
- Modifies include timestamp (monotonic)
- Matching Engine tracks last modification timestamp per order
- If incoming modify timestamp ≤ last timestamp → ignore (duplicate)
- If incoming modify timestamp > last timestamp → apply (new modification)

**Example:**
```
T=0: MODIFY(order_id=X, new_price=50000, timestamp=100) → applied
T=1: MODIFY(order_id=X, new_price=50001, timestamp=101) → applied
T=2: MODIFY(order_id=X, new_price=50000, timestamp=100) → ignored (duplicate)
```

### CANCEL

**Deduplication strategy:**
- Remove order from orderbook
- Mark as CANCELLED in FxHashMap (keep entry for dedup, do NOT remove)
- Add to pruning queue: (order_id, timestamp)

**If duplicate cancel:**
```rust
fn handle_cancel_order(&mut self, cancel: CancelOrder) -> Result<()> {
    match self.active_orders.get(&cancel.order_id) {
        None => {
            // Order doesn't exist (already cancelled or never existed)
            return Err(ORDER_FAILED(ORDER_NOT_FOUND));
        }
        Some(handle) => {
            let order = &self.orderbook.orders[*handle];
            if !order.is_active {
                // Order already cancelled (duplicate cancel request)
                return Err(ORDER_FAILED(ALREADY_CANCELLED));
            }

            // Cancel order
            self.orderbook.cancel_order(*handle);

            // Mark as cancelled in FxHashMap (keep for dedup)
            self.orderbook.orders[*handle].is_active = false;

            // Add to pruning queue (cleanup after 5min)
            self.pruning_queue.push_back((cancel.order_id, Instant::now()));

            Ok(())
        }
    }
}
```

**Cleanup after 5min:**
```rust
fn cleanup_old_orders(&mut self) {
    let cutoff = Instant::now() - Duration::from_secs(300); // 5min

    while let Some((order_id, timestamp)) = self.pruning_queue.front() {
        if *timestamp > cutoff {
            break; // Still within window, stop scanning
        }

        // Remove from both pruning_queue AND FxHashMap
        self.pruning_queue.pop_front();
        self.active_orders.remove(order_id);
    }
}
```

**Why 5min dedup window:**
- Typical network timeout: 10s
- Typical user retry window: 1-2min
- 5min provides safety margin (duplicate cancels rejected)
- After 5min: memory freed, order_id can be reused (unlikely, UUIDv7 is unique)

**After ME restart:** dedup map empty. Duplicate order_ids
in first 5min undetected. Accepted: UUIDv7 collisions
effectively impossible.

**Why keep cancelled orders in FxHashMap:**
- Prevent duplicate cancel requests (user spams cancel button)
- Avoid "cancel → cleanup → duplicate cancel → cancel succeeds on wrong order"
- Cleanup only after 5min window (safe dedup period)

### Time-Windowed Deduplication

**Strategy:**
- Track recent orders in FxHashMap for 5min
- After 5min: remove from FxHashMap (memory cleanup)
- Assumption: user won't retry with same order_id after 5min

**Balances:**
- **Memory:** FxHashMap size = (order rate) * (5min window)
  - Example: 1000 orders/sec * 300sec = 300K entries * 64B = ~19 MB
- **Safety:** 5min window covers typical retry scenarios
- **Cleanup:** Periodic scan (every 10s) removes old entries

**Alternative (not chosen):**
- Infinite dedup window (never remove from FxHashMap)
- Memory grows unbounded (GBs after hours/days)
- Not suitable for long-running process

## Risk Integration

### Gateway Validates BEFORE Matching Engine

**Order submission flow:**
```
User ──ORDER──→ Gateway
                   │
                   ├─ Check margin (user has enough collateral?)
                   ├─ Check position limits (user within max position?)
                   ├─ Check risk (would this order cause liquidation?)
                   │
                   ├─ If risk check fails: ORDER_FAILED(RISK_REJECTED)
                   └─ If risk check passes: send to Matching Engine
```

**Why Gateway validates:**
- Matching Engine is stateless (no user balances, no positions)
- Risk checks require user context (margin, positions, collateral)
- Faster rejection (no network round-trip to matching engine)

**Matching Engine double-check:**
- Matching Engine MAY re-validate margin (defense in depth)
- If Gateway validation was wrong → ORDER_FAILED(INSUFFICIENT_MARGIN)
- Should not happen (indicates Gateway bug)

### Fills Update Positions → Recalculate Risk

**After fill received:**
```
Gateway receives FILL(user_id=42, side=BUY, qty=100)
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
```proto
message NewOrder {
    int64 price = 5;  // Same: i64, fixed-point
    int64 qty = 6;    // Same: i64, fixed-point
}
```

**No conversion needed:**
- Gateway converts user input (float) → fixed-point (i64)
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
- Gateway handles out-of-order responses (LIFO VecDeque, reference RPC.md)

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
- Event::Fill → OrderFill message
- Event::OrderInserted → ORDER_DONE(RESTING)
- Event::OrderCancelled → ORDER_DONE(CANCELLED)

### Zero-Allocation Principle

**v1 (gRPC + protobuf):**
- Allocates (protobuf encoding, gRPC buffers)
- Acceptable for v1 (ergonomics > performance)

**Future (raw structs over SMRB):**
- Zero allocation (pre-allocated ring buffer)
- See blog/picking-a-wire-format.md for evolution path

## Cross-References

- **ORDERBOOK.md**: Price/Qty types, tick/lot size validation, matching algorithm
- **RPC.md**: Async handling, pending tracking, LIFO VecDeque optimization
- **NETWORK.md**: Component communication, stream lifecycle, topology
- **blog/picking-a-wire-format.md**: Why gRPC now, raw structs later
- **SMRB.md**: Future transport layer (raw structs over shared memory)
