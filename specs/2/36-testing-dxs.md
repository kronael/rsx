---
status: shipped
---

# TESTING-DXS.md — DXS (WAL + Streaming) Tests

Source specs: [DXS.md](DXS.md), [WAL.md](WAL.md)

Crate: `rsx-dxs`

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
| D1 | Fixed-record format: 16B header + repr(C) payload | DXS.md §1 |
| D1a | Payload begins with CMP prefix | CMP.md |
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
| D12 | DxsReplay server: TCP stream from from_seq | DXS.md §5 |
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

See `rsx-dxs/tests/wal_test.rs` — covers WAL record encoding
(header encode/decode, little-endian layout, CRC32 scope, all payload
types roundtrip, edge cases: max payload, CRC mismatch, truncated
header/payload, zero-length payload, unknown version fail-fast,
CancelReason all 6 values), WalWriter (monotonic seq, buffer/flush/fsync,
rotation at 64MB, GC, backpressure stalls, size-threshold flush), and
WalReader (open from seq, sequential iteration, file transitions, EOF,
CRC invalid truncation, unknown version stop).

See `rsx-dxs/tests/records_test.rs` — covers DxsConsumer (tip load/zero,
replay request from tip+1, tip advancement and persistence, reconnect
backoff, callback invocation, dedup by seq).

---

## E2E Tests

See `rsx-dxs/tests/wal_test.rs` — covers writer+reader roundtrip (100
records, rotation across files, crash/recover from last fsync, GC then
read retained range), replay server+consumer (from beginning/mid, CaughtUp
marker, live tail, multiple consumers, disconnect/reconnect resume), tip
persistence (crash resume, monotonic advance, idempotent replay), recorder
(daily archive files, UTC midnight rotation, format match), backpressure
(buffer full stall/resume, flush lag stall), archive fallback, and replica
lag stall.

---

## Edge Case Tests

See `rsx-dxs/tests/wal_test.rs` — covers crash mid-rotation, partial
records at EOF, CRC mismatch mid-file, unknown record types, seq gaps from
GC, replay from future seq, empty active file, interleaved rotation during
replay, orphaned active files, concurrent readers, tip persistence lag,
CaughtUp timing during concurrent appends, TCP disconnect/reconnect, writer
flush lag, replay from seq 0, and rotation boundary continuity.

### Invariant Verification Tests

The following invariants are verified in `rsx-dxs/tests/wal_test.rs`
(existing coverage). Aspirational scenarios (marked *) are planned but
not yet implemented:

- Seq monotonic across rotations and crashes; never decreases per stream
- All complete records have valid CRC; partial records never processed;
  CRC mismatch stops replay
- Rotated files never modified; active file append-only; no reader/writer
  conflicts
- Duplicate replay no side effects; position updates idempotent; ack
  messages idempotent
- Rotation atomic via rename; tip persistence atomic via rename; fsync
  ensures durability

---

## Benchmarks

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
- Cross-host live streaming via WAL/TCP + WAL wire format
  (DXS.md §7)
- CMP/UDP for hot-path between processes (ME -> risk, risk ->
  gateway), DXS for cross-host and replay (DXS.md §7,
  NETWORK.md §internal)
