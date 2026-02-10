# Order Validation Edge Cases

Comprehensive documentation of edge cases for order validation
across Gateway, Risk, and Matching Engine layers. Order validation
happens at multiple stages: Gateway (basic field validation),
Risk (margin/position checks), and Matching Engine (tick/lot
size enforcement).

## Validation Layers

```
User Order
    ↓
Gateway (Field Validation)
    ├─ parse errors, malformed inputs
    ├─ rate limiting
    └─ basic sanity checks
    ↓
Risk (Pre-Trade Checks)
    ├─ margin calculation
    ├─ position limits
    └─ reduce-only enforcement
    ↓
Matching Engine (Final Validation)
    ├─ tick/lot size alignment
    ├─ deduplication
    └─ symbol existence
```

## 1. Field Validation Edge Cases (Gateway Layer)

### 1.1 Price Edge Cases

| Case | Input | Validation | Outcome |
|------|-------|------------|---------|
| Zero price | px=0 | `price.0 > 0` | REJECT: invalid price |
| Negative price | px=-1000 | `price.0 > 0` | REJECT: invalid price |
| MAX i64 price | px=9223372036854775807 | OK if valid tick | ACCEPT (notional overflow checked in Risk) |
| Near-MAX price | px=2^62 | OK if valid tick | ACCEPT (may overflow notional) |
| Fractional tick | $50000.005 with tick=$0.01 | conversion rounds | SILENT BUG: becomes $50000.00 |

**Correct Gateway validation:**
```rust
// BEFORE converting to i64, check alignment
if (user_price_float / tick_size_float).fract() != 0.0 {
    return ORDER_FAILED(INVALID_TICK_SIZE);
}
price_raw = (user_price_float / tick_size_float) as i64;
```

**Edge case: Rounding on conversion**
- User sends `50000.005` with `tick_size=0.01`
- Naive: `(50000.005 / 0.01) as i64 = 5000000` (rounds down)
- Order appears valid but silently modified
- Solution: Gateway MUST validate before conversion

### 1.2 Quantity Edge Cases

| Case | Input | Validation | Outcome |
|------|-------|------------|---------|
| Zero qty | qty=0 | `qty.0 > 0` | REJECT: invalid qty |
| Negative qty | qty=-100 | `qty.0 > 0` | REJECT: invalid qty |
| MAX i64 qty | qty=9223372036854775807 | OK if valid lot | ACCEPT (position overflow checked in Risk) |
| Fractional lot | 1.0005 BTC with lot=0.001 | conversion rounds | SILENT BUG: becomes 1.000 BTC |
| Sub-lot quantity | 0.0001 BTC with lot=0.001 | rounds to 0 | REJECT: qty becomes 0 |

**Sub-lot rounding hazard:**
```rust
// User sends qty=0.0001 BTC, lot_size=0.001 BTC
qty_raw = (0.0001 / 0.001) as i64; // = 0
// Validation rejects: qty.0 > 0 fails
```

### 1.3 Client Order ID Edge Cases

| Case | Input | Validation | Outcome |
|------|-------|------------|---------|
| Empty cid | "" | zero-padded array | ACCEPT: [0; 20] |
| 20-char cid | "12345678901234567890" | exact fit | ACCEPT: full bytes |
| >20 chars | "123...789" (25 chars) | truncate to 20 | ACCEPT: truncated |
| UTF-8 multi-byte | "测试订单123" | byte count != char count | Truncation may split UTF-8 |
| Non-ASCII | binary bytes, emoji | bytes copied as-is | ACCEPT: raw bytes |

**UTF-8 truncation hazard:**
```rust
let cid_str = "测试订单1234567890123"; // 3*3 + 13 = 22 bytes
let mut cid_bytes = [0u8; 20];
let src = cid_str.as_bytes(); // 22 bytes
cid_bytes[..20].copy_from_slice(&src[..20]); // splits "3" mid-byte
// Result: invalid UTF-8 in cid_bytes[18..20]
```

**Impact:** Client order ID used for cancel-by-cid. If truncation
splits UTF-8 sequence, cancel lookup may fail. Not critical (can
use order_id), but violates user expectations.

**Solution:** Gateway MAY reject cid if byte length >20, OR document
that cid is byte-limited not char-limited.

### 1.4 Time In Force Edge Cases

| Case | Input | Validation | Outcome |
|------|-------|------------|---------|
| Invalid enum | tif=99 | enum cast | May panic or UB |
| GTC default | tif omitted | default 0 | ACCEPT: GTC |
| IOC with price far from market | tif=IOC, px >> ask | no immediate fill | CANCEL: no match |
| FOK with insufficient depth | tif=FOK, qty > book depth | partial match fails | CANCEL: partial not allowed |

**Enum safety:**
```rust
// WRONG: unchecked cast
let tif: TimeInForce = unsafe { std::mem::transmute(raw_tif) };

// CORRECT: validated conversion
let tif = match raw_tif {
    0 => TimeInForce::GTC,
    1 => TimeInForce::IOC,
    2 => TimeInForce::FOK,
    _ => return ORDER_FAILED(INVALID_TIF),
};
```

### 1.5 Reduce-Only Edge Cases

| Case | Scenario | Validation | Outcome |
|------|----------|------------|---------|
| ro=1, no position | User has 0 position | ME checks position | REJECT: ReduceOnlyViolation |
| ro=1, opposite side | Long position, ro SELL | OK | ACCEPT: reduces position |
| ro=1, same side | Long position, ro BUY | ME checks direction | REJECT: ReduceOnlyViolation |
| ro=1, qty > position | Long 10, ro SELL 20 | ME enforces qty ≤ pos | REJECT or PARTIAL: fill 10 only |

**Risk engine behavior (RISK.md §6):**
```rust
if order.reduce_only {
    return Ok(0); // No margin check, pass through to ME
}
```

Risk does NOT validate reduce-only semantics (no position state).
Matching Engine enforces based on current fills.

### 1.6 Symbol ID Edge Cases

| Case | Input | Validation | Outcome |
|------|----------|------------|---------|
| symbol_id=0 | Valid if symbol 0 exists | Gateway cache lookup | ACCEPT or REJECT |
| symbol_id=999 | Invalid, no such symbol | Gateway cache miss | REJECT: SymbolNotFound |
| symbol_id=MAX_u32 | Out of bounds | Array access OOB | Panic or REJECT |

**Gateway config cache:**
```rust
// WRONG: unchecked array access
let config = &self.symbol_configs[symbol_id];

// CORRECT: bounds check
let config = self.symbol_configs.get(symbol_id)
    .ok_or(SYMBOL_NOT_FOUND)?;
```

## 2. Margin & Risk Edge Cases (Risk Layer)

### 2.1 Notional Overflow

**Problem:** `notional = price * qty` can overflow i64.

| Price | Qty | Notional (i64) | Result |
|-------|-----|----------------|--------|
| 5000000 | 1000000 | 5_000_000_000_000 | OK (fits i64) |
| 5000000 | 10^12 | 5 * 10^18 | Overflow (>2^63) |
| 2^31 | 2^31 | 2^62 | Overflow (>2^63) |

**Implementation (RISK.md §6, margin.rs:82):**
```rust
let order_notional = (order.price as i128
    * order.qty as i128) as i64;
```

**Edge case: Downcast from i128 to i64**
- If notional >2^63-1, cast truncates (wraps)
- Result: negative notional or garbage value
- Margin check passes with wrong value

**Correct approach:**
```rust
let notional_i128 = order.price as i128 * order.qty as i128;
if notional_i128 > i64::MAX as i128 {
    return Err(RejectReason::OrderTooLarge);
}
let order_notional = notional_i128 as i64;
```

### 2.2 Available Margin Edge Cases

| Case | Equity | IM | Frozen | Available | Order IM | Result |
|------|--------|----|----|-----------|----------|--------|
| Exact match | 1000 | 500 | 200 | 300 | 300 | ACCEPT (available = needed) |
| 1 unit short | 1000 | 500 | 200 | 300 | 301 | REJECT: insufficient |
| Negative equity | -100 | 500 | 0 | -600 | 100 | REJECT: underwater |
| Zero available | 1000 | 800 | 200 | 0 | 10 | REJECT: fully utilized |
| Frozen > equity | 1000 | 200 | 900 | -100 | 50 | REJECT: over-reserved |

**Frozen margin accumulation:**
```rust
// On each order submission
account.frozen_margin += margin_needed;

// On ORDER_DONE
account.frozen_margin -= margin_needed;
```

**Edge case: Frozen never released**
- If ORDER_DONE message lost (network failure), frozen stays high
- User cannot place new orders (available = 0)
- Recovery: Gateway timeout + manual frozen adjustment

**v1 behavior:** No automatic recovery. Operator must manually
adjust frozen margin after confirmed order completion.

### 2.3 Position Flip Edge Cases

**Scenario:** User has long position, places sell order > long qty.

| Initial Position | Order | Expected Behavior | Implementation |
|------------------|-------|-------------------|----------------|
| Long 10 | Sell 15 | Close 10, open Short 5 | Two-step: realize PnL at flip |
| Long 10 | Sell 10 | Close 10, flat | Single step |
| Short 5 | Buy 8 | Close 5, open Long 3 | Two-step |

**Implementation (RISK.md §3, edge cases line 147):**
```rust
// On fill arrival
if position flips (sign change):
    1. Close old position at fill price (realize PnL)
    2. Open new position with entry = fill price
// Else:
    update existing position
```

**Edge case: Flip with multiple partial fills**
- Order Sell 15 on Long 10 position
- Fill 1: Sell 5 @ $50k (Long 10 → Long 5)
- Fill 2: Sell 10 @ $49k (Long 5 → Short 5)
- Realized PnL calculated at flip point (fill 2, first 5 units)

### 2.4 Liquidation vs. Normal Orders

| Field | Normal Order | Liquidation Order |
|-------|--------------|-------------------|
| Margin check | Required | Skipped (RISK.md §6:251) |
| Frozen margin | Reserved | None |
| Reduce-only | Optional | Implicit (always closes) |
| Priority | Normal FIFO | Normal FIFO (no special) |

**Edge case: Liquidation order fails**
- Liquidation placed as normal order (is_liquidation=true)
- No margin check, but can still fail (invalid tick/lot)
- If fails: liquidation round incomplete, retry next round

**Risk engine (RISK.md §6:250-254):**
```rust
if order.is_liquidation:
    route order to matching engine
    return  // no frozen margin, no margin check
```

## 3. Matching Engine Edge Cases

### 3.1 Tick Size Validation

**Constant tick size (v1):** Each symbol has single tick_size.
All prices must be multiple of tick_size.

| Price Input | Tick Size | Validation | Outcome |
|-------------|-----------|------------|---------|
| 5000000 | 1 | 5000000 % 1 = 0 | ACCEPT |
| 5000001 | 1 | 5000001 % 1 = 0 | ACCEPT |
| 5000001 | 10 | 5000001 % 10 = 1 | REJECT: InvalidTickSize |
| 5000000 | 1000 | 5000000 % 1000 = 0 | ACCEPT |

**Validation (rsx-types/src/lib.rs:92):**
```rust
price.0 % config.tick_size == 0
```

**Edge case: Tick size changes**
- ME polls metadata store every 10 minutes (METADATA.md §4)
- New config applied: tick_size 1 → 10
- Old orders at tick=1 now invalid for NEW orders
- Existing resting orders STAY (grandfathered)

**Solution (TESTING-BOOK.md:203):**
```rust
// On config change, existing orders remain valid
// New orders validated against new tick_size
```

### 3.2 Lot Size Validation

**Same logic as tick size, applied to quantities.**

| Qty Input | Lot Size | Validation | Outcome |
|-----------|----------|------------|---------|
| 1000 | 1 | 1000 % 1 = 0 | ACCEPT |
| 1500 | 1000 | 1500 % 1000 = 500 | REJECT: InvalidLotSize |
| 1000000 | 1000 | 1000000 % 1000 = 0 | ACCEPT |

**Edge case: Lot size decrease during open order**
- User places order qty=1500, lot_size=500 (valid)
- Config updates: lot_size 500 → 1000
- Order becomes invalid under new rules but stays in book

### 3.3 Order Deduplication

**Deduplication key:** order_id (UUIDv7, 16 bytes).

| Scenario | First Submission | Duplicate Submission | Outcome |
|----------|------------------|----------------------|---------|
| Exact duplicate | order_id=X | same order_id=X | REJECT: DuplicateOrderId |
| Retry with new ID | order_id=X | order_id=Y (new) | ACCEPT: new order |
| Duplicate after cancel | CANCEL(X), then NEW(X) | Within 5min window | REJECT: X still in map |
| Duplicate after 5min | NEW(X) → ORDER_DONE | 6 min later: NEW(X) | ACCEPT: X pruned from map |

**Dedup window: 5 minutes (MESSAGES.md §462)**
```rust
// FxHashMap<OrderId, OrderHandle>
// Pruning queue: remove entries older than 5min
```

**Edge case: ME restart**
- Dedup map cleared on restart
- Duplicate order_id in first 5min undetected
- Accepted: UUIDv7 collisions effectively impossible

**Post-cancel dedup (MESSAGES.md §396-427):**
```rust
// After CANCEL, mark as cancelled in map (don't remove)
order.is_active = false;
// Add to pruning queue, remove after 5min
pruning_queue.push((order_id, now));
```

### 3.4 Cancel Edge Cases

| Scenario | Cancel Request | Order State | Outcome |
|----------|----------------|-------------|---------|
| Valid cancel | cid or oid | Resting in book | CANCELLED: removed |
| Already filled | oid=X | Filled 1ms ago | REJECT: not found |
| Already cancelled | CANCEL(X) twice | First succeeds | REJECT: already cancelled |
| Never existed | oid=Y (typo) | No such order | REJECT: not found |
| Wrong user | user_id=42 cancels user_id=99 order | Auth mismatch | REJECT: not your order |

**Cancel-by-cid vs cancel-by-oid:**
- cid: 20-char string, client-chosen
- oid: 32-char hex UUIDv7, server-assigned
- Gateway pending map: lookup by either key

**Edge case: Cancel arrives before order**
- Network reorder: CANCEL sent, arrives before NEW
- ME has no order to cancel yet
- REJECT: order not found
- User retries, may double-cancel

### 3.5 IOC/FOK Edge Cases

**IOC (Immediate-Or-Cancel):**
- Match as much as possible immediately
- Cancel any unfilled remainder
- 0+ fills, then ORDER_DONE(CANCELLED) if partial

| Book State | IOC Order | Result |
|------------|-----------|--------|
| Ask $50k: 10 BTC | Buy 5 @ $50k | Fill 5, ORDER_DONE(FILLED) |
| Ask $50k: 10 BTC | Buy 15 @ $50k | Fill 10, ORDER_DONE(CANCELLED, filled=10, remaining=5) |
| Ask $51k: 10 BTC | Buy 10 @ $50k | No fill, ORDER_DONE(CANCELLED, filled=0, remaining=10) |

**FOK (Fill-Or-Kill):**
- Match entire quantity immediately or cancel all
- All-or-nothing, no partial fills

| Book State | FOK Order | Result |
|------------|-----------|--------|
| Ask $50k: 10 BTC | Buy 10 @ $50k | Fill 10, ORDER_DONE(FILLED) |
| Ask $50k: 10 BTC | Buy 15 @ $50k | No fill, ORDER_DONE(CANCELLED, filled=0) |
| Ask $50k: 5 BTC, $51k: 10 BTC | Buy 10 @ $51k | Fill 5+5=10, ORDER_DONE(FILLED) |

**Implementation:** Matching algorithm checks after matching loop
(rsx-book/src/matching.rs, IOC/FOK implemented).

### 3.6 Self-Trade Prevention

**v1 behavior:** Self-trades ALLOWED.

| Scenario | Behavior |
|----------|----------|
| User 42 resting bid, User 42 incoming sell | Match executes, both sides filled |
| Same user, same symbol | Two separate order_ids, both filled |

**Rationale:** Self-trade prevention adds complexity (cancel maker?
cancel taker? both?). Defer to v2.

**User responsibility:** Don't place opposing orders if self-trade
is undesired.

## 4. Cross-Layer Edge Cases

### 4.1 Config Sync Race

**Scenario:** Config update in flight, components have stale cache.

```
T=0: Metadata DB updates tick_size 1 → 10
T=1: ME polls, applies new config, emits CONFIG_APPLIED
T=2: Gateway receives CONFIG_APPLIED, updates cache
T=3: Risk receives CONFIG_APPLIED, updates cache
```

**Race window (T=1 to T=3):**
- Gateway validates with old tick_size=1
- Order passes Gateway validation
- ME rejects with new tick_size=10

**Outcome:** ORDER_FAILED(INVALID_TICK_SIZE) from ME.

**v1 acceptance:** Race is rare (10min poll interval), user retries.
Gateway MAY log mismatch as warning.

### 4.2 Margin Check vs. Fill Race

**Scenario:** Margin check passes, but position fills before order
placed, margin now insufficient.

```
T=0: User equity=1000, no positions, available=1000
T=1: Gateway checks margin for order (need 500), passes
T=2: Existing order fills, user now has position, equity drops to 600
T=3: Gateway sends new order to Risk
T=4: Risk re-checks margin, available now 100, needed 500
T=5: Risk rejects order: InsufficientMargin
```

**Solution:** Risk is authoritative, always re-checks at ingress.
Gateway validation is advisory only.

### 4.3 Frozen Margin Leak

**Scenario:** Order placed, margin frozen, ORDER_DONE lost.

```
T=0: User places order, Risk freezes 500 margin
T=1: ME processes order, sends ORDER_DONE(FILLED)
T=2: Network drops ORDER_DONE packet (UDP loss)
T=3: Risk never receives ORDER_DONE, frozen stays at 500
T=4: User tries to place another order, available=0 (all frozen)
```

**v1 mitigation:**
- Gateway timeout (10s), returns error to user
- User sees "order timeout" but order may be filled
- Frozen margin stuck until manual operator adjustment

**v2 planned:** Periodic reconciliation, Gateway queries ME for
order status, releases frozen on confirmed completion.

### 4.4 Price Precision Loss

**Scenario:** User submits float price, conversion loses precision.

```
User input: $50000.123456789 (9 decimals)
tick_size: $0.01 (2 decimals)
Conversion: 50000.123456789 / 0.01 = 5000012.3456789
Cast to i64: 5000012
Display back: $50000.12 (lost .003456789)
```

**Impact:** User intended $50000.12345, got $50000.12.
If tick_size permits finer granularity, precision lost.

**Solution:** Gateway MUST document precision limits. For
tick_size=0.01, only 2 decimal places honored.

## 5. Validation Order & Responsibility

### Layer Responsibilities

| Layer | Validates | Rejects | Notes |
|-------|-----------|---------|-------|
| Gateway | Field types, rate limits, basic sanity | Parse errors, rate limit, queue full | Fast path, no state |
| Risk | Margin, position limits, reduce-only | InsufficientMargin, ReduceOnlyViolation | Stateful, user context |
| ME | Tick/lot size, dedup, symbol existence | InvalidTickSize, InvalidLotSize, DuplicateOrderId | Authoritative, per-symbol |

### Validation Sequence (Full Order Flow)

```
1. Gateway parse (WebSocket → struct)
   ├─ Malformed JSON → {E:[1002, "parse error"]}
   ├─ Invalid UTF-8 → {E:[1001, "invalid utf8"]}
   └─ Missing fields → {E:[1002, "missing field"]}

2. Gateway rate limit
   ├─ Per-IP: 100/s → {E:[1006, "rate limited"]}
   ├─ Per-user: 10/s → {E:[1006, "rate limited"]}
   └─ Per-instance: 1000/s → {E:[5, "overloaded"]}

3. Gateway field validation (advisory)
   ├─ price ≤ 0 → {E:[...]}
   ├─ qty ≤ 0 → {E:[...]}
   ├─ symbol cache miss → {E:[2, "symbol not found"]}
   └─ (tick/lot validation SHOULD happen here, MAY skip to ME)

4. Risk pre-trade check
   ├─ is_liquidation → skip margin, forward to ME
   ├─ reduce_only → skip margin, forward to ME
   ├─ calculate portfolio margin → if insufficient: {U:[...,FAILED,4]}
   └─ freeze margin, forward to ME

5. ME validation (authoritative)
   ├─ symbol_id bounds check → ORDER_FAILED(SYMBOL_NOT_FOUND)
   ├─ tick size: price % tick_size → ORDER_FAILED(INVALID_TICK_SIZE)
   ├─ lot size: qty % lot_size → ORDER_FAILED(INVALID_LOT_SIZE)
   ├─ dedup: order_id in map → ORDER_FAILED(DUPLICATE_ORDER_ID)
   └─ match, emit fills + ORDER_DONE

6. Risk fill processing
   ├─ update position
   ├─ release frozen margin (on ORDER_DONE)
   └─ recalc margin, check liquidation

7. Gateway response
   ├─ forward fills to user
   └─ remove from pending tracking (on ORDER_DONE/FAILED)
```

## 6. Testing Strategy

### Unit Tests (per component)

**Gateway:**
- Invalid field types (negative price, zero qty)
- Rate limit enforcement (per-user, per-IP, global)
- UTF-8 handling in cid (truncation, invalid bytes)
- Config cache lookup (valid, missing symbol)

**Risk:**
- Notional overflow (MAX price * MAX qty)
- Available margin edge cases (exact match, 1 unit short)
- Position flip (long → short, short → long)
- Frozen margin accumulation/release

**ME:**
- Tick/lot size validation (valid multiples, invalid)
- Deduplication (exact duplicate, after 5min)
- IOC/FOK matching (full, partial, no fill)
- Config update (tick/lot change during open orders)

### Integration Tests

**Config sync race:**
- ME applies config, Gateway lags, order rejected by ME
- Verify ORDER_FAILED propagates to user

**Margin check race:**
- Position fills between Gateway check and Risk check
- Verify Risk rejects with latest state

**Frozen margin leak:**
- Simulate ORDER_DONE loss, verify timeout error
- Manual frozen adjustment required (operator tool)

### Edge Case Test Suite (TESTING-GATEWAY.md, TESTING-RISK.md)

Each validation edge case gets a dedicated test:
```
tick_size_validation_rejects_early     // Gateway pre-check
tick_size_validation_rejects_at_me     // ME authoritative check
lot_size_edge_case_zero_after_rounding // Sub-lot rounds to 0
notional_overflow_i64_max              // price * qty > 2^63
position_flip_partial_fills            // Multiple fills across flip
frozen_margin_accumulates_correctly    // Sum across orders
dedup_after_5min_window_allows         // Pruning works
```

## 7. Monitoring & Alerts

### Metrics to Track

**Rejection rate by reason:**
- InvalidTickSize: >1% → investigate Gateway validation gap
- InvalidLotSize: >1% → investigate Gateway validation gap
- DuplicateOrderId: >0.1% → investigate client retry logic
- InsufficientMargin: >5% → normal, track for user education

**Config sync lag:**
- Time between CONFIG_APPLIED emission and Gateway/Risk receipt
- Alert if lag >1s (expect <100ms)

**Frozen margin leak:**
- Sum(frozen_margin) vs. active orders count
- Alert if frozen > 2x active orders (indicates leak)

**Validation mismatch:**
- Gateway accepts, ME rejects: count by reason
- High rate → Gateway validation not matching ME

## Cross-References

- WEBPROTO.md: Wire format, field types, error codes
- MESSAGES.md: Order flow sequences, completion guarantees
- RISK.md: Margin calculations, position tracking, pre-trade checks
- ORDERBOOK.md: Tick/lot size validation, matching algorithm
- METADATA.md: Config scheduling, CONFIG_APPLIED propagation
- TESTING-GATEWAY.md: Gateway validation tests
- TESTING-RISK.md: Risk engine validation tests
- TESTING-MATCHING.md: ME validation tests
