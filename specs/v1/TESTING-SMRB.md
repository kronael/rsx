# TESTING-SMRB.md — SPSC Ring Buffer Tests

Source specs: [notes/SMRB.md](../../notes/SMRB.md),
[CONSISTENCY.md](CONSISTENCY.md)

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| S1 | SPSC push/pop is wait-free (bounded steps) | SMRB.md |
| S2 | FIFO ordering preserved | SMRB.md |
| S3 | Cache-line padded indices (no false sharing) | SMRB.md |
| S4 | Power-of-2 capacity (bitwise AND) | SMRB.md |
| S5 | No heap allocation after creation | SMRB.md |
| S6 | push_spin bare busy-spin on ring full | CONSISTENCY.md §3 |
| S7 | Per-consumer rings (isolation) | CONSISTENCY.md §1 |
| S8 | Producer stalls on full (backpressure) | WAL.md, CONSISTENCY.md |
| S9 | Acquire/release atomics only (no CAS) | SMRB.md |

---

## Unit Tests

```rust
// core operations
spsc_push_pop_single_item
spsc_push_pop_multiple_items
spsc_fifo_order_preserved
spsc_full_ring_returns_error
spsc_empty_ring_returns_none
spsc_capacity_is_power_of_two
spsc_fill_to_capacity_then_drain
spsc_wraparound_at_capacity_boundary
spsc_alternating_push_pop

// edge cases
spsc_push_after_full_and_one_pop
spsc_pop_after_empty_and_one_push
spsc_capacity_one_element
spsc_u64_index_wraparound
spsc_large_struct_64b_aligned
spsc_zero_copy_semantics

// push_spin
push_spin_returns_immediately_when_space
push_spin_blocks_until_consumer_pops
push_spin_no_data_loss_under_contention

// isolation
per_consumer_ring_independence
slow_consumer_does_not_block_other_rings
```

---

## E2E Tests

```rust
// producer-consumer pair (two threads)
two_thread_1m_messages_no_loss
two_thread_fifo_verified_sequence_numbers
two_thread_producer_faster_than_consumer_backpressure
two_thread_consumer_faster_than_producer_drains

// fan-out (matching engine pattern)
fanout_three_consumers_all_receive_same_events
fanout_slow_consumer_stalls_producer_on_that_ring
fanout_event_routing_fill_to_risk_gateway_mktdata
fanout_bbo_only_to_risk
fanout_order_inserted_only_to_mktdata

// cross-type messages
ring_of_enum_events_fill_bbo_done_inserted
ring_of_fixed_size_wal_records
```

---

## Benchmarks

```rust
bench_spsc_push_pop_latency          // target <100ns round-trip
bench_spsc_throughput_1m_messages    // target >10M msg/sec
bench_push_spin_contended            // measure spin duration
bench_fanout_3_rings_per_event       // target <500ns total
bench_cache_line_false_sharing       // compare padded vs unpadded
bench_large_payload_128b             // OrderSlot-sized messages
```

Targets from SMRB.md:

| Method | Target Latency |
|--------|---------------|
| SPSC ring (rtrb) | ~50-170ns |
| push_spin (ring full, spin) | bounded by consumer pop |

---

## Integration Points

- Matching engine drain_events() fans out to 3+ SPSC rings
  (CONSISTENCY.md drain loop)
- Risk engine main loop polls ME rings (RISK.md §main loop)
- Gateway receives fills/done via SPSC from risk
- Mark price aggregator pushes SourcePrice via SPSC
  (MARK.md §1)
- WAL writer backpressure: buf full triggers stall (DXS.md §3)
- System-level: verify no data loss across component boundaries
  under sustained 100K msg/sec load (TESTING.md load tests)
