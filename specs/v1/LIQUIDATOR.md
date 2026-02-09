# Liquidator Specification

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
    round: u32,              // current round (1-indexed)
    enqueued_at_ns: u64,
    next_round_at_ns: u64,   // when to escalate
    pending_orders: ArrayVec<PendingLiqOrder, MAX_SYMBOLS>,
    status: LiqStatus,
}

enum LiqStatus {
    Active,
    Cancelled,  // margin recovered
    Completed,  // all positions closed
}

struct PendingLiqOrder {
    symbol_id: u32,
    side: u8,         // opposite of position side
    qty: i64,         // full position qty
    price: i64,       // mark +/- slippage
    order_seq: u64,   // track in ME
}
```

In-memory `FxHashMap<u32, LiquidationState>` on the shard
(user_id is sparse/hashed, same as positions and accounts).
No per-symbol map inside -- just iterate user's positions from
the existing `FxHashMap<(user_id, symbol_id), Position>` and
build orders directly. `pending_orders` is a small fixed-capacity
array (user can't have more positions than MAX_SYMBOLS).

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
- Routed to ME via same SPSC ring as normal orders

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

## 10. Performance Targets

| Operation | Target |
|-----------|--------|
| Enqueue check | <100ns |
| Order generation per position | <500ns |
| Round escalation per user | <1us |
| 100-user cascade processing | <100us |

## 11. File Organization

```
crates/rsx-risk/src/
    liquidation.rs  -- LiquidationEngine, state, order gen
```

Added to existing risk crate, same as `funding.rs`.

## 12. Tests

Tests: see [TESTING-LIQUIDATOR.md](TESTING-LIQUIDATOR.md) for
complete unit tests, e2e tests, integration tests, benchmarks,
correctness invariants, and integration points.
