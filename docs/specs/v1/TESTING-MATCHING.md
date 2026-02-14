# TESTING-MATCHING.md — Matching Engine Tests

Source specs: [ORDERBOOK.md](ORDERBOOK.md),
[CONSISTENCY.md](CONSISTENCY.md), [RPC.md](RPC.md),
[MESSAGES.md](MESSAGES.md)

Binary: `rsx-matching` (one process per symbol or symbol group)

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
| M1 | Single-threaded per symbol, no locks | ORDERBOOK.md §0 |
| M2 | GTC limit orders only in v1 | ORDERBOOK.md §5 |
| M3 | Tick/lot validation before matching | ORDERBOOK.md §5 |
| M4 | Reduce-only enforcement before matching | ORDERBOOK.md §5 |
| M5 | UUIDv7 dedup via FxHashMap, 5min window | RPC.md, MESSAGES.md §7 |
| M6 | Event fan-out to risk/gateway/mktdata via CMP/UDP | CONSISTENCY.md §1 |
| M7 | CMP flow control via Status/Nak (no silent drop) | CONSISTENCY.md §3 |
| M8 | Fills precede ORDER_DONE | MESSAGES.md §fills |
| M9 | Exactly-one completion per order | MESSAGES.md §completion |
| M10 | Fill price = maker price | ORDERBOOK.md §5 |
| M11 | BBO emitted after best bid/ask change | CONSISTENCY.md §1 |
| M12 | WAL persistence via embedded WalWriter | ORDERBOOK.md §2.8 |
| M13 | Online snapshot + WAL replay recovery | ORDERBOOK.md §2.8 |
| M14 | DxsReplay server for downstream consumers | DXS.md §5 |
| M15 | Config polling every 10min, CONFIG_APPLIED | ORDERBOOK.md §2.9 |
| M16 | Position tracking per user (net_qty) | ORDERBOOK.md §6.5 |
| M17 | Deferred user reclamation (60s, net_qty==0 && order_count==0) | ORDERBOOK.md §6.5 |
| M18 | Fixed-point integer Price/Qty, never floating point | ORDERBOOK.md §1 |
| M19 | Compressed zone indexing (5 zones, bisection lookup) | ORDERBOOK.md §2.5 |
| M20 | Smooshed tick matching: scan within slot, check exact price | ORDERBOOK.md §2.6 |
| M21 | Incremental CoW recentering (no stop-the-world) | ORDERBOOK.md §2.7 |
| M22 | Slab allocator: O(1) alloc/free, free list, no shrink | ORDERBOOK.md §3 |
| M23 | Zero heap allocation on hot path | ORDERBOOK.md §7 |
| M24 | Event buffer: fixed array [Event; 10_000], no heap | ORDERBOOK.md §6 |
| M25 | Per-consumer CMP/UDP links (slow mktdata doesn't stall risk) | CONSISTENCY.md §3 |
| M26 | Total order within symbol (monotonic seq), no cross-symbol | CONSISTENCY.md §2 |
| M27 | ORDER_DONE is commit boundary for multi-fill sequences | CONSISTENCY.md §key invariants |
| M28 | Fills are final, no rollback | CONSISTENCY.md §4 |
| M29 | Snapshot + migration mutual exclusion | ORDERBOOK.md §2.8 |
| M30 | Best bid/ask tracking, scan on level exhaustion | ORDERBOOK.md §3 |
| M31 | Ingress backpressure: gateway rejects at 10k buffer cap | CONSISTENCY.md §3 |

---

## Unit Tests

### Order Processing

```rust
new_order_valid_tick_lot_accepted
new_order_invalid_tick_rejected
new_order_invalid_lot_rejected
new_order_zero_qty_rejected
new_order_negative_price_rejected
new_order_duplicate_id_rejected
new_order_after_dedup_window_accepted
```

### Deduplication

```rust
dedup_order_id_exists_returns_duplicate
dedup_cancelled_order_still_in_map
dedup_pruning_removes_after_5min
dedup_pruning_preserves_recent
dedup_cleanup_periodic_scan_10s
dedup_fxhashmap_lookup_by_uuid
```

### Event Fan-Out

```rust
fill_event_sent_to_risk_gateway_mktdata
bbo_event_sent_to_risk_only
order_inserted_sent_to_mktdata_only
order_cancelled_sent_to_gateway_mktdata
order_done_sent_to_risk_gateway
drain_events_empties_buffer
drain_events_order_matches_emission_order
```

### Reduce-Only Integration

```rust
reduce_only_order_closes_long_position
reduce_only_order_closes_short_position
reduce_only_order_clamped_to_position_size
reduce_only_no_position_fails
reduce_only_same_direction_fails
reduce_only_fill_updates_position_tracking
```

### Position Tracking

```rust
fill_updates_taker_and_maker_net_qty
position_buy_increases_net_qty
position_sell_decreases_net_qty
user_state_assigned_on_first_order
user_reclaim_when_net_qty_zero_and_no_orders
user_reclaim_deferred_60s_grace_period
user_free_list_reuses_reclaimed_slots
```

### Compression Map & Indexing

```rust
price_to_index_zone_0_one_to_one
price_to_index_zone_1_compression_10
price_to_index_zone_2_compression_100
price_to_index_zone_3_compression_1000
price_to_index_zone_4_catch_all_single_slot
price_to_index_bisection_2_to_3_comparisons
price_to_index_bid_and_ask_sides_distinct
compression_map_recomputed_on_recenter
```

### Slab Allocator

```rust
slab_alloc_returns_sequential_indices
slab_free_pushes_to_free_list
slab_alloc_reuses_freed_slots
slab_never_shrinks_vec
slab_free_list_no_cycles
slab_alloc_free_1m_ops_no_leak
```

### Best Bid/Ask Tracking

```rust
best_bid_updated_on_insert
best_ask_updated_on_insert
best_bid_scans_next_on_level_exhaustion
best_ask_scans_next_on_level_exhaustion
best_bid_none_when_book_empty
best_ask_none_when_book_empty
best_bid_below_best_ask_always
```

### Event Buffer

```rust
event_buffer_fixed_array_no_heap
event_len_reset_per_cycle_single_store
emit_writes_sequential_slots
event_buffer_max_10000_events
```

### Config Application

```rust
config_applied_emits_event
config_version_monotonic
config_effective_at_respected
config_updates_tick_lot_sizes
config_poll_interval_10min
config_from_metadata_store
```

---

## E2E Tests

### Order Lifecycle

```rust
order_submit_fill_done_complete_lifecycle
order_submit_rest_cancel_done_lifecycle
order_submit_partial_fill_rest_then_fill
order_submit_fail_insufficient_tick
order_submit_multi_fill_500_makers
```

### Correctness Invariants (TESTING.md)

```rust
fills_precede_order_done_always
exactly_one_completion_per_order
fifo_within_price_level_verified
no_negative_qty_in_orderbook
best_bid_ask_coherent_after_every_op
slab_no_leak_after_1m_operations
event_seq_monotonic_within_symbol
fills_are_final_no_rollback
order_done_is_commit_boundary_for_multi_fill
zero_heap_allocation_during_matching
```

### WAL + Recovery

```rust
wal_records_written_for_all_events
crash_recovery_from_snapshot_plus_wal
recovery_book_matches_pre_crash_state
recovery_seq_continues_from_last
wal_rotation_during_heavy_load
```

### DxsReplay

```rust
replay_serves_historical_records
replay_caught_up_then_live_tail
replay_multiple_consumers_concurrent
replay_consumer_disconnect_no_crash
```

### Fan-Out Under Load

```rust
10k_orders_per_sec_all_rings_drained
ring_full_on_mktdata_stalls_matching
ring_full_on_risk_stalls_matching
backpressure_recovery_resumes_matching
slow_mktdata_ring_does_not_stall_risk_ring
per_consumer_ring_independence_verified
ingress_backpressure_rejects_at_10k_buffer
```

### Smooshed Tick Matching

```rust
smooshed_level_scan_checks_exact_price
smooshed_level_skips_non_matching_price
smooshed_level_time_priority_within_slot
catch_all_zone_4_orders_coexist_single_slot
unsmoosh_on_recenter_spreads_to_finer_slots
```

### Recentering Under Load

```rust
price_crash_50pct_recenter_while_matching
orders_during_migration_all_processed
cancel_during_migration_succeeds
migration_completes_between_order_bursts
snapshot_waits_for_migration_to_finish
snapshot_and_migration_mutually_exclusive
```

---

## Benchmarks

```rust
bench_process_new_order_insert       // target 100-500ns
bench_process_new_order_match        // target 100-500ns
bench_cancel_order                   // target 100-300ns
bench_drain_events_10_fills          // target <1us
bench_drain_events_100_fills         // target <10us
bench_dedup_lookup_fxhashmap         // target <50ns
bench_dedup_cleanup_1000_entries     // target <100us
bench_wal_append_per_event           // target <200ns
bench_10k_orders_sec_sustained       // target stable latency
bench_100k_orders_sec_burst_10s      // target recovers gracefully
bench_price_to_index_bisection        // target <5ns
bench_recenter_lazy_migration         // target <3us per level
bench_slab_alloc_free                 // target <10ns
bench_smooshed_tick_match_k_orders    // target O(k) linear
bench_e2e_order_to_fill_latency      // target <50us same machine
```

Targets from TESTING.md §6:

| Metric | Target |
|--------|--------|
| Insert | 100-500ns (p50/p99/p99.9) |
| Match | 100-500ns |
| Cancel | 100-300ns |
| E2E latency (same machine, CMP/UDP) | <50us |
| Normal load | 10K orders/sec sustained 10min |
| Burst load | 100K orders/sec spike 10s |
| Recentering (lazy) | ~1-3us per level |
| Recentering (normal ops) | <1us overhead |
| Bisection lookup | <5ns |

---

## Integration Points

- Config polling tests use Postgres via testcontainers.

- Imports `rsx-book` crate for orderbook data structures
  (ORDERBOOK.md §3)
- Embeds `rsx-dxs` WalWriter + DxsReplay server
  (ORDERBOOK.md §2.8, DXS.md §5)
- CMP/UDP fan-out to risk, gateway, mktdata
  (CONSISTENCY.md §1)
- Receives orders from risk engine via CMP/UDP
  (NETWORK.md, RISK.md §6)
- System-level: participates in full order lifecycle tests
  (TESTING.md §2 e2e, §3 integration)
- Load tests: BTC-PERP hotspot, Zipf distribution
  (TESTING.md §6 load tests)
- Hot spare ME receives mirrored event stream
  (CONSISTENCY.md §1)
- Config distribution from metadata store, polling every
  10min (ORDERBOOK.md §2.9)
- Crash recovery: snapshot + WAL replay restores book
  (ORDERBOOK.md §2.8, NETWORK.md §matching engine failure)
- Replica takeover via DxsConsumer tip + Postgres advisory
  locks (ORDERBOOK.md §2.8)
