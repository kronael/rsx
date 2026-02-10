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
| MD4 | WS public feed (BBO/L2/trades) | WEBPROTO.md |
| MD5 | Subscribe by symbol_id list + depth | WEBPROTO.md §market data |
| MD6 | L2Snapshot sent on initial subscribe | WEBPROTO.md §market data |
| MD7 | Deltas after snapshot, seq monotonic per symbol | WEBPROTO.md §market data |
| MD8 | Backpressure: drop deltas, resend snapshot | MARKETDATA.md §notes |
| MD9 | Seq gap -> client re-subscribes for snapshot | WEBPROTO.md §market data |
| MD10 | Public endpoint, no auth | MARKETDATA.md |
| MD11 | Single-threaded, dedicated core, busy-spin | NETWORK.md §MARKETDATA |
| MD12 | Non-blocking epoll for WS I/O (no Tokio) | NETWORK.md §MARKETDATA |
| MD13 | CMP/UDP input from matching engine | NETWORK.md §MARKETDATA |
| MD14 | Recovery via DXS replay from ME WAL (planned; not in v1) | DXS.md §8 |
| MD15 | WS JSON: BBO, B (snapshot), D (delta), S, X | WEBPROTO.md |
| MD16 | Event routing: Fill + OrderInserted + Cancelled | CONSISTENCY.md §1 |
| MD17 | WS schema mirrors JSON (B/D/BBO) | WEBPROTO.md |
| MD18 | BBO includes order count per side (bid_count, ask_count) | MARKETDATA.md §messages |
| MD19 | Snapshot consistency: point-in-time best effort | MARKETDATA.md §transport |
| MD20 | OrderDone NOT routed to market data | CONSISTENCY.md §1 table |
| MD21 | MktData derives own BBO from shadow book (not ME BBO) | CONSISTENCY.md §1 |
| MD22 | WS seq gap: u jumps >1 triggers re-subscribe | WEBPROTO.md §market data |
| MD23 | `u` field is WS alias for `seq` | WEBPROTO.md §market data |
| MD24 | Server sends B snapshot on subscribe before D deltas | WEBPROTO.md §market data |
| MD25 | Trades derived from fill events | NETWORK.md §MARKETDATA |
| MD26 | Subscribe depth parameter: 10, 25, 50 | MARKETDATA.md §subscribe |

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
shadow_book_order_done_not_applied
shadow_book_uses_rsx_book_crate
```

### BBO Derivation

```rust
bbo_update_on_best_bid_change
bbo_update_on_best_ask_change
bbo_no_update_if_unchanged
bbo_includes_count_and_qty
bbo_correct_after_fill_at_best
bbo_correct_after_cancel_at_best
bbo_includes_bid_count_and_ask_count
bbo_derived_from_shadow_book_not_me_bbo
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
subscribe_depth_10_25_50
subscribe_send_snapshot_false_skips_snapshot
```

### Backpressure

```rust
slow_client_deltas_dropped
slow_client_gets_fresh_snapshot
slow_client_does_not_block_other_clients
backpressure_threshold_configurable
```

### Trade Derivation

```rust
trade_from_fill_event_correct
trade_price_and_qty_from_fill
trade_taker_side_preserved
trade_timestamp_from_fill
```

### Event Routing

```rust
fill_event_routed_to_marketdata
order_inserted_routed_to_marketdata
order_cancelled_routed_to_marketdata
order_done_not_routed_to_marketdata
bbo_event_not_routed_to_marketdata
```

### WS Frame Format

```rust
ws_bbo_frame_includes_u_seq_field
ws_b_snapshot_includes_u_seq_field
ws_d_delta_includes_u_seq_field
ws_u_field_equals_quic_seq
ws_seq_gap_detected_when_u_jumps
```

### Snapshot Consistency

```rust
snapshot_point_in_time_consistent
snapshot_before_deltas_on_subscribe
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
ws_s_subscribe_frame_parsed
ws_x_unsubscribe_frame_parsed

// recovery (planned; DXS replay not wired in v1)
dxs_replay_rebuilds_shadow_book
recovery_from_me_wal_then_live
recovery_snapshot_sent_after_catchup

// failure modes
me_disconnect_shadow_book_stale
me_reconnect_resumes_events
me_restart_replay_from_wal

// WS feed
quic_subscribe_by_symbol_id_list
quic_subscribe_with_depth_parameter
quic_send_snapshot_true_sends_snapshot_first
quic_send_snapshot_false_skips_snapshot
quic_mirrors_ws_json_schema

// trades
fill_event_produces_trade_to_client
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
bench_trade_derivation_from_fill    // target <100ns
bench_event_routing_filter          // target <50ns
```

No explicit performance targets in MARKETDATA.md.
Derived from system-level E2E latency target (<50us
same machine, TESTING.md §6) and throughput requirements
(10K orders/sec normal, 100K burst).

---

## Integration Points

- Imports `rsx-book` crate for shadow orderbook
  (NETWORK.md §MARKETDATA)
- Receives Fill, OrderInserted, OrderCancelled via CMP/UDP
  from matching engine (CONSISTENCY.md §1)
- Connects as DXS consumer for ME WAL replay on startup
  (DXS.md §8)
- Serves WS marketdata feed to external clients
  (MARKETDATA.md §service)
- Serves public WS endpoint with BBO/B/D frames
  (WEBPROTO.md §market data)
- System-level: market data streaming in smoke tests
  (TESTING.md §5 smoke)
- Load tests: 100 clients, Zipf symbol distribution
  (TESTING.md §6)
- Does NOT receive OrderDone or BBO events from ME
  (CONSISTENCY.md §1 event routing table)
- Derives own BBO from shadow book, not from ME BBO event
  (CONSISTENCY.md §1)
- WS `u` field maps to `seq` for gap detection
  (WEBPROTO.md §market data)
