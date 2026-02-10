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
| R21 | Forward CONFIG_APPLIED to gateway for cache sync | §1 |
| R22 | Backpressure: stall on ring full / flush lag / replica lag | §persistence |
| R23 | Promotion invariant: apply fills up to last tip only | §replication |
| R24 | Replay via DXS consumer from tips + 1, CaughtUp signal | §replication |
| R25 | Missed funding intervals settled on next startup | §5 |
| R26 | ME failover: dedup by (symbol_id, seq), no restart | §ME failover |
| R27 | Account persistence: collateral, frozen_margin to Postgres | §persistence |
| R28 | Both-crash recovery: 100ms data loss bound | §recovery |
| R29 | Index price fallback: no BBO ever -> use mark price | §4 |
| R30 | Funding payments persisted append-only to Postgres | §5 |
| R31 | Fills persisted via COPY binary bulk insert | §persistence |

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
flip_realizes_pnl_then_opens_at_fill_price
fill_at_same_price_no_pnl
realized_pnl_accumulates_across_fills
self_trade_taker_and_maker_same_user
max_qty_no_overflow
max_price_no_overflow
position_version_increments_per_fill
empty_position_zero_notional_zero_upnl

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
margin_mark_price_unavailable_uses_index
margin_mark_price_zero_handled
margin_max_leverage_enforced
frozen_margin_across_multiple_pending_orders
order_done_partial_fill_releases_remaining_frozen
order_failed_releases_all_frozen
fee_reserve_included_in_pretrade_check

// price.rs -- core
index_price_size_weighted_mid
index_price_balanced_book_equals_mid
index_price_imbalanced_favors_thicker_side

// price.rs -- edge cases
index_price_one_side_zero_qty_uses_that_side
index_price_both_sides_zero_qty_keeps_last
index_price_no_bbo_ever_uses_mark_price
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
funding_missed_interval_settled_on_startup
funding_settlement_uses_latest_mark_price
funding_payment_formula_qty_times_mark_times_rate

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
fill_fee_uses_symbol_config_rates

// order_done/cancel/failed -- frozen margin release
order_done_releases_frozen_margin_for_order
order_cancel_releases_frozen_margin_for_order
order_failed_releases_all_frozen_for_order

// config updates
config_applied_event_updates_symbol_params
config_applied_forwarded_to_gateway

// main loop ordering
fills_processed_before_bbo
orders_processed_after_fills
bbo_skipped_under_load
stale_bbo_replaced_by_latest
mark_price_update_triggers_margin_recalc
empty_rings_no_crash
burst_fills_then_idle
funding_check_amortized_in_loop
liquidation_check_in_loop_after_funding

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
shard_config_applied_updates_margin_rates
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
funding_missed_interval_settled_on_restart
me_failover_no_restart_dedup_by_seq
config_applied_forwarded_to_gateway_e2e
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
persist_accounts_roundtrip
persist_fills_via_copy_binary
persist_no_version_guard_on_upsert
cold_start_with_empty_postgres
cold_start_loads_accounts
backpressure_flush_lag_stalls_hot_path
backpressure_replica_ring_full_stalls_hot_path
```

### Phase 4: Replication + Failover

```rust
main_acquires_lease_replica_cannot
main_crash_replica_promotes
replica_applies_buffered_fills_on_promotion
replica_applies_fills_up_to_tip_on_sync
replica_buffers_fills_per_symbol
replica_polls_advisory_lock_500ms
replica_state_matches_main
both_crash_recovery_from_postgres
both_crash_loss_bounded_100ms
replay_from_tips_plus_one_via_dxs
replay_caught_up_signal_goes_live
me_failover_dedup_by_seq
promotion_invariant_only_up_to_last_tip
promotion_connects_gateway_starts_writebehind
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
funding_payments_persisted_append_only
accounts_persisted_on_flush
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
bench_index_price_calculation        // target <100ns
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
bench_bbo_processing                     // target <100ns
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
9. **Promotion invariant** -- replica applies fills only up to last
   tip from main, never beyond
10. **Backpressure stall** -- hot path stalls when persistence ring
    full, flush lag > 10ms, or replica ring full (100ms bound)
11. **Account balance consistent** -- collateral - fees + rebates +
    realized_pnl + funding = expected balance

---

## Integration Points

- Receives fills/BBO/OrderDone from matching engine via CMP/UDP
  (CONSISTENCY.md §1, event routing table)
- Receives orders from gateway via CMP/UDP (NETWORK.md §data flow)
- Mark prices from DXS are not wired into risk in v1
- Sends orders to matching engine via CMP/UDP (RISK.md §6)
- Sends fills/done to gateway via CMP/UDP (CONSISTENCY.md §1)
- Forwards CONFIG_APPLIED to gateway (RISK.md §1)
- Persists positions/accounts/fills/tips to Postgres via
  write-behind worker (RISK.md §persistence)
- Replica sync not implemented in v1
- Advisory lock via Postgres pg_advisory_lock (RISK.md §replication)
- Replay via WAL exists but DXS consumer path not wired in v1
- Liquidation via embedded liquidator (LIQUIDATOR.md)
- Funding via embedded funding engine (RISK.md §5)
- ME failover: dedup by (symbol_id, seq) (RISK.md §ME failover)
- Backpressure: CMP flow control (CONSISTENCY.md §3)
- System-level: full crash/recovery tests (TESTING.md §3)

## Implementation Status (2026-02-10)

171 tests across 11 files. Persist tests need Docker.

### Unit Tests -- Phase 1

| Spec Test | Status | File |
|-----------|--------|------|
| apply_buy_fill_opens_long | DONE | position_test.rs |
| apply_sell_fill_opens_short | DONE | position_test.rs |
| apply_opposing_fill_reduces_position | DONE | position_test.rs |
| apply_fill_closing_position_realizes_pnl | DONE | position_test.rs |
| avg_entry_price_weighted_correctly | DONE | position_test.rs |
| multiple_fills_same_side_accumulate | DONE | position_test.rs |
| fill_larger_than_position_flips_side | DONE | position_test.rs |
| flip_long_to_short_single_fill | DONE | position_test.rs |
| flip_short_to_long_single_fill | DONE | position_test.rs |
| flip_realizes_pnl_then_opens_at_fill_price | DONE | position_test.rs |
| max_qty_no_overflow | DONE | position_test.rs |
| max_price_no_overflow | DONE | position_test.rs |
| portfolio_margin_single_position | DONE | margin_test.rs:50 |
| portfolio_margin_multi_symbol | DONE | margin_test.rs:70 |
| portfolio_margin_long_short_offset | DONE | margin_test.rs:86 |
| check_order_sufficient_margin_accepts | DONE | margin_test.rs:99 |
| check_order_insufficient_margin_rejects | DONE | margin_test.rs:110 |
| needs_liquidation_below_maintenance | DONE | margin_test.rs:124 |
| needs_liquidation_above_maintenance_ok | DONE | margin_test.rs:135 |
| frozen_margin_reserved_on_order | DONE | margin_test.rs:146 |
| frozen_margin_released_on_done | DONE | margin_test.rs:159 |
| check_order_exactly_at_margin_limit | DONE | margin_test.rs:169 |
| margin_with_zero_collateral_rejects | DONE | margin_test.rs:197 |
| margin_unrealized_pnl_affects_equity | DONE | margin_test.rs:221 |
| margin_mark_price_unavailable_uses_index | DONE | margin_test.rs:232 |
| frozen_margin_across_multiple_orders | DONE | margin_test.rs:278 |
| order_failed_releases_all_frozen | DONE | margin_test.rs:307 |
| fee_reserve_included_in_pretrade_check | DONE | margin_test.rs:315 |
| exposure_add_user_on_fill | DONE | margin_test.rs:338 |
| exposure_remove_user_on_close | DONE | margin_test.rs:345 |
| reduce_only_bypasses_margin | DONE | margin_test.rs:393 |
| liquidation_order_skips_margin | DONE | margin_test.rs:405 |
| index_price_size_weighted_mid | DONE | price_test.rs |
| index_price_balanced_book_equals_mid | DONE | price_test.rs |
| index_price_one_side_zero | DONE | price_test.rs |
| index_price_no_bbo_ever_uses_mark | DONE | price_test.rs |
| funding_rate_mark_above_index_positive | DONE | funding_test.rs |
| funding_rate_mark_below_index_negative | DONE | funding_test.rs |
| funding_rate_clamped_to_bounds | DONE | funding_test.rs |
| funding_payment_long_pays_when_positive | DONE | funding_test.rs |
| funding_zero_sum_across_all_users | DONE | funding_test.rs |
| funding_missed_interval_settled | DONE | funding_test.rs |
| fee_floor_division_truncates | DONE | fee_test.rs |
| fee_negative_bps_is_rebate | DONE | fee_test.rs |

### Unit Tests -- Phase 2

| Spec Test | Status | File |
|-----------|--------|------|
| fill_for_shard_user_updates_position | DONE | shard_test.rs |
| fill_for_other_shard_ignored | DONE | shard_test.rs |
| fill_dedup_by_seq | DONE | shard_test.rs |
| fill_advances_tip_per_symbol | DONE | shard_test.rs |
| order_accepted_margin_sufficient | DONE | shard_test.rs |
| order_rejected_margin_insufficient | DONE | shard_test.rs |
| order_while_user_liquidated_rejected | TODO | Need integration |
| mark_price_update_triggers_recalc | DONE | margin_recalc_test.rs |
| config_applied_event_updates_params | TODO | |
| config_applied_forwarded_to_gateway | TODO | |

### Integration Tests (Postgres)

| Spec Test | Status | File |
|-----------|--------|------|
| persist_positions_roundtrip | DONE | persist_test.rs |
| persist_tips_roundtrip | DONE | persist_test.rs |
| cold_start_loads_positions | DONE | persist_test.rs |
| cold_start_loads_tips | DONE | persist_test.rs |
| cold_start_loads_accounts | DONE | persist_test.rs |
| cold_start_with_empty_postgres | DONE | persist_test.rs |
| upsert_idempotent_on_replay | DONE | persist_test.rs |
| advisory_lock_exclusive | DONE | persist_test.rs |
| replay_from_wal_rebuilds_positions | DONE | persist_test.rs |
| main_acquires_lease_replica_cannot | TODO | Phase 4 |
| main_crash_replica_promotes | TODO | Phase 4 |
| replica_applies_buffered_fills | TODO | Phase 4 |
| both_crash_recovery_from_postgres | TODO | Phase 4 |

### E2E Tests

| Spec Test | Status | File |
|-----------|--------|------|
| shard_processes_1000_fills | DONE | shard_e2e_test.rs |
| shard_multi_symbol_tips | DONE | shard_e2e_test.rs |
| shard_order_accept_reject_flow | DONE | shard_e2e_test.rs |
| shard_liquidation_on_price_drop | DONE | shard_e2e_test.rs |
| shard_funding_settlement | DONE | shard_e2e_test.rs |
| full_lifecycle_order_fill_margin | TODO | Phase 5 |
| liquidation_cascade | TODO | Phase 5 |
| me_failover_dedup_by_seq | TODO | Phase 4 |
