# TESTING-DXS.md — DXS (WAL + Streaming) Tests

Source specs: [DXS.md](DXS.md), [WAL.md](WAL.md)

Crate: `rsx-dxs`

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| D1 | Fixed-record format: 16B header + repr(C) payload | DXS.md §1 |
| D2 | CRC32 validation on read, truncate on invalid | DXS.md §1 |
| D3 | Little-endian encoding for all fields | DXS.md §1 |
| D4 | File layout: `wal/{stream_id}/{stream_id}_{first}_{last}.wal` | DXS.md §2 |
| D5 | Rotate at 64MB, retain 10min | DXS.md §2 |
| D6 | WalWriter: monotonic seq, O(1) append | DXS.md §3 |
| D7 | WalWriter: flush every 10ms + fsync | DXS.md §3 |
| D8 | WalWriter: backpressure stall at 2x max_file_size | DXS.md §3 |
| D9 | WalWriter: GC deletes files outside retention | DXS.md §3 |
| D10 | WalReader: open from seq via filename binary search | DXS.md §4 |
| D11 | WalReader: sequential iteration across files | DXS.md §4 |
| D12 | DxsReplay server: gRPC stream from from_seq | DXS.md §5 |
| D13 | DxsReplay: CaughtUp marker then live tail | DXS.md §5 |
| D14 | DxsConsumer: tip persistence every 10ms | DXS.md §6 |
| D15 | DxsConsumer: reconnect with backoff 1/2/4/8/30s | DXS.md §6 |
| D16 | DxsConsumer: resume from tip+1 | DXS.md §6 |
| D17 | Recorder: daily rotation, same fixed-record format | DXS.md §8 |
| D18 | Bounded loss window: 10ms (WAL.md) | WAL.md |
| D19 | Replica lag bound: 100ms, stall if exceeded | WAL.md |
| D20 | Unknown version: stop replay and fail fast (no skip) | DXS.md §1 |
| D21 | CancelReason enum: 6 values (0-5) | DXS.md §1 |
| D22 | No file header, no index, sequential read only | DXS.md §2 |
| D23 | Active file uses temp name `{stream_id}_active.wal` | DXS.md §2 |
| D24 | GC runs on rotation, not on timer | DXS.md §2,3 |
| D25 | Archive fallback when from_seq older than hot retention | DXS.md §2 |
| D26 | Local WAL full: stall producer | WAL.md |
| D27 | Flush lag >10ms: stall producer | WAL.md |
| D28 | Flush on size threshold (1000 records) in addition to 10ms | WAL.md |

---

## Unit Tests

### WAL Record Encoding

```rust
// header
wal_header_encode_decode_roundtrip
wal_header_little_endian_verified
wal_header_crc32_matches_payload

// fixed-record payloads
fill_record_encode_decode_roundtrip
bbo_record_encode_decode_roundtrip
order_inserted_record_roundtrip
order_cancelled_record_roundtrip
order_done_record_roundtrip
config_applied_record_roundtrip
caught_up_record_roundtrip

// edge cases
record_max_payload_64kb
record_crc32_mismatch_detected
record_truncated_header_detected
record_truncated_payload_detected
record_zero_length_payload_valid
record_unknown_version_fails_fast_no_skip

// cancel reason
cancel_reason_all_6_values_roundtrip
cancel_reason_maps_to_correct_semantics
```

### WalWriter

```rust
writer_assigns_monotonic_seq
writer_append_to_buffer_no_io
writer_flush_writes_to_file
writer_flush_calls_fsync
writer_rotation_at_64mb
writer_rotation_renames_with_seq_range
writer_active_file_uses_temp_name
writer_gc_deletes_old_files
writer_gc_preserves_recent_files
writer_backpressure_stalls_at_2x_buffer
writer_empty_flush_no_io
writer_seq_starts_at_1
writer_gc_runs_on_rotation_not_timer
writer_flush_on_size_threshold_1000_records
writer_local_wal_full_stalls_producer
writer_flush_lag_exceeding_10ms_stalls_producer
```

### WalReader

```rust
reader_open_from_seq_finds_correct_file
reader_open_from_seq_0_starts_at_beginning
reader_sequential_iteration_all_records
reader_file_transition_seamless
reader_returns_none_at_eof
reader_returns_none_when_caught_up
reader_skips_to_target_seq_within_file
reader_handles_empty_wal_directory
reader_handles_single_file
reader_handles_multiple_files_sorted
reader_crc32_invalid_truncates_stream
reader_unknown_version_stops_replay_fails_fast
reader_no_file_header_no_index_sequential_only
```

### DxsConsumer

```rust
consumer_loads_tip_from_file
consumer_tip_zero_if_file_missing
consumer_sends_replay_request_tip_plus_1
consumer_advances_tip_per_record
consumer_persists_tip_on_interval
consumer_reconnect_backoff_1_2_4_8_30
consumer_reconnect_resets_on_success
consumer_callback_invoked_per_record
consumer_dedup_by_seq_at_consumer
```

---

## E2E Tests

```rust
// writer + reader roundtrip
write_100_records_read_all_back
write_rotate_read_across_files
write_flush_crash_recover_from_last_fsync
write_gc_then_read_from_retained_range

// replay server + consumer
replay_from_beginning_receives_all
replay_from_mid_receives_subset
replay_caught_up_marker_sent
replay_live_tail_receives_new_records
replay_multiple_consumers_independent
replay_consumer_disconnect_reconnect_resumes

// tip persistence
consumer_crash_resume_from_persisted_tip
consumer_tip_advances_monotonically
consumer_replays_idempotent_no_side_effects

// recorder
recorder_writes_daily_archive_files
recorder_daily_rotation_at_utc_midnight
recorder_archive_format_matches_wal_format

// backpressure
writer_stall_on_buffer_full_then_resume
flush_lag_exceeding_10ms_stalls_producer

// archive fallback
consumer_from_seq_older_than_hot_falls_back_to_archive
archive_to_hot_tail_seamless_transition

// replica lag
replica_lag_exceeding_100ms_stalls_producer
```

---

## Benchmarks

```rust
bench_wal_append_in_memory           // target <200ns
bench_wal_flush_fsync_64kb           // target <1ms
bench_wal_read_sequential_throughput // target >500 MB/s
bench_replay_100k_records            // target <1s
bench_recorder_sustained_write       // target >100K records/s
bench_fill_record_encode             // target <50ns
bench_fill_record_decode             // target <50ns
bench_crc32_compute_128b             // target <20ns
bench_tip_persist_flush              // target <100us
bench_reader_seek_to_seq             // target <1ms
```

Targets from DXS.md §10:

| Operation | Target |
|-----------|--------|
| WAL append (in-memory) | <200ns |
| WAL flush (fsync) | <1ms per 64KB batch |
| WAL read (sequential) | >500 MB/s |
| Replay 100K records | <1s |
| Recorder sustained write | >100K records/s |
| Tip persist | every 10ms, batched |

---

## Integration Points

- Matching engine embeds WalWriter for event persistence
  (ORDERBOOK.md §2.8)
- Matching engine embeds DxsReplay server for downstream
  consumers (DXS.md §5)
- Risk engine connects as DxsConsumer for replay on startup
  (RISK.md §replication)
- Mark aggregator embeds WalWriter + DxsReplay for mark
  prices (MARK.md §1)
- Recorder connects as DxsConsumer for archival (DXS.md §8)
- Market data connects as DxsConsumer for recovery
  (MARKETDATA.md §8)
- WAL backpressure rules propagate to matching engine stall
  (WAL.md, CONSISTENCY.md §3)
- Archive fallback when hot WAL retention exceeded
  (DXS.md §2)
- Cross-host live streaming via gRPC / gRPC over QUIC
  (DXS.md §7)
- SPSC rings for in-process hot-path (ME -> risk, risk ->
  gateway), DXS for cross-host and replay (DXS.md §7,
  NETWORK.md §internal)
