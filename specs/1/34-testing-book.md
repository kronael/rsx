---
status: shipped
---

# TESTING-BOOK.md — Shared Orderbook Crate Tests

Source specs: [ORDERBOOK.md](ORDERBOOK.md),
[CONSISTENCY.md](CONSISTENCY.md)

Crate: `rsx-book` — shared by matching engine and market data.

## Table of Contents

- [Requirements Checklist](#requirements-checklist)
- [Unit Tests](#unit-tests)
- [E2E Tests](#e2e-tests)
- [Benchmarks](#benchmarks)
- [Integration Points](#integration-points)

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| B1 | Price/Qty are i64 newtypes, never float | §1 |
| B2 | Tick/lot size validation at order entry | §2 |
| B3 | Compressed zone indexing (5 zones) | §2.5 |
| B4 | Bisection lookup 2-3 comparisons | §2.5 |
| B5 | Smooshed ticks store exact price per order | §2.6 |
| B6 | Matching at smooshed levels checks actual price | §2.6 |
| B7 | Incremental copy-on-write recentering | §2.7 |
| B8 | Two pre-allocated level arrays (active+staging) | §2.7 |
| B9 | Frontier-based lazy migration | §2.7 |
| B10 | Slab arena O(1) alloc/free via free list | §3 |
| B11 | OrderSlot 128B, #[repr(C, align(64))] | §3 |
| B12 | PriceLevel 24B (head, tail, total_qty, count) | §3 |
| B13 | Best bid/ask tracking (cached tick index) | §3 |
| B14 | O(1) add, cancel, match in zone 0 | §4 |
| B15 | GTC limit orders only in v1 | §5 |
| B16 | Fill price = maker's price | §5 |
| B17 | Event buffer fixed array, no heap alloc | §6 |
| B18 | Fills precede ORDER_DONE per order | §6 |
| B19 | Reduce-only enforcement via position tracking | §6.5 |
| B20 | User position tracking (net_qty per user) | §6.5 |
| B21 | Zero allocation on hot path | §7 |
| B22 | ~617K level slots, ~14.8MB per array | §2.5 |
| B23 | 78M order slots, ~10GB per book | §7 |
| B24 | Modify order (price change) = cancel + re-insert, loses time priority | §4 |
| B25 | Modify order (qty down) = in-place reduction, keeps time priority | §4 |
| B26 | SymbolConfig distribution: ME polls metadata, emits CONFIG_APPLIED | §2.9 |
| B27 | Single thread per orderbook, no locking | Design Goals |
| B28 | Hot/cold cache line split: matching touches only line 1 (48B) | §7 |
| B29 | GTC limit orders only, perpetuals only (no market orders, no spot) | Design Goals |
| B30 | WAL + online snapshot for durability/recovery | §2.8 |
| B31 | Snapshot never runs during migration | §2.8 |

---

## Unit Tests

### Price / Qty / Validation

```rust
price_newtype_ordering_correct
qty_newtype_arithmetic
validate_order_price_aligned_to_tick
validate_order_price_not_aligned_rejects
validate_order_qty_aligned_to_lot
validate_order_qty_not_aligned_rejects
validate_order_qty_zero_rejects
validate_order_price_zero_rejects
validate_order_price_negative_rejects
validate_order_qty_negative_rejects
```

### Compression Map / Zone Lookup

```rust
compression_map_zone_0_1_to_1_resolution
compression_map_zone_1_10_to_1
compression_map_zone_2_100_to_1
compression_map_zone_3_1000_to_1
compression_map_zone_4_catchall_two_slots
price_to_index_at_mid_price
price_to_index_at_zone_boundary_0_1
price_to_index_at_zone_boundary_1_2
price_to_index_at_zone_boundary_2_3
price_to_index_at_zone_boundary_3_4
price_to_index_bid_side_decreasing
price_to_index_ask_side_increasing
price_to_index_symmetric_around_mid
price_to_index_extreme_distance_catchall
total_slot_count_matches_expected
```

### Modify Order

```rust
modify_price_cancels_and_reinserts
modify_price_loses_time_priority
modify_qty_down_in_place
modify_qty_down_keeps_time_priority
modify_qty_down_updates_level_total_qty
modify_qty_down_to_zero_removes_order
```

### Struct Layout

```rust
order_slot_size_is_128_bytes
order_slot_alignment_is_64
price_level_size_is_24_bytes
order_slot_hot_fields_in_first_cache_line
```

### Slab Allocator

```rust
slab_alloc_returns_sequential_indices
slab_free_then_alloc_reuses_slot
slab_free_list_lifo_order
slab_alloc_exhausts_free_list_then_bumps
slab_free_all_then_realloc_all
slab_double_free_panics_or_corrupts  // defensive check
slab_capacity_limit
```

### PriceLevel Operations

```rust
level_append_order_updates_head_tail
level_append_order_increments_count_qty
level_remove_head_updates_head
level_remove_tail_updates_tail
level_remove_middle_maintains_links
level_remove_last_order_zeroes_count
level_empty_after_removing_all_orders
```

### Best Bid/Ask Tracking

```rust
insert_bid_updates_best_bid
insert_ask_updates_best_ask
insert_below_best_bid_no_change
insert_above_best_ask_no_change
remove_best_bid_scans_to_next
remove_best_ask_scans_to_next
remove_best_bid_empty_book_returns_none
remove_best_ask_empty_book_returns_none
best_bid_less_than_best_ask_invariant
```

### Matching

```rust
match_buy_against_single_ask
match_sell_against_single_bid
match_buy_multiple_makers_same_level
match_buy_crosses_multiple_levels
match_partial_fill_maker_remains
match_partial_fill_taker_rests
match_no_cross_taker_rests_immediately
match_exact_fill_both_removed
match_fill_price_is_maker_price
match_fifo_within_price_level
match_buy_limit_below_best_ask_no_match
match_sell_limit_above_best_bid_no_match
```

### Smooshed Tick Matching

```rust
smooshed_level_orders_with_different_prices
smooshed_match_skips_orders_outside_limit
smooshed_match_fills_qualifying_orders_only
smooshed_match_preserves_time_priority
smooshed_zone_4_catchall_match
```

### Events

```rust
event_buffer_fill_emitted_on_match
event_buffer_order_inserted_on_rest
event_buffer_order_cancelled_on_cancel
event_buffer_order_done_after_full_fill
event_buffer_order_failed_on_validation
event_buffer_len_reset_each_cycle
event_buffer_multiple_fills_single_order
event_buffer_fills_before_done
```

### Reduce-Only / User Position Tracking

```rust
user_state_net_qty_updates_on_fill
user_state_buy_increases_net_qty
user_state_sell_decreases_net_qty
reduce_only_buy_rejected_if_long
reduce_only_sell_rejected_if_short
reduce_only_buy_accepted_if_short
reduce_only_sell_accepted_if_long
reduce_only_qty_clamped_to_position
reduce_only_no_position_rejected
user_state_assigned_on_first_order
user_state_reclaimed_after_idle
```

### Symbol Config

```rust
config_applied_emitted_on_update
config_tick_size_change_validates_new_orders
config_lot_size_change_validates_new_orders
config_version_monotonic
```

### Recentering

```rust
recenter_triggers_when_mid_drifts_beyond_zone_0
recenter_swaps_active_and_staging
recenter_frontier_starts_at_new_mid
resolve_level_migrates_on_access_outside_frontier
migrate_single_level_moves_orders_to_new_indices
migrate_smooshed_level_unsmooshes_to_finer_slots
migrate_empty_level_is_noop
migrate_batch_expands_frontiers
migrate_completes_when_all_levels_drained
cancel_during_migration_resolves_first
insert_during_migration_goes_to_new_array
best_bid_ask_correct_after_recenter
snapshot_blocked_during_migration
snapshot_runs_after_migration_completes
```

---

## E2E Tests

```rust
// full order lifecycle
order_insert_match_fill_done_sequence
order_insert_rest_cancel_done_sequence
order_insert_partial_fill_rest_then_cancel
multi_fill_whale_order_500_makers

// book state
empty_book_insert_first_bid_and_ask
book_spread_narrows_on_insert
book_spread_widens_on_cancel
crossed_book_impossible_after_any_operation

// recentering scenario
btc_50pct_crash_recenter_no_lost_orders
btc_3x_rally_recenter_catchall_absorbs
rapid_orders_during_migration_correct
migration_completes_in_idle_cycles

// modify order lifecycle
modify_price_cancel_reinsert_full_lifecycle
modify_qty_down_partial_then_fill

// config updates
config_applied_mid_session_validates_new_tick
config_applied_propagates_to_consumers

// snapshot + recovery
snapshot_and_wal_recovery_restores_book
snapshot_waits_for_migration_before_running

// stress
1m_insert_cancel_cycles_no_slab_leak
alternating_fill_cancel_slab_reuse
zipf_distribution_100_symbols
```

---

## Benchmarks

```rust
bench_insert_order_zone_0          // target 100-500ns
bench_insert_order_zone_3          // compare to zone 0
bench_cancel_order_by_handle       // target 100-300ns
bench_match_single_fill_zone_0     // target 100-500ns
bench_match_10_fills_same_level    // target <5us
bench_match_smooshed_level_100     // target <50us
bench_price_to_index_bisection     // target <5ns
bench_slab_alloc_free_cycle        // target <50ns
bench_recenter_10k_orders          // target <10ms
bench_recenter_lazy_per_access     // target <3us
bench_event_buffer_drain_100       // target <1us
bench_best_bid_scan_after_cancel   // target <100ns amortized
bench_modify_order_price_change    // target <1us (cancel + insert)
bench_modify_order_qty_down        // target <100ns (in-place)
```

Targets from ORDERBOOK.md §4 and TESTING.md:

| Operation | Target |
|-----------|--------|
| Add order | O(1), 100-500ns |
| Cancel order | O(1), 100-300ns |
| Match per fill (zone 0) | O(1), 100-500ns |
| Modify (price change) | O(1) + O(1), <1us |
| Modify (qty down) | O(1), <100ns |
| Recentering per access | O(1) amortized, ~1-3us |
| Memory: 78M orders | ~10GB (128B slots) |
| Price level arrays | ~30MB (2 x 617K x 24B) |

---

## Integration Points

- Matching engine imports `rsx-book` for order processing
  (ORDERBOOK.md §3)
- Market data service imports `rsx-book` for shadow orderbook
  (MARKETDATA.md §2, NETWORK.md §MARKETDATA)
- BookObserver trait allows different event handling per consumer
- Event buffer drained into CMP/UDP fan-out (CONSISTENCY.md §1)
- Event routing per consumer: Fill to risk/gateway/mktdata,
  BBO to risk, OrderInserted to mktdata, OrderCancelled to
  gateway/mktdata, OrderDone to risk/gateway
  (CONSISTENCY.md §1)
- Mirrored stream to hot spare ME (CONSISTENCY.md §1)
- WAL + online snapshot for book persistence and recovery
  (ORDERBOOK.md §2.8, DXS.md §3)
- Replica takeover via DXS consumer on ME WAL stream
  (ORDERBOOK.md §2.8)
- SymbolConfig distributed from metadata store, CONFIG_APPLIED
  syncs risk and gateway caches (ORDERBOOK.md §2.9)
- System-level tests verify matching engine uses book correctly
  under load (TESTING.md §6 load tests)
