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

Per-position limit orders. Each open position gets a closing
order at `mark_price +/- round^2 * base_slip_bps`:

- Long position -> sell at `mark_price - slippage`
- Short position -> buy at `mark_price + slippage`

Order properties:
- Reduce-only flag (ME enforces, never opens new position)
- Skip pre-trade margin check (user is already underwater)
- Liquidation flag so risk engine doesn't reject them
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

```toml
[liquidation]
base_delay_ns = 1_000_000_000   # 1s
base_slip_bps = 1               # 1bp
max_slip_bps = 500              # 5% cap
max_rounds = 50
```

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

### Unit Tests (liquidation.rs)

```rust
// core
enqueue_user_starts_liquidation
round_delay_increases_linearly
round_slippage_increases_quadratically
slippage_round_1_is_1bp
slippage_round_2_is_4bp
slippage_round_3_is_9bp
slippage_round_10_is_100bp
limit_order_price_sell_below_mark
limit_order_price_buy_above_mark
multiple_positions_all_get_orders
partial_fill_reduces_position
full_fill_closes_position
user_recovers_cancels_liquidation
max_slippage_cap_enforced
max_rounds_clamp_slippage

// edge cases
user_already_in_liquidation_not_re_enqueued
user_deposit_during_liquidation_restores_margin
price_recovery_cancels_liquidation
zero_qty_after_fill_completes_liquidation
long_and_short_positions_both_liquidated
long_position_gets_sell_order
short_position_gets_buy_order
order_not_filled_escalates_next_round
round_timer_not_reset_on_partial_fill
slippage_calc_no_overflow_at_high_rounds
pending_non_liq_orders_cancelled_on_entry
frozen_margin_released_on_entry
liquidation_orders_skip_margin_check
new_orders_rejected_during_liquidation
liquidation_order_done_no_frozen_release
mark_price_update_rechecks_liquidating_users
bbo_update_rechecks_liquidating_users
empty_position_skipped_no_order
single_position_single_order
cancel_unfilled_on_round_escalation
```

### E2E Tests (shard-level)

```rust
// core
price_drop_triggers_liquidation_closes_position
gradual_price_drop_multiple_rounds_increasing_slippage
price_recovery_mid_liquidation_cancels
liquidation_partial_fill_then_full_close
liquidation_across_multiple_symbols
liquidation_interleaved_with_normal_orders
liquidation_with_funding_settlement_concurrent

// cascade / stress
cascade_10_users_all_liquidated
cascade_100_users_all_liquidated
cascade_mixed_some_recover_some_closed
liquidation_orders_match_against_resting

// ordering
fills_processed_before_liquidation_check
liquidation_orders_after_normal_orders
round_escalation_timing_accurate

// interaction
liquidation_then_deposit_cancels
liquidation_user_order_rejected
partial_fill_restores_margin_cancels_rest
```

### Integration Tests (Postgres)

```rust
liquidation_events_persisted_on_flush
liquidation_recovery_after_crash
liquidation_state_rebuilt_from_positions
concurrent_liquidation_and_funding_persist
```

### Benchmarks

```rust
bench_enqueue_liquidation           // target <100ns
bench_generate_orders_1_position    // target <500ns
bench_generate_orders_10_positions  // target <5us
bench_round_escalation              // target <1us
bench_cascade_100_users             // target <100us
bench_margin_recheck_during_liq     // target <10us/user
```
