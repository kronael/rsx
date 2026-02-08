# TESTING-RISK.md — Risk Engine Tests

Source spec: [RISK.md](RISK.md)

Binary: `rsx-risk` (one process per user shard)

Tests extracted from RISK.md phases 1-5 and expanded with
missing edge cases, requirements checklist, and integration
points.

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| R1 | Fill ingestion: filter by shard, dedup by seq | §1 |
| R2 | Fee calculation on fill (taker + maker) | §1 |
| R3 | Position: long_qty, short_qty, entry_cost tracking | §2 |
| R4 | Account: collateral, frozen_margin tracking | §2 |
| R5 | Portfolio margin across all symbols per user | §3 |
| R6 | Exposure index: users with open positions per symbol | §3 |
| R7 | Index price: size-weighted mid from BBO | §4 |
| R8 | Mark price from DXS consumer (MARK.md) | §4 |
| R9 | Funding rate: f(mark, index), clamped, 8h interval | §5 |
| R10 | Pre-trade risk check before order to ME | §6 |
| R11 | Frozen margin reserved on order, released on done | §6 |
| R12 | Per-tick margin recalc for exposed users | §7 |
| R13 | Liquidation trigger: equity < maint_margin | §7 |
| R14 | Postgres persistence: write-behind 10ms flush | §persistence |
| R15 | Advisory lock: single writer per shard | §replication |
| R16 | Replica: buffer fills, apply on tip sync | §replication |
| R17 | Promotion: acquire lock, apply remaining, go live | §replication |
| R18 | Config updates from ME CONFIG_APPLIED events | §1 |
| R19 | Reduce-only/liquidation orders skip margin check | §6 |
| R20 | Main loop priority: fills > orders > mark > BBO | §main loop |

---

## Unit Tests

### Phase 1: Position + Margin Math (no I/O)

```rust
// position.rs -- core
apply_buy_fill_opens_long
apply_sell_fill_opens_short
apply_opposing_fill_reduces_position
apply_fill_closing_position_realizes_pnl
avg_entry_price_weighted_correctly
multiple_fills_same_side_accumulate
fill_larger_than_position_flips_side
zero_qty_after_exact_close

// position.rs -- edge cases
flip_long_to_short_single_fill
flip_short_to_long_single_fill
fill_at_same_price_no_pnl
realized_pnl_accumulates_across_fills
self_trade_taker_and_maker_same_user
max_qty_no_overflow
max_price_no_overflow
position_version_increments_per_fill

// margin.rs -- core
portfolio_margin_single_position
portfolio_margin_multi_symbol
portfolio_margin_long_short_offset
check_order_sufficient_margin_accepts
check_order_insufficient_margin_rejects
needs_liquidation_below_maintenance
needs_liquidation_above_maintenance_ok
frozen_margin_reserved_on_order
frozen_margin_released_on_done

// margin.rs -- edge cases
check_order_exactly_at_margin_limit_accepts
check_order_one_unit_over_limit_rejects
margin_with_zero_collateral_rejects_all
margin_with_no_positions_all_available
margin_unrealized_pnl_affects_equity
margin_mark_price_zero_handled
margin_max_leverage_enforced
frozen_margin_across_multiple_pending_orders
order_done_partial_fill_releases_remaining_frozen
order_failed_releases_all_frozen

// price.rs -- core
index_price_size_weighted_mid
index_price_balanced_book_equals_mid
index_price_imbalanced_favors_thicker_side

// price.rs -- edge cases
index_price_one_side_zero_qty
index_price_both_sides_zero_qty
index_price_max_values_no_overflow
index_price_spread_zero_equals_price

// funding.rs -- core
funding_rate_mark_above_index_positive
funding_rate_mark_below_index_negative
funding_rate_clamped_to_bounds
funding_payment_long_pays_when_positive
funding_payment_short_pays_when_negative
funding_zero_position_no_payment

// funding.rs -- edge cases
funding_rate_mark_equals_index_zero
funding_zero_sum_across_all_users
funding_with_position_opened_mid_interval
funding_extreme_divergence_clamped
funding_settlement_idempotent
funding_index_price_zero_handled

// exposure index -- core
exposure_add_user_on_fill
exposure_remove_user_on_close
exposure_no_duplicate_entries

// exposure index -- edge cases
exposure_user_in_multiple_symbols
exposure_close_one_symbol_keeps_others
exposure_symbol_idx_out_of_bounds_panics
exposure_empty_vec_for_unused_symbol
```

### Phase 2: Fill Ingestion + Main Loop (mocked rings)

```rust
// fill ingestion -- core
fill_for_shard_user_updates_position
fill_for_other_shard_ignored
fill_both_users_in_shard_updates_both
fill_dedup_by_seq
fill_advances_tip_per_symbol
tip_monotonic_never_decreases

// fill ingestion -- edge cases
fill_seq_gap_still_advances_tip
fill_seq_zero_first_ever
fill_for_unknown_symbol_advances_tip_only
fill_taker_in_shard_maker_not
fill_maker_in_shard_taker_not
fill_self_trade_same_user_both_sides
fill_rapid_sequence_same_symbol
fill_interleaved_symbols
tip_not_advanced_on_duplicate_fill

// fee calculation
fill_taker_fee_deducted_from_collateral
fill_maker_fee_deducted_from_collateral
fill_maker_rebate_credited_to_collateral
fill_fee_persisted_with_fill_record

// main loop ordering
fills_processed_before_bbo
orders_processed_after_fills
bbo_skipped_under_load
stale_bbo_replaced_by_latest
mark_price_update_triggers_margin_recalc
empty_rings_no_crash
burst_fills_then_idle

// pre-trade risk -- core
order_accepted_margin_sufficient
order_rejected_margin_insufficient
frozen_margin_accumulates_on_multiple_orders
order_done_releases_frozen_margin

// pre-trade risk -- edge cases
order_for_user_not_in_shard_rejected
order_while_user_being_liquidated_rejected
order_reducing_position_always_accepted
order_with_zero_qty_rejected
order_duplicate_id_within_dedup_window
order_cancel_releases_frozen_margin
liquidation_order_skips_margin_check
```

---

## E2E Tests

### Phase 2: Shard-Level

```rust
shard_processes_1000_fills_positions_correct
shard_multi_symbol_tips_advance_independently
shard_margin_recalc_on_bbo_update
shard_order_accept_reject_flow
shard_liquidation_detected_on_price_drop
shard_bbo_skip_under_fill_pressure
shard_multiple_users_same_symbol
shard_user_opens_closes_reopens
shard_position_flip_through_fills
shard_fill_updates_exposure_index
shard_order_accepted_then_rejected_margin_used
shard_cancel_restores_margin_for_next_order
shard_mark_price_divergence_triggers_liquidation
shard_funding_settlement_at_interval
shard_idle_no_resource_leak
```

### Phase 5: Full System

```rust
full_lifecycle_order_fill_position_margin
multi_user_multi_symbol_positions_independent
funding_settlement_8h_correct
funding_rate_updates_on_price_change
mark_price_updates_via_dxs_consumer
liquidation_cascade_under_price_crash
bbo_skip_under_heavy_fill_load
order_rejected_during_liquidation
shard_boundary_fill_taker_shard0_maker_shard1
all_symbols_simultaneous_bbo_update
mark_price_stale_falls_back_to_index
rapid_open_close_cycles
max_users_per_shard_performance
fill_burst_after_idle_period
funding_with_position_changes_during_interval
```

---

## Integration Tests (testcontainers Postgres)

### Phase 3: Persistence

```rust
persist_positions_roundtrip
persist_fills_copy_batch
persist_tips_roundtrip
persist_funding_payments_append
cold_start_loads_positions
cold_start_loads_tips
recovery_bounded_loss_10ms
upsert_idempotent_on_replay
fill_partitioning_works
persist_handles_pg_connection_drop
persist_backpressure_ring_full
persist_empty_batch_no_transaction
persist_position_overwritten_by_later_version
cold_start_with_empty_postgres
```

### Phase 4: Replication + Failover

```rust
main_acquires_lease_replica_cannot
main_crash_replica_promotes
replica_applies_buffered_fills_on_promotion
replica_state_matches_main
both_crash_recovery_from_postgres
me_failover_dedup_by_seq
promotion_no_fill_loss
split_brain_prevented_by_advisory_lock
```

### Phase 5: Full System

```rust
full_crash_recovery_end_to_end
backpressure_slow_postgres
multi_shard_same_fill_different_users
funding_persisted_to_postgres
concurrent_shard_leases_independent
```

---

## Smoke Tests

```rust
risk_engine_responds_to_order
risk_engine_positions_update_on_fill
risk_engine_margin_query_returns
risk_engine_funding_rate_available
risk_engine_replica_running
```

---

## Benchmarks

### Phase 1

```rust
bench_apply_fill_to_position        // target <100ns
bench_portfolio_margin_10_positions  // target <10us
bench_portfolio_margin_50_positions
bench_index_price_calculation        // target <50ns
bench_exposure_lookup_100_users      // target <50ns
bench_exposure_lookup_1000_users
```

### Phase 2

```rust
bench_shard_fill_throughput_1_symbol     // target >1M fills/sec
bench_shard_fill_throughput_10_symbols
bench_shard_fill_throughput_100_symbols
bench_pretrade_check_latency             // target <5us
bench_margin_recalc_100_users_1_symbol   // target <10us/user
bench_margin_recalc_100_users_10_symbols
bench_bbo_processing                     // target <200ns
bench_main_loop_idle                     // target <1us
```

### Phase 3

```rust
bench_flush_100_positions       // target <5ms
bench_flush_1000_positions      // target <15ms
bench_copy_1000_fills           // target <5ms
bench_copy_10000_fills          // target <20ms
bench_load_10k_positions        // target <500ms
bench_load_100k_positions       // target <2s
bench_sustained_flush_10ms_interval_60s
```

### Phase 4

```rust
bench_failover_detection_time        // target <600ms
bench_replica_drain_1000_fills       // target <100us
bench_replica_drain_10000_fills      // target <1ms
bench_promotion_total_time           // target <1s
```

### Phase 5

```rust
bench_e2e_fill_to_margin_latency          // target <15us
bench_sustained_1m_fills_10_symbols_100_users
    // target >100K fills/sec/shard
bench_margin_recalc_1000_users_10_symbols  // target <10ms
bench_memory_10k_positions                 // target <10MB
bench_memory_100k_positions                // target <100MB
bench_funding_settlement_10k_positions     // target <50ms
bench_cold_start_10k_positions_50_symbols  // target <5s
```

Targets from RISK.md §performance:

| Path | Target |
|------|--------|
| Fill processing | <1us |
| Pre-trade check | <5us |
| Per-tick margin | <10us/user |
| BBO -> index price | <100ns |
| Postgres flush | every 10ms |
| Failover detection | ~500ms |
| Replay catch-up | <5s |

---

## Correctness Invariants

1. **Fills never lost** -- sum of applied fills = sum of ME-emitted
   fills (for shard users)
2. **Position = sum of fills** -- verified after every test scenario
3. **Tips monotonic** -- never decreases, even after recovery
4. **Margin consistent with positions** -- recalc from scratch matches
   incremental state
5. **Funding zero-sum** -- per symbol per interval
6. **Exposure index consistent** -- matches actual positions
7. **Advisory lock exclusive** -- at most one main per shard
8. **Seq dedup prevents double-counting** -- replay = no change

---

## Integration Points

- Receives fills/BBO from matching engine SPSC rings
  (CONSISTENCY.md §1)
- Receives orders from gateway SPSC ring (NETWORK.md)
- Receives mark prices from DXS consumer (MARK.md §1)
- Sends orders to matching engine SPSC ring
- Sends fills/done to gateway SPSC ring
- Persists to Postgres via write-behind worker
- Replica sync via SPSC tip channel
- Liquidation via embedded liquidator (LIQUIDATOR.md)
- Funding via embedded funding engine
- System-level: full crash/recovery tests (TESTING.md §3)
