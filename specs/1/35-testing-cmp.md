---
status: shipped
---

# TESTING-CMP: CMP Protocol Test Specification

Version: 1.0
Status: Draft
Depends on: CMP.md, DXS.md, TILES.md, NETWORK.md

## Table of Contents

- [1. Requirements Checklist](#1-requirements-checklist)
- [2. Unit Tests](#2-unit-tests)
- [3. E2E Tests](#3-e2e-tests)
- [4. Benchmarks](#4-benchmarks)
- [5. Integration Points](#5-integration-points)
- [6. monoio / io_uring Test Considerations](#6-monoio--io_uring-test-considerations)
- [7. Test File Organization](#7-test-file-organization)
- [8. Coverage Matrix](#8-coverage-matrix)

---

## 1. Requirements Checklist

Every requirement maps to one or more tests. Status
tracks implementation progress.

### Wire Format

| ID | Requirement | Test(s) | Status |
|----|-------------|---------|--------|
| C1 | 16B header + repr(C, align(64)) payload, same as WAL | `control_record_type_values_match_spec`, `*_size_is_64_bytes` | ☐ |
| C2 | All fields little-endian, compile-time endian assert | `*_fields_little_endian` | ☐ |
| C3 | CRC32 in header covers payload, validated on read | `crc32_covers_payload_not_header`, `receiver_validates_crc32_discards_invalid` | ☐ |
| C4 | MAX_PAYLOAD = 64KB, reject before allocating | `sender_rejects_payload_exceeding_max`, `receiver_rejects_len_exceeding_max_payload` | ☐ |
| C5 | size_of asserts for all wire types (StatusMessage, Nak, Heartbeat = 64B each) | `status_message_size_is_64_bytes`, `nak_size_is_64_bytes`, `heartbeat_size_is_64_bytes` | ☐ |

### CMP/UDP Sender

| ID | Requirement | Test(s) | Status |
|----|-------------|---------|--------|
| C6 | One WAL record per UDP datagram | `sender_one_record_per_udp_datagram` | ☐ |
| C7 | Monotonic next_seq, assigned per send | `sender_assigns_monotonic_seq` | ☐ |
| C8 | Heartbeat sent every 10ms | `sender_heartbeat_sent_every_10ms`, `sender_heartbeat_contains_highest_seq` | ☐ |
| C9 | Flow control: won't send beyond consumption_seq + receiver_window | `sender_respects_flow_control_window`, `sender_stalls_when_window_exhausted` | ☐ |
| C10 | On Nak: fetch from WAL, resend as normal data | `sender_handles_nak_fetches_from_wal`, `sender_retransmit_is_normal_data_record` | ☐ |
| C12 | Retransmits are normal data records (no special type) | `sender_retransmit_is_normal_data_record` | ☐ |
| C13 | Sender stalls when flow control window exhausted | `sender_stalls_when_window_exhausted`, `zero_window_sender_fully_stalled` | ☐ |
| C14 | Sender updates peer state on StatusMessage receipt | `sender_updates_peer_state_on_status_msg`, `sender_resumes_after_status_msg_opens_window` | ☐ |

### CMP/UDP Receiver

| ID | Requirement | Test(s) | Status |
|----|-------------|---------|--------|
| C15 | Sequential seq expected per stream | `receiver_delivers_sequential_records_in_order` | ☐ |
| C16 | Gap detection via Heartbeat highest_seq | `receiver_detects_gap_from_heartbeat`, `receiver_handles_heartbeat_no_gap` | ☐ |
| C17 | Nak sent immediately on gap detection | `receiver_sends_nak_immediately_on_gap`, `receiver_nak_specifies_from_seq_and_count` | ☐ |
| C18 | StatusMessage sent every 10ms | `receiver_sends_status_message_every_10ms` | ☐ |
| C19 | StatusMessage contains consumption_seq + window | `receiver_status_contains_consumption_seq`, `receiver_status_contains_window_size` | ☐ |
| C20 | Reorder buffer bounded at 512 slots | `receiver_reorder_buf_bounded_at_512` | ☐ |
| C21 | Reorder buffer overflow = drop/error | `receiver_reorder_buf_overflow_drops`, `reorder_buf_at_512_limit_then_overflow` | ☐ |
| C22 | In-order delivery to application after reorder | `receiver_reorder_buf_delivers_when_gap_filled`, `reorder_2_packets_delivered_in_seq_order` | ☐ |
| C23 | Duplicate seq ignored (already seen) | `receiver_ignores_duplicate_seq`, `duplicate_packet_ignored_no_double_delivery` | ☐ |
| C24 | Records delivered to callback in seq order | `receiver_delivers_sequential_records_in_order`, `send_100_records_receive_all_in_order` | ☐ |

### TCP Cold Path

| ID | Requirement | Test(s) | Status |
|----|-------------|---------|--------|
| C25 | Client sends ReplayRequest as WAL record | `tcp_replay_request_encode_decode_roundtrip`, `tcp_replay_request_size_is_64_bytes` | ☐ |
| C26 | Server streams write_all(header) + write_all(payload) | `tcp_server_streams_header_then_payload` | ☐ |
| C27 | Client reads read_exact(16) + read_exact(len) | `tcp_client_reads_exact_header_then_payload` | ☐ |
| C28 | RECORD_CAUGHT_UP sent when replay complete | `tcp_server_sends_caught_up_at_end_of_replay` | ☐ |
| C29 | After CaughtUp, server transitions to live broadcast | `tcp_server_transitions_to_live_after_caught_up`, `tcp_replay_caught_up_then_live_tail` | ☐ |
| C30 | Optional TLS via rustls (config flag) | `tcp_tls_handshake_with_rustls`, `tcp_tls_disabled_when_config_false` | ☐ |
| C31 | Reconnect with exponential backoff 1/2/4/8/30s | `tcp_client_reconnect_backoff_1_2_4_8_30`, `tcp_client_reconnect_resets_on_success` | ☐ |
| C32 | Resume from tip+1 on reconnect | `tcp_client_resumes_from_tip_plus_1`, `tcp_disconnect_reconnect_resumes_from_tip` | ☐ |

### Control Message Encoding

| ID | Requirement | Test(s) | Status |
|----|-------------|---------|--------|
| C33 | RECORD_STATUS_MESSAGE = 0x10 | `control_record_type_values_match_spec` | ☐ |
| C34 | RECORD_NAK = 0x11 | `control_record_type_values_match_spec` | ☐ |
| C35 | RECORD_HEARTBEAT = 0x12 | `control_record_type_values_match_spec` | ☐ |
| C36 | StatusMessage fields: stream_id, consumption_seq, receiver_window | `status_message_encode_decode_roundtrip`, `status_message_fields_little_endian` | ☐ |
| C37 | Nak fields: stream_id, from_seq, count | `nak_encode_decode_roundtrip`, `nak_fields_little_endian` | ☐ |
| C38 | Heartbeat fields: stream_id, highest_seq | `heartbeat_encode_decode_roundtrip`, `heartbeat_fields_little_endian` | ☐ |

### Safety

| ID | Requirement | Test(s) | Status |
|----|-------------|---------|--------|
| C39 | ptr::read on Copy types only, never transmute | code review (compile-time) | ☐ |
| C40 | Padding bytes zeroed in all control messages | `padding_bytes_zeroed_in_all_control_msgs` | ☐ |
| C41 | Unknown record types ignored by receiver | `receiver_ignores_unknown_record_type` | ☐ |
| C42 | Invalid CRC = discard (UDP) or disconnect (TCP) | `receiver_validates_crc32_discards_invalid`, `tcp_invalid_crc_disconnects` | ☐ |
| C43 | len > MAX_PAYLOAD rejected before allocation | `sender_rejects_payload_exceeding_max`, `receiver_rejects_len_exceeding_max_payload` | ☐ |
| C44 | Zero heap allocation on hot path | `sender_zero_heap_in_send_loop`, `receiver_zero_heap_in_recv_loop`, `bench_zero_alloc_send_recv_loop` | ☐ |
| C45 | All control structs repr(C, align(64)) | `*_size_is_64_bytes` (compile-time size_of asserts) | ☐ |

## 2. Unit Tests

### 2.1 Control Message Encoding

File: `rsx-dxs/tests/cmp_encoding_test.rs`

```
status_message_encode_decode_roundtrip
    Build StatusMessage with known fields, encode to
    bytes, decode back, assert all fields match.

status_message_size_is_64_bytes
    static_assert: size_of::<StatusMessage>() == 64.

status_message_fields_little_endian
    Encode StatusMessage, read raw bytes at known
    offsets, verify little-endian byte order.

nak_encode_decode_roundtrip
    Build Nak with known fields, encode, decode, assert.

nak_size_is_64_bytes
    static_assert: size_of::<Nak>() == 64.

nak_fields_little_endian
    Encode Nak, read raw bytes, verify LE order.

heartbeat_encode_decode_roundtrip
    Build Heartbeat, encode, decode, assert.

heartbeat_size_is_64_bytes
    static_assert: size_of::<Heartbeat>() == 64.

heartbeat_fields_little_endian
    Encode Heartbeat, read raw bytes, verify LE order.

control_record_type_values_match_spec
    Assert RECORD_STATUS_MESSAGE == 0x10,
    RECORD_NAK == 0x11, RECORD_HEARTBEAT == 0x12.

padding_bytes_zeroed_in_all_control_msgs
    Encode each control message, read padding byte
    ranges, assert all zero.

crc32_covers_payload_not_header
    Encode record, compute CRC32 over payload only,
    assert matches header.crc field. Mutate header
    byte, recompute -- CRC unchanged. Mutate payload
    byte -- CRC mismatch.
```

### 2.2 CmpSender

File: `rsx-dxs/tests/cmp_sender_test.rs`

```
sender_assigns_monotonic_seq
    Send 10 records, capture seq from each datagram,
    assert strictly increasing.

sender_heartbeat_sent_every_10ms
    Create sender, advance mock clock by 25ms, assert
    2-3 heartbeats sent.

sender_heartbeat_contains_highest_seq
    Send 5 records, trigger heartbeat, assert
    heartbeat.highest_seq == 5.

sender_respects_flow_control_window
    Set receiver_window=3, consumption_seq=0. Send 4
    records. Assert 3 sent, 4th blocked.

sender_stalls_when_window_exhausted
    Set window=0. Attempt send. Assert send returns
    WouldBlock/Stall.

sender_updates_peer_state_on_status_msg
    Inject StatusMessage with consumption_seq=10,
    window=5. Assert sender.peer_consumption_seq==10,
    sender.peer_window==5.

sender_handles_nak_fetches_from_wal
    Send 5 records (written to WAL). Inject Nak for
    seq 2..4. Assert sender reads seq 2,3,4 from WAL
    and retransmits.

sender_retransmit_is_normal_data_record
    Trigger retransmit via Nak. Capture datagram.
    Assert record_type matches original, not a special
    retransmit type.

sender_multiple_naks_coalesced
    Send 3 Naks for overlapping ranges within 1ms.
    Assert coalesced into single fetch covering union.

sender_resumes_after_status_msg_opens_window
    Stall sender (window exhausted). Inject StatusMessage
    opening window. Assert queued record sent.

sender_one_record_per_udp_datagram
    Send 5 records. Capture 5 datagrams. Assert each
    contains exactly one record.

sender_rejects_payload_exceeding_max
    Attempt send with len > 64KB. Assert error returned,
    no datagram sent.

sender_zero_heap_in_send_loop
    Run send loop for 1000 records with
    #[global_allocator] counting allocator. Assert
    0 allocations during loop (setup excluded).
```

### 2.3 CmpReceiver

File: `rsx-dxs/tests/cmp_receiver_test.rs`

```
receiver_delivers_sequential_records_in_order
    Feed seq 1,2,3,4,5. Assert callback receives
    1,2,3,4,5 in order.

receiver_detects_gap_from_heartbeat
    Feed seq 1,2. Inject Heartbeat highest_seq=5.
    Assert gap detected for 3,4,5.

receiver_sends_nak_immediately_on_gap
    Feed seq 1,2. Inject Heartbeat highest_seq=5.
    Assert Nak sent with from_seq=3, count=3.

receiver_sends_status_message_every_10ms
    Create receiver, advance mock clock by 25ms.
    Assert 2-3 StatusMessages sent.

receiver_status_contains_consumption_seq
    Feed seq 1,2,3. Trigger StatusMessage. Assert
    consumption_seq == 3.

receiver_status_contains_window_size
    Create receiver with window=512. Trigger
    StatusMessage. Assert receiver_window == 512.

receiver_reorder_buf_holds_out_of_order
    Feed seq 3 (gap at 2). Assert not delivered,
    held in reorder buffer.

receiver_reorder_buf_delivers_when_gap_filled
    Feed seq 3, then seq 2. Assert callback receives
    2, then 3 in order.

receiver_reorder_buf_bounded_at_512
    Fill reorder buffer with 512 out-of-order records.
    Assert buffer at capacity.

receiver_reorder_buf_overflow_drops
    Fill 512 slots. Feed 513th out-of-order record.
    Assert dropped/error.

receiver_ignores_duplicate_seq
    Feed seq 1,2,3. Feed seq 2 again. Assert callback
    receives 1,2,3 only (no duplicate delivery).

receiver_ignores_unknown_record_type
    Feed datagram with record_type=0xFF. Assert no
    crash, no delivery, datagram discarded.

receiver_validates_crc32_discards_invalid
    Feed datagram with corrupted CRC. Assert discarded,
    no delivery.

receiver_rejects_len_exceeding_max_payload
    Feed datagram with header.len > 64KB. Assert
    rejected before allocation.

receiver_handles_heartbeat_no_gap
    Feed seq 1,2,3. Inject Heartbeat highest_seq=3.
    Assert no Nak sent.

receiver_gap_detection_multiple_missing
    Feed seq 1, skip 2-6, feed 7. Inject Heartbeat
    highest_seq=7. Assert Nak covers 2..6.

receiver_nak_specifies_from_seq_and_count
    Trigger gap at seq 5-8. Assert Nak.from_seq==5,
    Nak.count==4.

receiver_zero_heap_in_recv_loop
    Run recv loop for 1000 records with counting
    allocator. Assert 0 allocations during loop.
```

### 2.4 TCP Replication

File: `rsx-dxs/tests/tcp_repl_test.rs`

```
tcp_replay_request_encode_decode_roundtrip
    Build ReplayRequest, encode, decode, assert fields.

tcp_replay_request_size_is_64_bytes
    static_assert: size_of::<ReplayRequest>() == 64.

tcp_server_streams_header_then_payload
    Start mock server, send one record. Client reads
    16B header first, then payload. Assert correct.

tcp_client_reads_exact_header_then_payload
    Feed partial header (8B), then rest (8B), then
    payload. Assert client waits for exact bytes.

tcp_server_sends_caught_up_at_end_of_replay
    Server replays 5 records. Assert 6th message is
    RECORD_CAUGHT_UP.

tcp_server_transitions_to_live_after_caught_up
    After CaughtUp, server writes new WAL record.
    Assert client receives it (live tail).

tcp_client_reconnect_backoff_1_2_4_8_30
    Simulate 5 failures. Assert delays are 1s, 2s,
    4s, 8s, 30s (capped).

tcp_client_reconnect_resets_on_success
    Fail twice (1s, 2s backoff), succeed, fail again.
    Assert next backoff resets to 1s.

tcp_client_resumes_from_tip_plus_1
    Client consumes to seq=10. Disconnect. Reconnect.
    Assert ReplayRequest.from_seq == 11.

tcp_tls_handshake_with_rustls
    Start TLS server with self-signed cert. Client
    connects with rustls. Assert handshake completes.

tcp_tls_disabled_when_config_false
    Set tls_enabled=false. Assert plain TCP connection.

tcp_invalid_crc_disconnects
    Server sends record with bad CRC over TCP. Assert
    client disconnects.
```

## 3. E2E Tests

All e2e tests use real CmpSender + CmpReceiver over
loopback UDP/TCP. Fault injection via trait-based mock
socket (not tc-netem).

### 3.1 Happy Path

File: `rsx-dxs/tests/cmp_e2e_test.rs`

```
send_100_records_receive_all_in_order
    Sender sends 100 records. Receiver collects all.
    Assert all 100 delivered in seq order.

send_burst_1000_records_all_delivered
    Burst-send 1000 records. Assert all delivered,
    no loss, correct order.

sender_receiver_steady_state_10ms_heartbeats
    Run sender+receiver for 100ms. Assert heartbeats
    and status messages exchanged at ~10ms intervals.
```

### 3.2 Gap Detection + Recovery

```
drop_1_packet_nak_sent_retransmit_fills_gap
    Drop seq 5. Receiver detects gap on heartbeat,
    sends Nak. Sender retransmits from WAL. Assert
    all records delivered.

drop_5_consecutive_nak_covers_all_missing
    Drop seq 10-14. Assert Nak covers from_seq=10,
    count=5. All retransmitted and delivered.

drop_packets_at_3_positions_all_recovered
    Drop seq 3, 7, 12. Assert 3 Naks sent (or
    coalesced). All gaps recovered.

drop_nak_sender_retransmits_on_next_nak
    Drop the Nak itself. Next heartbeat triggers
    new gap detection + Nak. Assert recovery.

drop_heartbeat_gap_detected_on_next_heartbeat
    Drop one heartbeat. Gap detected on next
    heartbeat (20ms later). Assert recovery.

drop_status_message_sender_uses_stale_window
    Drop StatusMessage. Sender continues with stale
    window. Assert no data loss (may stall earlier).
```

### 3.3 Flow Control + Backpressure

```
slow_receiver_sender_stalls_at_window_limit
    Receiver processes slowly (1 record/10ms). Sender
    sends fast. Assert sender stalls at window limit.

receiver_window_opens_sender_resumes
    Stall sender. Receiver processes records (opens
    window). Assert sender resumes.

zero_window_sender_fully_stalled
    Set initial window=0. Assert sender sends zero
    data records (heartbeats still sent).

window_update_via_status_message_unblocks
    Stall sender. Inject StatusMessage with larger
    window. Assert sender unblocks.
```

### 3.4 Reorder

```
reorder_2_packets_delivered_in_seq_order
    Deliver seq 2 before seq 1. Assert application
    receives 1 then 2.

reorder_10_packets_delivered_in_seq_order
    Reverse-deliver seq 10..1. Assert application
    receives 1..10 in order.

reorder_buf_at_512_limit_then_overflow
    Deliver 512 out-of-order records (gap at seq 1).
    Assert buffered. Deliver 513th. Assert overflow
    error.
```

### 3.5 TCP Cold Path E2E

```
tcp_replay_from_beginning_receives_all
    Write 100 records to WAL. Client requests replay
    from seq=1. Assert all 100 received.

tcp_replay_from_mid_receives_subset
    Write 100 records. Client requests from seq=50.
    Assert 51 records received (50..100 + CaughtUp).

tcp_replay_caught_up_then_live_tail
    Replay existing records. After CaughtUp, write
    new record. Assert client receives it live.

tcp_disconnect_reconnect_resumes_from_tip
    Client at seq=50. Kill connection. Reconnect.
    Assert ReplayRequest starts at 51.

tcp_tls_full_replay_cycle
    TLS-enabled server. Client replays, receives
    CaughtUp, live tails. Assert all works over TLS.

tcp_multiple_consumers_independent_streams
    Two clients replay same WAL. Assert independent
    progress, no interference.
```

### 3.6 Fault Injection

File: `rsx-dxs/tests/cmp_fault_test.rs`

```
corrupted_crc_discarded_gap_detected_nak_sent
    Corrupt CRC on seq 5. Receiver discards it.
    Heartbeat triggers Nak. Retransmit fills gap.

corrupted_header_discarded_no_crash
    Send datagram with garbage header. Assert
    receiver discards, no panic.

duplicate_packet_ignored_no_double_delivery
    Send seq 5 twice. Assert delivered exactly once.

delayed_duplicate_after_retransmit_ignored
    Retransmit fills gap. Original (delayed) arrives
    later. Assert ignored.

random_5pct_loss_sustained_throughput_stable
    MockSocket drops 5% randomly. Send 10000 records.
    Assert all delivered (via retransmit). Measure
    throughput degradation < 20%.
```

### 3.7 Long-Running Stability

```
sustained_1m_messages_no_leak_no_drift
    Send 1M records through sender+receiver. Assert
    no memory growth (RSS delta < 1MB), seq monotonic,
    consumption_seq == 1M at end.

seq_monotonic_over_1m_messages
    Send 1M records. Assert every delivered seq >
    previous delivered seq.

tip_persistence_survives_crash_restart
    Consumer at seq=500. Kill process. Restart.
    Assert resumes from persisted tip (seq >= 500).
```

## 4. Benchmarks

File: `rsx-dxs/benches/cmp_bench.rs`

All benchmarks use Criterion. monoio runtime for async
benchmarks. Measure userspace time only.

```
bench_status_message_encode          target: <50ns
    Encode StatusMessage 1M times. Report p50/p99.

bench_status_message_decode          target: <50ns
    Decode StatusMessage from bytes 1M times.

bench_nak_encode                     target: <50ns
    Encode Nak 1M times.

bench_nak_decode                     target: <50ns
    Decode Nak from bytes 1M times.

bench_heartbeat_encode               target: <50ns
    Encode Heartbeat 1M times.

bench_heartbeat_decode               target: <50ns
    Decode Heartbeat from bytes 1M times.

bench_cmp_send_udp_loopback          target: <10us RTT
    Send record via CmpSender, receive via CmpReceiver
    over loopback UDP. Measure RTT.

bench_cmp_send_recv_1m_sustained     target: >1M msg/s
    Send 1M records through sender+receiver. Report
    messages/second.

bench_tcp_replay_100k_records        target: <1s
    Write 100K records to WAL. TCP replay all. Measure
    wall time.

bench_tcp_sustained_throughput       target: >500K msg/s
    TCP live tail, sustained send. Report msg/s.

bench_reorder_buf_insert_lookup      target: <100ns
    Insert + lookup in reorder buffer, 1M iterations.

bench_gap_detect_to_retransmit       target: <50us
    From gap detection to retransmit datagram sent.
    Measure end-to-end latency.

bench_nak_to_recovery_latency        target: <100us
    From Nak receipt at sender to retransmitted record
    delivered at receiver.

bench_flow_control_stall_resume      target: <1ms
    From window=0 stall to StatusMessage receipt to
    first record sent. Measure resume latency.

bench_zero_alloc_send_recv_loop      target: 0 heap
    Run 10K send+recv with counting allocator. Assert
    exactly 0 heap allocations in loop body.
```

### Performance Target Summary

From CMP.md §9:

| Operation | Target |
|-----------|--------|
| CMP message encode | <50ns (memcpy) |
| CMP message decode | <50ns (ptr::read) |
| UDP round-trip (same machine) | <10us |
| UDP round-trip (same datacenter) | <50us |
| TCP round-trip (same machine) | <100us |
| TCP round-trip (cross-datacenter) | <1ms |
| UDP sustained throughput | >1M msg/s |
| TCP sustained throughput | >500K msg/s |

## 5. Integration Points

| Integration | Spec Reference | Test Coverage |
|-------------|----------------|---------------|
| Gateway -> Risk (CmpSender) | NETWORK.md | `cmp_e2e_test.rs` |
| Risk -> ME (CmpSender) | NETWORK.md | `cmp_e2e_test.rs` |
| ME -> Risk (CmpSender) | NETWORK.md | `cmp_e2e_test.rs` |
| Risk -> Gateway (CmpSender) | NETWORK.md | `cmp_e2e_test.rs` |
| CmpSender reads WAL for retransmit | DXS.md §3,4 | `sender_handles_nak_fetches_from_wal` |
| DxsConsumer uses TCP replication | DXS.md §5,6 | `tcp_repl_test.rs` |
| Recorder uses TCP replication | DXS.md §8 | `tcp_replay_from_beginning_receives_all` |
| Marketdata recovery via TCP | MARKETDATA.md §8 | `tcp_replay_caught_up_then_live_tail` |
| SPSC intra-process, CMP inter-process | TILES.md | architectural (no direct test) |

## 6. monoio / io_uring Test Considerations

- Use `#[monoio::test]` for all async tests
- Each test gets its own monoio runtime (no shared state)
- UDP send/recv via `monoio::net::UdpSocket`
- TCP via `monoio::net::TcpStream` / `TcpListener`
- No tokio in test harness
- Fault injection via wrapper trait over socket (drop,
  reorder, delay, corrupt) -- not tc-netem
- Benchmarks: Criterion with monoio runtime, measure
  userspace time only (exclude kernel scheduling)

## 7. Test File Organization

```
rsx-dxs/tests/
    cmp_encoding_test.rs    control message encode/decode
    cmp_sender_test.rs      CmpSender unit tests
    cmp_receiver_test.rs    CmpReceiver unit tests
    cmp_e2e_test.rs         sender+receiver integration
    cmp_fault_test.rs       fault injection scenarios
    tcp_repl_test.rs        TCP replication tests
    common/mod.rs           shared test helpers

rsx-dxs/benches/
    cmp_bench.rs            Criterion benchmarks
```

## 8. Coverage Matrix

Every requirement C1-C45 maps to at least one test.
Every CMP.md section has corresponding test coverage.
Benchmarks cover all 8 performance targets from CMP.md §9.
Fault injection covers 5 categories: loss, burst loss,
reorder, corruption, duplication (per Aeron/RFC 8085).
