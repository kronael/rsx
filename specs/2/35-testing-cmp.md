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

See `rsx-dxs/tests/cmp_encoding_test.rs` — covers control message
encode/decode roundtrips, size asserts, little-endian field layout,
record type constant values, padding zeroing, and CRC32 scope.

See `rsx-dxs/tests/cmp_test.rs` — covers CmpSender (monotonic seq,
heartbeat timing, flow control window, NAK handling, retransmit format,
zero-heap send loop) and CmpReceiver (sequential delivery, gap detection,
NAK emission, StatusMessage timing, reorder buffer, duplicate/unknown
record handling, CRC validation, zero-heap recv loop).

See `rsx-dxs/tests/client_test.rs` and `rsx-dxs/tests/tls_test.rs` —
covers TCP replication: ReplayRequest encode/decode, server streaming,
CaughtUp marker, live tail transition, reconnect backoff, resume from
tip+1, TLS handshake, invalid CRC disconnect.

## 3. E2E Tests

Note: the e2e test files listed below (`cmp_e2e_test.rs`,
`cmp_fault_test.rs`) are aspirational — not yet present in
`rsx-dxs/tests/`. Scenarios below describe intended coverage.

See `rsx-dxs/tests/cmp_test.rs` for existing happy-path integration.
Remaining e2e scenarios (fault injection, flow control under real load,
long-running stability) are planned for a future iteration.

### Planned E2E Scenarios

- Happy path: 100/1000-record burst, steady-state heartbeat/status exchange
- Gap detection + recovery: single drop, consecutive drops, multiple gaps,
  dropped NAK, dropped heartbeat, dropped StatusMessage
- Flow control: slow receiver stalls sender, zero-window stall, window
  update unblocks
- Reorder: 2-packet and 10-packet out-of-order delivery, reorder buffer
  overflow at 512+1
- TCP cold path: replay from beginning/mid, CaughtUp then live tail,
  disconnect/reconnect, TLS full cycle, multiple independent consumers
- Fault injection: corrupted CRC, garbage header, duplicate delivery,
  delayed duplicate after retransmit, 5% random loss sustained throughput
- Long-running: 1M messages, no memory growth, seq monotonic, tip
  persistence survives crash/restart

## 4. Benchmarks

File: `rsx-dxs/benches/cmp_bench.rs`

All benchmarks use Criterion. monoio runtime for async
benchmarks. Measure userspace time only.

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
| Gateway -> Risk (CmpSender) | NETWORK.md | `cmp_test.rs` |
| Risk -> ME (CmpSender) | NETWORK.md | `cmp_test.rs` |
| ME -> Risk (CmpSender) | NETWORK.md | `cmp_test.rs` |
| Risk -> Gateway (CmpSender) | NETWORK.md | `cmp_test.rs` |
| CmpSender reads WAL for retransmit | DXS.md §3,4 | `cmp_test.rs` |
| DxsConsumer uses TCP replication | DXS.md §5,6 | `client_test.rs` |
| Recorder uses TCP replication | DXS.md §8 | `client_test.rs` |
| Marketdata recovery via TCP | MARKETDATA.md §8 | `client_test.rs` |
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
    cmp_test.rs             CmpSender + CmpReceiver unit tests
    client_test.rs          TCP replication client tests
    header_test.rs          WAL header encoding tests
    records_test.rs         WAL record type tests
    tls_test.rs             TLS handshake tests
    wal_test.rs             WalWriter + WalReader tests
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
