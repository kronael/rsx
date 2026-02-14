# Liquidator Specification

## Table of Contents

- [Context](#context)
- [1. Liquidation State](#1-liquidation-state)
- [2. Backoff Schedule](#2-backoff-schedule)
- [3. Order Generation](#3-order-generation)
- [4. Lifecycle](#4-lifecycle)
- [5. Margin Recovery (Cancellation)](#5-margin-recovery-cancellation)
- [6. Frozen Margin Interaction](#6-frozen-margin-interaction)
- [7. Main Loop Integration](#7-main-loop-integration)
- [8. Persistence](#8-persistence)
- [9. Config](#9-config)
- [10. Edge Cases and Boundary Conditions](#10-edge-cases-and-boundary-conditions)
- [11. Performance Targets](#11-performance-targets)
- [12. File Organization](#12-file-organization)
- [13. Tests](#13-tests)

---

## Context

The liquidator is embedded in the risk engine (like the funding
engine in [RISK.md](RISK.md) section 5). Triggered by
`enqueue_liquidation()` from per-tick margin recalc
([RISK.md](RISK.md) section 7). Not a sidecar. It generates
reduce-only limit orders that chase the market with progressive
slippage and linear backoff.

## 1. Liquidation State

```rust
struct LiquidationState {
    user_id: u32,
    symbol_id: u32,          // KEY: per-(user,symbol), not per-user
    round: u32,              // current round (1-indexed)
    enqueued_at_ns: u64,
    last_order_ns: u64,      // when last order was placed
    status: LiqStatus,
}

enum LiqStatus {
    Pending,    // enqueued, not yet active
    Active,     // currently liquidating
    Done,       // completed or cancelled
}
```

**Implementation note**: The current implementation tracks
liquidation state per (user_id, symbol_id) pair, NOT per user.
This means each symbol's position liquidates independently with
its own round counter and delay timer. A user with positions in
multiple symbols will have multiple `LiquidationState` entries.
This differs from the original spec's `pending_orders` approach
but is simpler and avoids coordinating rounds across symbols.

In-memory `Vec<LiquidationState>` on the shard. Small number
of concurrent liquidations expected (<100), so linear scan is
acceptable. If liquidation volume increases, migrate to
`FxHashMap<(u32, u32), LiquidationState>` keyed by
(user_id, symbol_id).

## 2. Backoff Schedule

Linear delay, quadratic slippage:

| Round | Delay | Slippage |
|-------|-------|----------|
| 1 | 1s | 1 bp |
| 2 | 2s | 4 bp |
| 3 | 3s | 9 bp |
| n | n * base_delay_ns | n^2 * base_slip_bps |

Configurable `base_delay_ns`, `base_slip_bps`, cap at
`max_slip_bps`. Continues until user is above maintenance
margin or all positions closed.

## 3. Order Generation

Per-position limit orders. Price determined by fallback chain:

1. **Mark price** (primary): from mark price aggregator
2. **Index price** (fallback): BBO-derived index from risk engine
3. **Last known mark price** (final fallback): most recent valid
   mark price cached by risk engine

The liquidator NEVER stalls waiting for price. If all sources
are unavailable, it uses the last known mark price (which may
be stale but prevents indefinite liquidation delay).

Each open position gets a closing order at
`price_source +/- round^2 * base_slip_bps`:

- Long position -> sell at `price - slippage`
- Short position -> buy at `price + slippage`

Order properties:
- `reduce_only = true` (NewOrder field 8, ME enforces)
- `is_liquidation = true` (RiskNewOrder field 11, risk skips
  margin check)
- ME clamps qty to position size via position tracking
- Routed to ME via same CMP/UDP link as normal orders

## 4. Lifecycle

```
enqueue_liquidation(user_id)
  -> if already in liquidation: skip (no re-enqueue)
  -> cancel all user's pending non-liquidation orders
     (releases frozen margin, may restore margin)
  -> re-check margin: if recovered -> done, no liquidation
  -> create LiquidationState { round=1,
     next_round_at_ns=now+base_delay_ns }
  -> place round-1 orders (1bp slippage)

on_fill(fill) for liquidation order:
  -> update position (normal fill path)
  -> remove from pending_orders
  -> re-check margin: if above maintenance -> cancel remaining,
     set status=Cancelled
  -> if all positions closed -> status=Completed

on_order_done(order) for liquidation order:
  -> remove from pending_orders (fully filled handled above)

on_order_failed(order) for liquidation order:
  -> symbol halted: pause liquidation for that symbol
  -> otherwise: treat as unfilled, escalate next round

maybe_process_liquidations() [called each main loop iteration]:
  -> for each Active liquidation where now >= next_round_at_ns:
    -> cancel all pending unfilled orders for this user
    -> re-check margin: if recovered -> Cancelled, continue
    -> increment round
    -> compute new slippage: min(round^2 * base_slip_bps,
       max_slip_bps)
    -> compute new delay: round * base_delay_ns
    -> place new orders at new slippage
    -> next_round_at_ns = now + delay
```

## 5. Margin Recovery (Cancellation)

Re-check margin at every opportunity:
- After each fill on a liquidation order
- At round escalation
- On fresh price tick (mark or BBO) for user in liquidation

If `equity >= maintenance_margin`: cancel all pending liquidation
orders, remove from liquidation map, set status=Cancelled.

## 6. Frozen Margin Interaction

- Cancel all user's pending non-liquidation orders on entering
  liquidation (releases frozen margin, may restore margin)
- Liquidation orders do NOT freeze margin (user is already
  underwater, no point reserving)
- Reject new non-liquidation orders from user while in
  liquidation (`order_while_user_being_liquidated_rejected`)

## 7. Main Loop Integration

Added between funding check and lease renewal in
[RISK.md](RISK.md) main loop:

```
    // 5.5. Liquidation processing
    maybe_process_liquidations()
```

Same pattern as `maybe_settle_funding()`.

## 8. Persistence

Postgres table (append-only):

```sql
CREATE TABLE liquidation_events (
    user_id       INT NOT NULL,
    symbol_id     INT NOT NULL,
    round         INT NOT NULL,
    side          SMALLINT NOT NULL,
    price         BIGINT NOT NULL,
    qty           BIGINT NOT NULL,
    slippage_bps  INT NOT NULL,
    status        SMALLINT NOT NULL, -- 0=placed, 1=filled, 2=cancelled
    timestamp_ns  BIGINT NOT NULL,
    inserted_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Written via same write-behind worker as positions/fills.

## 9. Config

```
RSX_LIQUIDATION_BASE_DELAY_NS=1000000000
RSX_LIQUIDATION_BASE_SLIP_BPS=1
RSX_LIQUIDATION_MAX_SLIP_BPS=500
RSX_LIQUIDATION_MAX_ROUNDS=50
```

**Post-max-rounds behavior:** After `max_rounds` is reached,
liquidation continues at `max_slip_bps` (100% slippage cap
from config) with no further delay between rounds. If the
position still cannot be closed (no counterparty at any
price), the remaining loss is socialized via the insurance
fund. Insurance fund deduction is logged as a
`liquidation_events` row with `status = 3` (socialized).

## 10. Edge Cases and Boundary Conditions

### 10.1. Round Progression Edge Cases

**First order fires immediately**: When `enqueue_liquidation()`
is called, the first order (round 1) is placed immediately
without delay. `last_order_ns` is initially 0, so the first
`maybe_process()` call will always generate an order.

**Delay calculation for subsequent rounds**: Round N requires
`N * base_delay_ns` to have elapsed since `last_order_ns`.
For example, with `base_delay_ns=1s`:
- Round 1: immediate (last_order_ns=0)
- Round 2: waits 2s from round 1's timestamp
- Round 3: waits 3s from round 2's timestamp

**Max rounds boundary**: When `round > max_rounds`, the engine
emits a `SocializedLoss` event and marks the liquidation Done.
No further orders are placed. The inequality is strict (>),
so round `max_rounds` still generates a normal order.

**Slippage overflow protection**: With large rounds and high
`base_slip_bps`, the slippage calculation `round^2 * base_slip_bps`
could overflow. However, the config caps rounds at 50 and
`max_slip_bps` at 500, so maximum slippage is `50^2 * 1 = 2500 bps`
before the cap applies. This fits comfortably in i64.

**Price calculation underflow**: For long positions, the sell
price is `mark * (10_000 - slip) / 10_000`. If `slip >= 10_000`,
this could produce zero or negative prices. The `max_slip_bps`
cap prevents this: capped at 500 bps (5%), the worst case is
95% of mark price.

### 10.2. Position and Price Edge Cases

**Zero position during liquidation**: If position becomes zero
(all fills processed) before the next round, `maybe_process()`
sets status to Done and skips order generation. This can happen
from:
- Liquidation orders being fully filled
- User manually closing via other orders (if allowed pre-liquidation)
- Funding settlement changing position (rare, funding doesn't
  typically close positions)

**Mark price unavailable**: If `get_mark_fn()` returns 0
(no mark price available), `maybe_process()` skips that round
without incrementing the round counter. The liquidation pauses
until mark price is available again. This prevents placing
orders at nonsensical prices during price oracle outages.

**Mark price returns after being unavailable**: When mark price
becomes available again (returns non-zero), the next
`maybe_process()` iteration continues from the current round.
The delay timer is NOT reset -- if enough time has passed during
the price outage, multiple rounds may fire rapidly to catch up.

**Negative mark price**: Not possible in the current design --
mark prices for perpetuals are always positive. If a malformed
mark price somehow produces a negative value, the sign is NOT
checked, and the liquidation order would have a negative price
(rejected by ME). This is a data integrity issue, not a
liquidation engine issue.

### 10.3. Multiple Symbols and Concurrent Liquidations

**Multiple positions per user**: The spec describes
`pending_orders: ArrayVec<PendingLiqOrder, MAX_SYMBOLS>` in §1,
but the current implementation liquidates per (user_id, symbol_id)
pair, not per user. Each symbol's position is an independent
`LiquidationState` entry. This means:
- User with long BTC and short ETH gets two separate liquidation
  states
- Each progresses through rounds independently
- Round timers are per-symbol, not per-user

**Round synchronization across symbols**: There is NONE. If a
user's BTC position is enqueued at t=0 (round 1) and ETH at
t=2s (round 1), they remain out of phase. BTC reaches round 5
while ETH is at round 3. This is intentional -- liquidating
one symbol may restore margin without needing to liquidate
others.

**Partial recovery (one symbol closes, others continue)**: If
liquidating BTC+ETH and the BTC position fully closes first,
only BTC's liquidation moves to Done. ETH continues liquidating
unless margin is restored. The risk engine's margin recalc
(called after each fill) determines if the user is still
underwater.

**Margin recovery cancels ALL symbols**: When
`cancel_if_recovered(user_id, symbol_id)` is called from the
risk engine, it removes only that (user_id, symbol_id) pair.
However, the risk engine must call it for each symbol if margin
is restored. The spec in §5 says "cancel all pending liquidation
orders" -- this is the risk engine's responsibility, not the
liquidation engine's. The liquidation engine is per-symbol only.

### 10.4. Timing and Concurrency

**Clock skew or time going backwards**: The liquidation engine
assumes monotonic `now_ns`. If time goes backwards (NTP
adjustment), the delay check `now_ns < last_order_ns + delay`
may never trigger, stalling liquidation indefinitely. Use
monotonic clock sources (CLOCK_MONOTONIC on Linux) for `now_ns`.

**Rapid-fire `maybe_process()` calls**: Calling `maybe_process()`
in a tight loop with the same `now_ns` is safe. Once an order
is placed (round incremented, `last_order_ns` updated), the
delay check prevents re-firing until the delay elapses. No
order duplication.

**Interleaved enqueue and process**: Enqueueing a new liquidation
while processing existing ones is safe. The new entry is appended
to `active`, and the iteration over `active` is non-borrowing
(mutable iteration). Rust borrow checker enforces this.

**Order fills arriving during round processing**: Fills are
processed by the risk engine, which updates positions via the
position map. The liquidation engine reads positions via
`get_position_fn()` callback. If a fill arrives between two
`maybe_process()` calls, the next call sees the updated position.
No race -- single-threaded risk shard.

### 10.5. Order Lifecycle Edge Cases

**Order immediately filled before next round**: If a liquidation
order is fully filled before the next `maybe_process()` call,
the position becomes zero (or reduced), and the next call either
generates a smaller order (partial fill) or marks Done (full fill).
The round counter still increments as if the order wasn't filled,
increasing slippage. This is intentional -- aggressive slippage
escalation.

**Order rejected by ME (symbol halted)**: Per §4, if
`on_order_failed()` is called with reason=symbol_halted, the
liquidation for that symbol pauses (implementation TODO). The
current code does not distinguish halted from other failures.

**Order rejected by ME (other reasons)**: Treated as unfilled.
The next round fires after the delay, with higher slippage. No
special handling.

**Order partially filled, then cancelled**: If the risk engine
cancels pending liquidation orders (e.g., during round escalation
or margin recovery), partially filled orders have already updated
the position. The next round sees the reduced position and
generates a smaller order.

**Order cancelled by risk engine, position still open**: On
round escalation (§4), all pending unfilled orders are cancelled.
If position is still open, a new order is placed at higher
slippage. The fill race (order fills between cancel and new
order) is benign -- position update happens first, new order
uses updated position.

### 10.6. Socialization Edge Cases

**Socialized loss when round > max_rounds**: The engine emits
`SocializedLoss` and marks Done. The insurance fund deduction
is the responsibility of the risk engine, not the liquidation
engine. The liquidation engine only records the event.

**Multiple symbols reaching max_rounds**: Each symbol's
liquidation independently reaches max_rounds. If a user has
BTC and ETH both past max_rounds, two `SocializedLoss` events
are emitted. The risk engine must aggregate these for insurance
fund deduction.

**Zero mark price at max_rounds**: If mark price is unavailable
(returns 0) when round > max_rounds, the code uses `price = mark`
(which is 0) for the `SocializedLoss` event. This is a data
integrity issue. Insurance fund calculations must handle
zero-price socialized loss (reject or use last known price).

**Negative PnL during socialization**: Socialized loss is always
a loss to the exchange/insurance fund. The remaining position
qty is recorded, but the PnL calculation (position * price) is
not performed by the liquidation engine. The risk engine must
compute the actual USD loss from the socialized position.

### 10.7. Configuration Edge Cases

**base_delay_ns = 0**: All rounds fire immediately (no delay).
This is valid for testing or high-urgency liquidation policies.
Round counter still increments, slippage still escalates.

**base_slip_bps = 0**: No slippage at any round. Orders placed
at mark price exactly. Valid for zero-slippage liquidation
(market-making CEX with deep order books).

**max_slip_bps = 0**: Slippage immediately capped at zero. Same
as `base_slip_bps = 0` but enforced via cap. Edge case: if
`base_slip_bps > 0` but `max_slip_bps = 0`, all rounds place
orders at mark price.

**max_rounds = 0**: First round (round 1) places an order, then
immediately exceeds max_rounds. The check is `round > max_rounds`,
so round 1 is allowed even if max_rounds=0. After round 1,
`round=2 > 0` triggers socialization. This is likely a
misconfiguration; recommend `max_rounds >= 1`.

**max_rounds = 1**: Round 1 places an order. After delay, round
2 exceeds max_rounds, triggers socialization. Only one liquidation
attempt before socialization.

**Extreme slippage values**: With `base_slip_bps=500` and no cap,
round 10 would be `10^2 * 500 = 50000 bps = 500%`. For a long
position, sell price would be `mark * (10_000 - 50_000) / 10_000
= mark * (-4)`, producing a negative price. Always set
`max_slip_bps` to prevent this. Recommended cap: 500 bps (5%)
or 1000 bps (10%) for extreme volatility.

## 11. Performance Targets

| Operation | Target |
|-----------|--------|
| Enqueue check | <100ns |
| Order generation per position | <500ns |
| Round escalation per user | <1us |
| 100-user cascade processing | <100us |

## 12. File Organization

```
crates/rsx-risk/src/
    liquidation.rs  -- LiquidationEngine, state, order gen
```

Added to existing risk crate, same as `funding.rs`.

## 13. Tests

Tests: see [TESTING-LIQUIDATOR.md](TESTING-LIQUIDATOR.md) for
complete unit tests, e2e tests, integration tests, benchmarks,
correctness invariants, and integration points.
