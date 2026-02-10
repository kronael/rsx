# TESTING-LIQUIDATOR.md — Liquidation Engine Tests

Source spec: [LIQUIDATOR.md](LIQUIDATOR.md)

Module: `crates/rsx-risk/src/liquidation.rs`

Tests extracted from LIQUIDATOR.md §12 and expanded with
missing edge cases, requirements checklist, and integration
points.

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| L1 | Liquidation triggered when equity < maint_margin | §context |
| L2 | LiquidationState per user in FxHashMap | §1 |
| L3 | Linear delay: round * base_delay_ns | §2 |
| L4 | Quadratic slippage: round^2 * base_slip_bps | §2 |
| L5 | Slippage capped at max_slip_bps | §2 |
| L6 | Per-position limit orders at mark +/- slippage | §3 |
| L7 | Orders are reduce_only + is_liquidation | §3 |
| L8 | No re-enqueue if already in liquidation | §4 |
| L9 | Cancel non-liq orders on entry (release frozen) | §4, §6 |
| L10 | Re-check margin after cancel: may recover | §4 |
| L11 | Re-check margin after each fill | §4, §5 |
| L12 | Re-check margin at round escalation | §4 |
| L13 | Re-check on price tick for liquidating users | §5 |
| L14 | Cancel remaining orders if margin recovered | §5 |
| L15 | Liquidation orders skip margin check | §6 |
| L16 | Reject non-liq orders during liquidation | §6 |
| L17 | Liquidation orders do NOT freeze margin | §6 |
| L18 | Persistence: append-only liquidation_events table | §8 |
| L19 | Configurable base_delay, base_slip, max_slip | §9 |
| L20 | Max rounds configurable | §9 |
| L21 | Order failed (symbol halted): pause that symbol | §4 |
| L22 | Order failed (other): treat as unfilled, escalate | §4 |
| L23 | Status transitions: Active -> Cancelled or Completed | §1 |
| L24 | ME clamps qty to position size (reduce_only) | §3 |
| L25 | Orders routed via same CMP/UDP link as normal orders | §3 |
| L26 | Persisted via same write-behind worker as fills | §8 |
| L27 | First order fires immediately (last_order_ns=0) | §10.1 |
| L28 | Mark price=0 pauses round, no increment | §10.2 |
| L29 | Zero position during liq sets Done | §10.2 |
| L30 | Multiple symbols liquidate independently | §10.3 |
| L31 | Round timers per-symbol, not per-user | §10.3 |
| L32 | Monotonic clock assumed (no time backwards) | §10.4 |
| L33 | Rapid maybe_process calls safe (no dupe orders) | §10.4 |
| L34 | Slippage escalates even if orders filled | §10.5 |
| L35 | Socialized loss when round > max_rounds | §10.6 |
| L36 | base_delay_ns=0 fires all rounds immediately | §10.7 |
| L37 | max_rounds=0 allows round 1 then socializes | §10.7 |
| L38 | max_slip_bps caps prevent negative prices | §10.7 |

---

## Unit Tests

### Core

```rust
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
max_rounds_reached_stops_escalation
status_active_to_cancelled_on_recovery
status_active_to_completed_on_close
```

### Edge Cases

```rust
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
order_failed_symbol_halted_pauses_symbol
order_failed_other_escalates_next_round
order_seq_tracked_in_pending_orders
pending_orders_bounded_by_max_symbols
orders_routed_via_normal_spsc_ring
recheck_margin_on_round_before_placing
```

### Edge Cases from LIQUIDATOR.md §10

```rust
first_order_fires_immediately_no_delay
round_delay_calculation_cumulative_not_from_enqueue
max_rounds_boundary_strict_inequality
slippage_overflow_prevented_by_config_cap
price_calculation_no_underflow_with_max_slip
zero_position_marks_done_skips_order
mark_price_zero_pauses_round_no_increment
mark_price_recovery_continues_from_current_round
multiple_symbols_independent_round_timers
round_sync_not_enforced_across_symbols
partial_recovery_one_symbol_closes_others_continue
rapid_fire_maybe_process_no_duplicate_orders
order_immediately_filled_next_round_higher_slip
order_partially_filled_then_cancelled_uses_updated_position
socialized_loss_when_round_exceeds_max_rounds
multiple_symbols_reach_max_rounds_independent_events
zero_mark_price_at_max_rounds_recorded_as_zero
base_delay_zero_all_rounds_immediate
base_slip_zero_orders_at_mark_exactly
max_slip_zero_caps_all_rounds_to_mark
max_rounds_zero_allows_round_one_then_socializes
max_rounds_one_single_attempt_before_socialization
extreme_slippage_prevented_by_max_slip_cap
```

### Config

```rust
config_base_delay_ns_respected
config_base_slip_bps_respected
config_max_slip_bps_caps_slippage
config_max_rounds_limits_escalation
```

---

## E2E Tests

### Core

```rust
price_drop_triggers_liquidation_closes_position
gradual_price_drop_multiple_rounds_increasing_slippage
price_recovery_mid_liquidation_cancels
liquidation_partial_fill_then_full_close
liquidation_across_multiple_symbols
liquidation_interleaved_with_normal_orders
liquidation_with_funding_settlement_concurrent
liquidation_max_rounds_exhausted
liquidation_order_failed_symbol_halted
```

### Cascade / Stress

```rust
cascade_10_users_all_liquidated
cascade_100_users_all_liquidated
cascade_mixed_some_recover_some_closed
liquidation_orders_match_against_resting
```

### Ordering

```rust
fills_processed_before_liquidation_check
liquidation_orders_after_normal_orders
round_escalation_timing_accurate
```

### Interaction

```rust
liquidation_then_deposit_cancels
liquidation_user_order_rejected
partial_fill_restores_margin_cancels_rest
```

---

## Integration Tests (testcontainers Postgres)

```rust
liquidation_events_persisted_on_flush
liquidation_recovery_after_crash
liquidation_state_rebuilt_from_positions
concurrent_liquidation_and_funding_persist
liquidation_event_fields_match_schema
liquidation_persisted_via_write_behind_worker
```

---

## Benchmarks

```rust
bench_enqueue_liquidation           // target <100ns
bench_generate_orders_1_position    // target <500ns
bench_generate_orders_10_positions  // target <5us
bench_round_escalation              // target <1us
bench_cascade_100_users             // target <100us
bench_margin_recheck_during_liq     // target <10us/user (RISK.md)
```

Targets from LIQUIDATOR.md §10:

| Operation | Target |
|-----------|--------|
| Enqueue check | <100ns |
| Order generation per position | <500ns |
| Round escalation per user | <1us |
| 100-user cascade processing | <100us |

---

## Correctness Invariants

1. **No re-enqueue** -- user in liquidation is never re-enqueued
2. **All positions get orders** -- every open position generates a
   closing order per round
3. **Margin re-check at every opportunity** -- fill, round, tick
4. **Recovery cancels all** -- if margin recovered, all pending
   liquidation orders cancelled
5. **No frozen margin on liquidation orders** -- user already
   underwater
6. **Non-liq orders rejected** -- while user is in liquidation
7. **Status terminal** -- Cancelled and Completed are terminal states,
   no further rounds placed
8. **Slippage monotonic** -- slippage never decreases across rounds
   (capped at max_slip_bps)
9. **Order count bounded** -- pending_orders.len() <= MAX_SYMBOLS
10. **First order immediate** -- round 1 fires on first maybe_process,
    no delay (last_order_ns=0 special case)
11. **Price non-negative** -- with max_slip_bps cap, liquidation
    prices never negative (sell >= 0, buy > 0)
12. **Round monotonic** -- round number only increases, never
    decreases or resets during Active status
13. **Zero position terminal** -- if position becomes zero during
    liquidation, status moves to Done, no further orders
14. **Mark price stall pauses, not fails** -- zero mark price pauses
    liquidation without incrementing round or marking failed
15. **Symbol independence** -- multiple symbols for same user liquidate
    independently, each with own round timer and state

---

## Integration Points

- Embedded in risk engine main loop (RISK.md §main loop step 5.5)
- Triggered by per-tick margin recalc (RISK.md §7)
- Generates reduce_only + is_liquidation orders to ME via same
  CMP/UDP link as normal orders (LIQUIDATOR.md §3)
- ME clamps qty to position size via position tracking
  (ORDERBOOK.md §6.5)
- Fills processed by normal fill path in risk engine (RISK.md §1)
- Cancels non-liquidation orders on entry, releasing frozen
  margin (RISK.md §6, LIQUIDATOR.md §6)
- Events persisted via risk write-behind worker (RISK.md §persistence)
- Gateway notified via Q frame on private WS (WEBPROTO.md §Q)
- System-level: liquidation cascade under price crash
  (TESTING.md §6 load tests)

## Implementation Status (2026-02-10)

File: `rsx-risk/tests/liquidation_test.rs`

| Spec Test | Status | Location |
|-----------|--------|----------|
| enqueue_user_starts_liquidation | DONE | liquidation_test.rs:15 (enqueue_creates_active_state) |
| round_delay_increases_linearly | DONE | liquidation_test.rs:67 (maybe_process_respects_delay) |
| round_slippage_increases_quadratically | DONE | liquidation_test.rs:107 (maybe_process_escalates_slippage) |
| slippage_round_1_is_1bp | DONE | liquidation_test.rs:176 (maybe_process_order_price_with_slippage) |
| slippage_round_2_is_4bp | DONE | liquidation_test.rs:176 (covered in slippage test) |
| limit_order_price_sell_below_mark | DONE | liquidation_test.rs:149 (maybe_process_long_position_sells) |
| limit_order_price_buy_above_mark | DONE | liquidation_test.rs:158 (maybe_process_short_position_buys) |
| multiple_positions_all_get_orders | TODO | Need multi-position per user test |
| partial_fill_reduces_position | TODO | Need fill integration |
| full_fill_closes_position | TODO | Need fill integration |
| user_recovers_cancels_liquidation | DONE | liquidation_test.rs:232 (cancel_if_recovered_removes_active) |
| max_slippage_cap_enforced | DONE | liquidation_test.rs:107 (covered in escalation) |
| max_rounds_reached_stops_escalation | DONE | liquidation_test.rs:202 (maybe_process_marks_done_after_max_rounds) |
| status_active_to_cancelled_on_recovery | DONE | liquidation_test.rs:232 |
| status_active_to_completed_on_close | DONE | liquidation_test.rs:248 (remove_done_cleans_completed) |
| user_already_in_liquidation_not_re_enqueued | DONE | liquidation_test.rs:29 (enqueue_dedup_same_user_symbol) |
| zero_qty_after_fill_completes_liquidation | DONE | liquidation_test.rs:263 (zero_position_no_order) |
| long_position_gets_sell_order | DONE | liquidation_test.rs:149 |
| short_position_gets_buy_order | DONE | liquidation_test.rs:158 |
| liquidation_orders_skip_margin_check | DONE | margin_test.rs:393 (check_order_liquidation_order_skips_margin_check) |
| new_orders_rejected_during_liquidation | TODO | Need shard-level integration |
| empty_position_skipped_no_order | DONE | liquidation_test.rs:263 |
| maybe_process_generates_reduce_only_order | DONE | liquidation_test.rs:137 |
| multiple_users_independent_rounds | DONE | liquidation_test.rs:277 |
| price_drop_triggers_liquidation_closes_position | TODO | E2E |
| gradual_price_drop_multiple_rounds | TODO | E2E |
| cascade_10_users_all_liquidated | TODO | E2E |
| cascade_100_users_all_liquidated | TODO | E2E |
| liquidation_events_persisted_on_flush | TODO | Integration (Postgres) |
| liquidation_recovery_after_crash | TODO | Integration (Postgres) |
| order_failed_symbol_halted_pauses_symbol | TODO | Need ORDER_FAILED handling |
| order_failed_other_escalates_next_round | TODO | Need ORDER_FAILED handling |

Margin recalc tests in
`rsx-risk/tests/margin_recalc_test.rs`:

| Spec Test | Status | Location |
|-----------|--------|----------|
| margin_check_detects_undercollateralized | DONE | margin_recalc_test.rs:22 |
| margin_check_passes_healthy_account | DONE | margin_recalc_test.rs:36 |
| margin_check_borderline_not_liquidated | DONE | margin_recalc_test.rs:48 |
| exposure_index_tracks_users | DONE | margin_recalc_test.rs:61 |
| exposure_index_removes_on_close | DONE | margin_recalc_test.rs:72 |
