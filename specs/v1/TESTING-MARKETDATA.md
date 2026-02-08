# TESTING-MARKETDATA.md — Market Data Service Tests

Source specs: [MARKETDATA.md](MARKETDATA.md),
[NETWORK.md](NETWORK.md) §MARKETDATA,
[CONSISTENCY.md](CONSISTENCY.md) §1

Binary: `rsx-marketdata`

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| MD1 | Shadow orderbook per symbol (shared rsx-book) | NETWORK.md §MARKETDATA |
| MD2 | Derive BBO from shadow book | MARKETDATA.md |
| MD3 | Derive L2 depth (snapshot + delta) from events | MARKETDATA.md |
| MD4 | gRPC MarketData.Stream service | MARKETDATA.md §service |
| MD5 | Subscribe by symbol_id list + depth | MARKETDATA.md §subscribe |
| MD6 | L2Snapshot sent on initial subscribe | MARKETDATA.md §transport |
| MD7 | Deltas after snapshot, seq monotonic per symbol | MARKETDATA.md §transport |
| MD8 | Backpressure: drop deltas, resend snapshot | MARKETDATA.md §transport |
| MD9 | Seq gap -> client re-subscribes for snapshot | MARKETDATA.md §transport |
| MD10 | Public endpoint, no auth | MARKETDATA.md §transport |
| MD11 | Single-threaded, dedicated core, busy-spin | NETWORK.md §MARKETDATA |
| MD12 | Non-blocking epoll for WS I/O (no Tokio) | NETWORK.md §MARKETDATA |
| MD13 | SPSC consumer ring per matching engine | NETWORK.md §MARKETDATA |
| MD14 | Recovery via DXS replay from ME WAL | DXS.md §8 |
| MD15 | WS JSON: BBO, B (snapshot), D (delta), S, X | WEBPROTO.md |
| MD16 | Event routing: Fill + OrderInserted + Cancelled | CONSISTENCY.md §1 |

---

## Unit Tests

### Shadow Orderbook

```rust
shadow_book_insert_updates_level
shadow_book_cancel_removes_from_level
shadow_book_fill_reduces_qty
shadow_book_fill_removes_exhausted_order
shadow_book_bbo_derived_correctly
shadow_book_empty_returns_no_bbo
shadow_book_seq_monotonic
```

### BBO Derivation

```rust
bbo_update_on_best_bid_change
bbo_update_on_best_ask_change
bbo_no_update_if_unchanged
bbo_includes_count_and_qty
bbo_correct_after_fill_at_best
bbo_correct_after_cancel_at_best
```

### L2 Snapshot

```rust
snapshot_top_10_levels_correct
snapshot_top_25_levels_correct
snapshot_top_50_levels_correct
snapshot_fewer_levels_than_depth
snapshot_empty_book_returns_empty
snapshot_includes_all_fields_per_level
snapshot_seq_matches_latest_event
```

### L2 Delta

```rust
delta_insert_new_level
delta_remove_level_qty_zero
delta_update_level_qty_change
delta_side_correct_bid_vs_ask
delta_seq_monotonic_per_symbol
delta_only_for_changed_levels
```

### Subscription Management

```rust
subscribe_adds_symbol_to_client
subscribe_sends_snapshot_first
subscribe_multiple_symbols
unsubscribe_removes_symbol
unsubscribe_all_clears_all
subscribe_with_depth_parameter
resubscribe_sends_fresh_snapshot
```

### Backpressure

```rust
slow_client_deltas_dropped
slow_client_gets_fresh_snapshot
slow_client_does_not_block_other_clients
backpressure_threshold_configurable
```

---

## E2E Tests

```rust
// full pipeline
event_from_me_to_bbo_update_to_client
event_from_me_to_l2_delta_to_client
subscribe_snapshot_then_deltas_continuous
multi_symbol_events_routed_correctly

// correctness
shadow_book_matches_me_book_state
bbo_consistent_with_shadow_book
l2_snapshot_consistent_with_shadow_book
delta_sequence_builds_correct_book

// multi-client
100_clients_all_receive_same_bbo
client_subscribe_different_symbols
client_subscribe_different_depths
client_disconnect_no_crash

// WS protocol
ws_bbo_frame_format_correct
ws_b_snapshot_frame_format_correct
ws_d_delta_frame_format_correct
ws_seq_gap_client_resnapshots

// recovery
dxs_replay_rebuilds_shadow_book
recovery_from_me_wal_then_live
recovery_snapshot_sent_after_catchup

// failure modes
me_disconnect_shadow_book_stale
me_reconnect_resumes_events
me_restart_replay_from_wal
```

---

## Benchmarks

```rust
bench_shadow_book_insert             // target <500ns
bench_shadow_book_fill               // target <500ns
bench_bbo_derivation                 // target <100ns
bench_l2_snapshot_10_levels          // target <1us
bench_l2_snapshot_50_levels          // target <5us
bench_l2_delta_generation            // target <200ns
bench_event_processing_throughput    // target >100K events/sec
bench_ws_serialize_bbo               // target <500ns
bench_ws_serialize_snapshot_10       // target <2us
bench_100_clients_broadcast          // target <100us per event
bench_subscribe_snapshot_latency     // target <1ms
```

No explicit performance targets in MARKETDATA.md.
Derived from system-level E2E latency target (<50us
same machine, TESTING.md §6) and throughput requirements
(10K orders/sec normal, 100K burst).

---

## Integration Points

- Imports `rsx-book` crate for shadow orderbook
  (NETWORK.md §MARKETDATA)
- Receives Fill, OrderInserted, OrderCancelled via SPSC
  from matching engine (CONSISTENCY.md §1)
- Connects as DXS consumer for ME WAL replay on startup
  (DXS.md §8)
- Serves gRPC MarketData.Stream to external clients
  (MARKETDATA.md §service)
- Serves public WS endpoint with BBO/B/D frames
  (WEBPROTO.md §market data)
- System-level: market data streaming in smoke tests
  (TESTING.md §5 smoke)
- Load tests: 100 clients, Zipf symbol distribution
  (TESTING.md §6)
