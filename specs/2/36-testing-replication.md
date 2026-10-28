---
status: shipped
---

# TESTING-replication.md — replication (WAL + Streaming) Tests

Source specs: [10-replication.md](10-replication.md), [48-wal.md](48-wal.md)

Crate: `rsx-cast`

## Table of Contents

- [Requirements Checklist](#requirements-checklist)
- [Unit Tests](#unit-tests)
- [E2E Tests](#e2e-tests)
- [Edge Case Tests](#edge-case-tests)
- [Benchmarks](#benchmarks)
- [Integration Points](#integration-points)

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| D1 | Fixed-record format: 16B header + repr(C) payload | replication.md §1 |
| D1a | Payload begins with casting prefix | casting.md |
| D2 | CRC32 validation on read, truncate on invalid | replication.md §1 |
| D3 | Little-endian encoding for all fields | replication.md §1 |
| D4 | File layout: `wal/{stream_id}/{stream_id}_{first}_{last}.wal` | replication.md §2 |
| D5 | Rotate at 64MB, retain 10min | replication.md §2 |
| D6 | WalWriter: monotonic seq, O(1) append | replication.md §3 |
| D7 | WalWriter: flush every 10ms + fsync | replication.md §3 |
| D8 | WalWriter: backpressure stall at 2x max_file_size | replication.md §3 |
| D9 | WalWriter: GC deletes files outside retention | replication.md §3 |
| D10 | WalReader: open from seq via filename binary search | replication.md §4 |
| D11 | WalReader: sequential iteration across files | replication.md §4 |
| D12 | ReplicationService: TCP stream from from_seq | replication.md §5 |
| D13 | ReplicationService: CaughtUp marker then live tail | replication.md §5 |
| D14 | ReplicationConsumer: tip persistence every 10ms | replication.md §6 |
| D15 | ReplicationConsumer: reconnect with backoff 1/2/4/8/30s | replication.md §6 |
| D16 | ReplicationConsumer: resume from tip+1 | replication.md §6 |
| D17 | Recorder: daily rotation, same fixed-record format | replication.md §8 |
| D18 | Bounded loss window: 10ms (WAL.md) | WAL.md |
| D19 | Replica lag bound: 100ms, stall if exceeded | WAL.md |
| D20 | Unknown version: stop replay and fail fast (no skip) | replication.md §1 |
| D21 | CancelReason enum: 6 values (0-5) | replication.md §1 |
| D22 | No file header, no index, sequential read only | replication.md §2 |
| D23 | Active file uses temp name `{stream_id}_active.wal` | replication.md §2 |
| D24 | GC runs on rotation, not on timer | replication.md §2,3 |
| D25 | Archive fallback when from_seq older than hot retention | replication.md §2 |
| D26 | Local WAL full: stall producer | WAL.md |
| D27 | Flush lag >10ms: stall producer | WAL.md |
| D28 | Flush on size threshold (1000 records) in addition to 10ms | WAL.md |

---

## Unit Tests

`rsx-cast/src/header_test.rs` covers the 16 B header: encode and decode
roundtrip, little-endian layout, size, version 1 on new headers, an
unknown version returning `None`, short and random input without a
panic, and the CRC32 over the payload.

`rsx-cast/src/wal_test.rs` covers the writer: monotonic seq starting at 1,
append without I/O, flush and fsync, a failed fsync not duplicating
records, the temp name of the active file, rotation at the size threshold
and at 64 MB with the seq range in the new name, retention pruning of old
segments, and the backpressure stall. It covers the reader: open from a
seq, sequential iteration, file transitions, EOF, an empty directory, a
single file and sorted multiple files, `read_record_at_seq`, an unknown
record type, a CRC failure mid-file or at the end of a rotated segment,
a partial record in a rotated segment, a torn tail of the active segment,
and the 64 KB maximum payload. `WalWriter::open` cuts a torn tail and
keeps a file with mid-file corruption.

`rsx-cast/src/replication_client_test.rs` covers the tip file: write and
read roundtrip, a missing file reading as zero. The other cases in that
file model the consumer rules (tip plus one, dedup by seq, backoff
1/2/4/8/30 s, unknown record types) on plain variables. They do not
exercise `ReplicationConsumer`.

`rsx-cast/src/replication_server_test.rs` covers TLS: a client and server
connect, the service requires a server certificate and the consumer
requires a client CA.

---

## E2E Tests

`rsx-cast/tests/seq_gap_replay_test.rs` drives a receiver into FAULTED
on a seq gap and resumes from a TCP replay.

`rsx-cast/tests/nak_wal_fallback_test.rs` delivers an evicted seq from
the WAL after a NAK.

`rsx-cast/tests/replication_federation_test.rs` falls through from an
endpoint that answers NOT_AVAILABLE to the next endpoint.

`rsx-cast/tests/wal_flush_fault_test.rs` retries after a partial write
and leaves each record once.

`rsx-cast/src/replication_server_test.rs` catches up over a corrupt WAL
and never sends `CAUGHT_UP`.

The recorder's daily archive file naming is covered in
`rsx-recorder/tests/recorder_test.rs`.

---

## Edge Case Tests

Covered: CRC mismatch mid-file, torn tail of the active segment, partial
record in a rotated segment, unknown record type, a corrupt WAL under
catch-up, a partial write on flush, a failed fsync, replay from seq 0,
and the seq gap that forces FAULTED.

Planned, no test yet: crash mid-rotation, replay from a seq in the future,
rotation during replay, orphaned active files, concurrent readers, tip
file corruption, `CAUGHT_UP` timing during concurrent appends, TCP
disconnect and reconnect during the live tail, writer flush-lag stall,
replica lag stall, archive fallback in the replication server, and tests
that run `ReplicationConsumer` itself for dedup, backoff and tip
advancement.

### Invariants with a test

- Seq is monotonic from 1 within a writer (`writer_assigns_monotonic_seq`,
  `writer_seq_starts_at_1`).
- A CRC mismatch stops the reader
  (`reader_errors_on_crc_failure_mid_file`).
- A rotated file is named by its seq range and the active file keeps the
  temp name (`writer_rotation_renames_with_seq_range`,
  `writer_active_file_uses_temp_name`).
- A catch-up over corrupt data never reports `CAUGHT_UP`.

---

## Benchmarks

Targets from replication.md §10:

| Operation | Target |
|-----------|--------|
| `WalWriter::prepare` + `append_framed` (Vec extend, no disk I/O) | <200ns |
| WAL flush (fsync) | <1ms per 64KB batch |
| WAL read (sequential) | >500 MB/s |
| Replay 100K records | <1s |
| Recorder sustained write | >100K records/s |
| Tip persist | every 10ms, batched |

---

## Integration Points

- Matching engine embeds WalWriter for event persistence
  (ORDERBOOK.md §2.8)
- Matching engine embeds ReplicationService for downstream
  consumers (replication.md §5)
- Risk engine connects as ReplicationConsumer for replay on startup
  (RISK.md §replication)
- Mark aggregator embeds WalWriter + ReplicationService for mark
  prices (MARK.md §1)
- Recorder connects as ReplicationConsumer for archival (replication.md §8)
- Market data connects as ReplicationConsumer for recovery
  (MARKETDATA.md §8)
- WAL backpressure rules propagate to matching engine stall
  (WAL.md, CONSISTENCY.md §3)
- Archive fallback when hot WAL retention exceeded
  (replication.md §2)
- Cross-host live streaming via WAL/TCP + WAL wire format
  (replication.md §7)
- casting/UDP for hot-path between processes (ME -> risk, risk ->
  gateway), replication for cross-host and replay (replication.md §7,
  NETWORK.md §internal)
