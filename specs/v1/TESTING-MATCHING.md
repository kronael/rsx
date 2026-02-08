# TESTING-MATCHING.md — Matching Engine Tests

Source specs: [ORDERBOOK.md](ORDERBOOK.md),
[CONSISTENCY.md](CONSISTENCY.md), [RPC.md](RPC.md),
[GRPC.md](GRPC.md)

Binary: `rsx-matching` (one process per symbol or symbol group)

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| M1 | Single-threaded per symbol, no locks | ORDERBOOK.md §0 |
| M2 | GTC limit orders only in v1 | ORDERBOOK.md §5 |
| M3 | Tick/lot validation before matching | ORDERBOOK.md §5 |
| M4 | Reduce-only enforcement before matching | ORDERBOOK.md §5 |
| M5 | UUIDv7 dedup via FxHashMap, 5min window | RPC.md, GRPC.md §7 |
| M6 | Event fan-out to risk/gateway/mktdata SPSC | CONSISTENCY.md §1 |
| M7 | push_spin on ring full (stall, no drop) | CONSISTENCY.md §3 |
| M8 | Fills precede ORDER_DONE | GRPC.md §fills |
| M9 | Exactly-one completion per order | GRPC.md §completion |
| M10 | Fill price = maker price | ORDERBOOK.md §5 |
| M11 | BBO emitted after best bid/ask change | CONSISTENCY.md §1 |
| M12 | WAL persistence via embedded WalWriter | ORDERBOOK.md §2.8 |
| M13 | Online snapshot + WAL replay recovery | ORDERBOOK.md §2.8 |
| M14 | DxsReplay server for downstream consumers | DXS.md §5 |
| M15 | Config polling every 10min, CONFIG_APPLIED | ORDERBOOK.md §2.9 |
| M16 | Position tracking per user (net_qty) | ORDERBOOK.md §6.5 |
| M17 | Deferred user reclamation (60s delay) | ORDERBOOK.md §6.5 |

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

### Config Application

```rust
config_applied_emits_event
config_version_monotonic
config_effective_at_respected
config_updates_tick_lot_sizes
config_poll_interval_10min
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
```

### Recentering Under Load

```rust
price_crash_50pct_recenter_while_matching
orders_during_migration_all_processed
cancel_during_migration_succeeds
migration_completes_between_order_bursts
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
bench_e2e_order_to_fill_latency      // target <50us same machine
```

Targets from TESTING.md §6:

| Metric | Target |
|--------|--------|
| Insert | 100-500ns (p50/p99/p99.9) |
| Match | 100-500ns |
| Cancel | 100-300ns |
| E2E latency (same machine, SPSC) | <50us |
| Normal load | 10K orders/sec sustained 10min |
| Burst load | 100K orders/sec spike 10s |

---

## Integration Points

- Imports `rsx-book` crate for orderbook data structures
  (ORDERBOOK.md §3)
- Embeds `rsx-dxs` WalWriter + DxsReplay server
  (ORDERBOOK.md §2.8, DXS.md §5)
- SPSC fan-out to risk, gateway, mktdata rings
  (CONSISTENCY.md §1)
- Receives orders from risk engine via SPSC ring
  (NETWORK.md, RISK.md §6)
- System-level: participates in full order lifecycle tests
  (TESTING.md §2 e2e, §3 integration)
- Load tests: BTC-PERP hotspot, Zipf distribution
  (TESTING.md §6 load tests)
