# TESTING-GATEWAY.md — Gateway Tests

Source specs: [NETWORK.md](NETWORK.md), [WEBPROTO.md](WEBPROTO.md),
[RPC.md](RPC.md), [MESSAGES.md](MESSAGES.md)

Binary: `rsx-gateway`

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| G1 | WS overlay: compact JSON, single-letter types | WEBPROTO.md |
| G2 | QUIC + WAL wire format for native clients | NETWORK.md |
| G3 | JWT auth via WS upgrade headers (A fallback) | WEBPROTO.md |
| G4 | UUIDv7 order ID generated at gateway | RPC.md §order-id |
| G5 | LIFO VecDeque pending order tracking | RPC.md §pending |
| G6 | Rate limiting: 10/s per user, 100/s per IP | RPC.md §rate-limit |
| G7 | Ingress backpressure: cap 10k, OVERLOADED | RPC.md §backpressure |
| G8 | Heartbeat: 5s interval, 10s timeout | WEBPROTO.md §H |
| G9 | Order timeout: 10s | RPC.md §timeout |
| G10 | No ACK on order -- first response is update/fill | WEBPROTO.md |
| G11 | Fill streaming: 0+ fills then ORDER_DONE/FAILED | MESSAGES.md §fills |
| G12 | Circuit breaker: 10 failures -> open -> half-open | RPC.md §circuit |
| G13 | Market data WS: S subscribe, X unsubscribe | WEBPROTO.md §S |
| G14 | Liquidation event Q frame to user WS | WEBPROTO.md §Q |
| G15 | Multiplexed quinn QUIC stream to risk engine | NETWORK.md |
| G16 | Config cache synced via CONFIG_APPLIED | MESSAGES.md |
| G17 | Tick/lot pre-validation (fail fast) | ORDERBOOK.md §2.9 |
| G18 | Out-of-order response handling via order_id | RPC.md §pending |
| G19 | Stale order policy: 5 min, client cancels/forgets | RPC.md §timeout |
| G20 | Per-instance throughput cap: 1000 orders/s | RPC.md §rate-limit |
| G21 | Enum validation: Side, TIF, OrderStatus, FailureReason | WEBPROTO.md §enums |
| G22 | Reduce-only (ro) field in N frame (optional, default 0) | WEBPROTO.md §N |
| G23 | Fill fee field: signed int64, negative=rebate | WEBPROTO.md §F |
| G24 | Error frame E: code + msg | WEBPROTO.md §E |
| G25 | No permessage-deflate compression | WEBPROTO.md §frame-shape |
| G26 | Horizontal scaling: user_id hash sharding | NETWORK.md §scaling |
| G27 | Dedup: 5-min window in ME, fresh UUIDv7 on retry | RPC.md §dedup |
| G28 | OrderDone/OrderFailed exactly one per order | MESSAGES.md §completion |
| G29 | Fills precede ORDER_DONE in stream | MESSAGES.md §fill-streaming |
| G30 | Fixed-point price/qty: int64, no float | MESSAGES.md §field-encodings |
| G31 | Exactly one key per WS frame | WEBPROTO.md §frame-shape |

---

## Unit Tests

### WS Protocol Parsing

```rust
// new order
parse_n_frame_all_fields
parse_n_frame_reduce_only_default_0
parse_n_frame_reduce_only_1
parse_n_frame_invalid_side_rejected
parse_n_frame_missing_field_rejected

// cancel
parse_c_frame_by_cid
parse_c_frame_by_oid

// auth fallback
parse_a_frame_jwt_token
parse_a_frame_invalid_token_rejected

// subscribe
parse_s_frame_subscribe_bbo
parse_s_frame_subscribe_depth
parse_x_frame_unsubscribe
parse_x_frame_unsubscribe_all

// heartbeat
parse_h_frame_server_initiated
parse_h_frame_client_echo

// error
parse_e_frame_error_code_and_msg

// liquidation
parse_q_frame_liquidation_all_statuses

// market data (server->client outbound)
parse_bbo_frame_all_fields
parse_b_snapshot_frame
parse_d_delta_frame

// validation
parse_frame_rejects_multiple_keys
parse_frame_rejects_non_letter_key
parse_n_frame_invalid_tif_rejected
```

### WS Protocol Serialization

```rust
serialize_u_frame_order_update
serialize_f_frame_fill
serialize_e_frame_error
serialize_h_frame_heartbeat
serialize_bbo_frame
serialize_b_frame_l2_snapshot
serialize_d_frame_l2_delta
serialize_q_frame_liquidation
serialize_s_frame_subscribe
serialize_x_frame_unsubscribe
```

### Enum Validation

```rust
enum_side_valid_0_1_only
enum_tif_valid_0_1_2_only
enum_order_status_valid_0_1_2_3
enum_failure_reason_valid_0_through_7
enum_unknown_value_rejected
```

### Fill Fee Handling

```rust
fill_fee_positive_taker
fill_fee_negative_rebate_maker
fill_fee_zero
fill_fee_forwarded_in_f_frame
```

### Reduce-Only

```rust
n_frame_ro_default_zero_when_absent
n_frame_ro_1_maps_to_quic_reduce_only
```

### Fixed-Point Conversion

```rust
price_float_to_fixed_point_correct
qty_float_to_fixed_point_correct
price_fractional_tick_rejected
qty_fractional_lot_rejected
```

### UUIDv7 Order ID

```rust
uuid_v7_monotonic_within_millisecond
uuid_v7_globally_unique_across_instances
uuid_v7_time_sortable
uuid_v7_16_bytes_binary
```

### Pending Order Tracking

```rust
pending_push_back_new_order
pending_pop_back_lifo_match
pending_linear_scan_on_mismatch
pending_remove_by_order_id
pending_empty_after_all_removed
pending_timeout_removes_stale_order
pending_multiple_orders_same_user
```

### Rate Limiting

```rust
rate_limit_allows_under_threshold
rate_limit_rejects_at_threshold
rate_limit_refills_over_time
rate_limit_per_user_independent
rate_limit_per_ip_independent
rate_limit_10_per_sec_per_user
rate_limit_100_per_sec_per_ip
rate_limit_1000_per_sec_per_instance
```

### Ingress Backpressure

```rust
backpressure_accepts_under_10k
backpressure_rejects_at_10k_overloaded
backpressure_resumes_after_drain
```

### Circuit Breaker

```rust
circuit_closed_allows_orders
circuit_open_after_10_failures
circuit_open_rejects_immediately
circuit_half_open_after_30s
circuit_half_open_success_closes
circuit_half_open_failure_reopens
```

### Heartbeat

```rust
heartbeat_sent_every_5s
heartbeat_timeout_closes_at_10s
heartbeat_client_response_resets_timer
```

### Pre-validation

```rust
tick_size_validation_rejects_early
lot_size_validation_rejects_early
symbol_not_found_rejects_early
config_cache_updated_on_config_applied
```

---

## E2E Tests

### Order Lifecycle

```rust
ws_new_order_fill_update_complete
ws_new_order_rest_cancel_done
ws_new_order_rejected_insufficient_margin
ws_new_order_rejected_invalid_tick
ws_new_order_rejected_overloaded
ws_new_order_timeout_returns_error
quic_new_order_fill_done_complete
quic_cancel_order_done
ws_reduce_only_order_lifecycle
ws_fill_with_fee_forwarded
ws_error_frame_sent_on_invalid_input
```

### Multi-User

```rust
100_concurrent_ws_sessions
10_users_rapid_fire_orders
user_disconnect_cleans_pending
user_reconnect_fresh_session
two_users_fill_notification_both_receive
```

### Market Data WS

```rust
subscribe_bbo_receives_updates
subscribe_depth_receives_snapshot_then_deltas
unsubscribe_stops_updates
unsubscribe_all_clears_subscriptions
seq_gap_triggers_resnapshot
multiple_symbols_concurrent_streams
```

### Failure Modes

```rust
risk_engine_disconnect_circuit_opens
risk_engine_reconnect_circuit_closes
matching_engine_timeout_order_failed
network_partition_pending_orders_failed
duplicate_order_id_rejected_by_me
stale_order_5min_no_update_client_cancels
```

### Stream Ordering

```rust
fills_precede_order_done_in_stream
exactly_one_completion_per_order
order_done_or_failed_never_both
no_permessage_deflate_negotiated
```

### Liquidation Notification

```rust
liquidation_event_routed_to_correct_user
liquidation_q_frame_all_statuses
liquidation_event_fire_and_forget
```

---

## Benchmarks

```rust
bench_ws_parse_n_frame              // target <500ns
bench_ws_serialize_f_frame          // target <500ns
bench_uuid_v7_generation            // target <50ns
bench_pending_lifo_pop_5_orders     // target <100ns
bench_pending_linear_scan_10        // target <100ns
bench_rate_limit_check              // target <50ns
bench_quic_order_serialization      // target <1us
bench_100_concurrent_sessions       // target stable throughput
bench_1000_orders_sec_per_user      // target <1ms per order
bench_ws_parse_c_frame              // target <200ns
bench_ws_parse_a_frame              // target <500ns
bench_backpressure_reject           // target <100ns
bench_fill_fee_extraction           // target <50ns
bench_fixed_point_conversion        // target <50ns
```

Targets from NETWORK.md:

| Path | Target |
|------|--------|
| External -> Gateway | ~1-10ms (after TLS) |
| Gateway -> Risk (UDS) | ~50-100us per message |
| Gateway -> Risk (TCP) | ~100-300us per message |

---

## Integration Points

- Single multiplexed quinn QUIC stream to risk engine
  (NETWORK.md)
- Receives fills/done/failed from risk via SPSC or QUIC
- Receives liquidation events from risk SPSC
  (WEBPROTO.md §Q)
- Forwards CONFIG_APPLIED to local config cache
  (MESSAGES.md §ConfigApplied)
- Public market data WS endpoint separate from trading WS
  (WEBPROTO.md §market data)
- System-level: full order lifecycle gateway -> risk -> ME
  (TESTING.md §2 e2e)
- Load tests: 10K concurrent users, 100K orders/sec burst
  (TESTING.md §6 load tests)
- Fixed-point price/qty conversion at gateway ingress
  (MESSAGES.md §field-encodings)
- Horizontal scaling via user_id hash sharding
  (NETWORK.md §gateway-scaling)
