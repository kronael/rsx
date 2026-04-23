---
status: shipped
---

# DXS — Direct Exchange Streaming

Brokerless WAL streaming. Each producer IS the server for its own
stream. Consumers connect directly to producers. No central broker.

The WAL disk format = fixed-record wire format = what gets streamed.
No transformation between storage and network.

Crate: `rsx-dxs`. Embedded by all producers and consumers.

## Table of Contents

- [1. WAL Record Format](#1-wal-record-format)
- [2. File Layout](#2-file-layout)
- [3. WalWriter](#3-walwriter)
- [4. WalReader](#4-walreader)
- [5. Replay Server](#5-replay-server)
- [6. Consumer](#6-consumer)
- [7. Transport](#7-transport)
- [8. Recorder Pattern](#8-recorder-pattern)
- [9. How DXS Replaces Existing Specs](#9-how-dxs-replaces-existing-specs)
- [10. WAL Replay Edge Cases](#10-wal-replay-edge-cases)
- [11. Performance Targets](#11-performance-targets)
- [12. File Organization](#12-file-organization)

---

## 1. WAL Record Format

Each record on disk is a **fixed-size** struct with a 16-byte header:

```
struct WalHeader {
  u16 record_type;   // enum
  u16 len;           // payload bytes (<= 64KB)
  u32 crc32;         // checksum of payload bytes
  u8  _reserved[8];  // reserved for future use
}
```

The payload immediately follows the header and is a fixed-record
struct for that `record_type` (`#[repr(C, align(64))]`, little-endian).

### CmpRecord trait

All **data** payloads implement the CmpRecord trait and have
`seq: u64` as the first 8 bytes:

```rust
pub trait CmpRecord: Copy {
    fn seq(&self) -> u64;
    fn set_seq(&mut self, seq: u64);
}
```

Sequence numbers are assigned by WalWriter::append<T: CmpRecord>
or CmpSender::send<T: CmpRecord>. Control messages
(StatusMessage, Nak, CmpHeartbeat) do NOT implement CmpRecord.

Readers validate `crc32` and truncate the WAL on the first
invalid record.

**Version policy:**

- **Additive changes** (new record types): readers ignore
  unknown record types (log + continue).
- **Breaking changes** (field reordering, type changes,
  removed fields): require coordinated deployment (stop all
  producers, upgrade all readers, restart).
- **Upgrade order:** deploy consumers first (they ignore
  unknown record types), then deploy producers emitting
  new records.

**Record types (v1):**
- `RECORD_FILL`
- `RECORD_BBO`
- `RECORD_ORDER_INSERTED`
- `RECORD_ORDER_CANCELLED`
- `RECORD_ORDER_DONE`
- `RECORD_CONFIG_APPLIED`
- `RECORD_CAUGHT_UP` (replay marker)
- `RECORD_ORDER_ACCEPTED` (dedup record)
- `RECORD_STATUS_MESSAGE` (CMP control: flow control)
- `RECORD_NAK` (CMP control: gap detection)
- `RECORD_HEARTBEAT` (CMP control: liveness)
- `RECORD_MARK_PRICE`
- `RECORD_ORDER_REQUEST`
- `RECORD_ORDER_RESPONSE`
- `RECORD_CANCEL_REQUEST`
- `RECORD_ORDER_FAILED`
- `RECORD_LIQUIDATION`
- `RECORD_REPLAY_REQUEST`

Each payload is a fixed struct with explicit little-endian fields and
no padding beyond `#[repr(C, align(64))]`.

**CancelReason (u8):**
- 0 = user_cancel
- 1 = reduce_only
- 2 = expiry
- 3 = system
- 4 = post_only_reject
- 5 = other

**Mapping (simple):**
- user_cancel: explicit client cancel request
- reduce_only: reduce-only clamp/reject
- expiry: time-in-force expiry (IOC/FOK)
- post_only_reject: post-only would take
- system: internal kill switch or maintenance

**Dedup record:**

Order deduplication survives ME restart via WAL. On each
accepted order, ME appends `RECORD_ORDER_ACCEPTED {
user_id, order_id }` to WAL before processing. On replay,
ME rebuilds the dedup set from these records. Dedup key is
`(user_id, order_id)`.

The dedup window is bounded by the same 5min pruning as
in-memory (MESSAGES.md section 7). During replay, records older
than 5min from WAL tip are skipped.

See `rsx-dxs/src/records.rs`

All fields are encoded little-endian on disk/wire.

On disk: `[header][payload][header][payload]...`

Over CMP/UDP (hot path) and TCP (cold path): the same fixed
records are streamed as raw bytes. See [CMP.md](CMP.md) for
the transport specification.

Maximum record size is 64KB.

---

## 2. File Layout

```
wal/{stream_id}/{stream_id}_{first_seq}_{last_seq}.wal
```

- Rotate by size: 64MB default.
- Retention: 10min for hot replay (in-memory). Offload to ARCHIVE for infinite retention.
- No file header, no index. Sequential read only.
- Filenames encode seq range for O(1) file selection.

**Rotation:** when current file exceeds 64MB, close it (rename
with `last_seq` in filename), open a new file. The active file
uses a temporary name `{stream_id}_active.wal` until rotation.

**GC:** delete hot WAL files where `last_seq` timestamp is older than
retention window *after offload*. GC runs on rotation (no timer needed).

**Archive fallback:** if `from_seq_no` is older than hot retention, consumers
request replay from ARCHIVE (see ARCHIVE.md), then resume from DXS hot tail.

---

## 3. WalWriter

Append-only writer embedded in each producer. See `rsx-dxs/src/wal.rs`

**Append:** `append<T: CmpRecord>(record: &mut T)` assigns
monotonic `seq`, serializes fixed record to buf.
Producer-local sequence, no coordination. O(1) memcpy.

**Flush:** every 10ms, write buf to file + fsync. Resets buf.
Flush is called by the producer's main loop (not a background
thread) to avoid synchronization.

**Rotation:** on flush, if `file_size > max_file_size`, close
current file with final seq range in name, open new file, run GC.

**GC:** scan directory, parse filenames, delete files outside
retention window.

**Backpressure:** if buf exceeds 2x `max_file_size`, the append
call blocks (producer stalls). This prevents unbounded memory
growth if flush falls behind.

---

## 4. WalReader

Sequential reader for WAL files. See `rsx-dxs/src/wal.rs`

**Open from seq:** list files, parse filenames, binary search
for the file containing `target_seq`. Seek within file by reading
fixed records until `seq >= target_seq`.

**Iteration:** read header + payload, decode fixed record.
Returns `Option<WalRecord>` — `None` at EOF.

**File transition:** when current file is exhausted, open next
file in sorted order. Returns `None` when all files exhausted
(reader is caught up).

---

## 5. Replay Server (DxsReplayService)

DxsReplayService embedded in each producer. Serves WAL records
to consumers over TCP. See [CMP.md](CMP.md).

**Request format (WAL record):**
```
#[repr(C, align(64))]
struct ReplayRequest {
  u32 stream_id;     // routing for TCP connection
  u32 _pad0;
  u64 from_seq;
  u8  _pad1[48];
}
```

ReplayRequest does NOT implement CmpRecord (no seq field).
It uses `stream_id` for TCP connection routing.

**Response format:**
Streams raw WAL bytes (header + payload, fixed-record format)
over a TCP stream.

**Protocol:**

1. Consumer opens TCP connection to producer (optional TLS).
2. Consumer sends `ReplayRequest` as a WAL record.
3. Server opens WalReader at `from_seq`.
4. Server streams WalRecords over the TCP stream.
5. When reader exhausts all files (caught up to live): server
   sends a WalRecord with a `CaughtUp` marker payload, then
   transitions to broadcasting new records as they are appended.
6. Live broadcast: server registers as a listener on WalWriter.
   Each flush notifies listeners. Server reads new records and
   streams them.

**CaughtUp marker:**

Use a fixed record type `RECORD_CAUGHT_UP` with payload:
`{live_seq: u64}`.

**CaughtUp semantics:**

- `live_seq` = last seq the consumer has seen (inclusive).
  The consumer's WAL reader has delivered all records up to
  and including `live_seq`.
- After CaughtUp, the consumer resumes processing at
  `live_seq + 1` (the next record appended by the producer).
- CaughtUp is **per-symbol stream** (one per `stream_id`),
  not a global sync point. A consumer with multiple streams
  receives independent CaughtUp markers for each.
- Risk engine "goes live" after receiving CaughtUp for
  **all** subscribed streams (per RISK.md replication
  section).

**Concurrency:** one TCP connection per connected consumer.
Each connection has its own WalReader. Live broadcast uses a
notify mechanism (e.g., eventfd or channel).

**Transport:** WAL replication over TCP (optional TLS via
rustls). Same WAL record format on wire as on disk. Zero
serialization overhead. See [CMP.md](CMP.md) for full
transport specification.

---

## 6. Consumer

Embedded in each consumer process. Manages connection to a
producer's DxsReplay service, tracks processing tips.
See `rsx-dxs/src/client.rs`

**Startup sequence:**

1. Load tip from `tip_file` (0 if missing).
2. Connect to producer's TCP endpoint (optional TLS).
3. Send `ReplayRequest { stream_id, from_seq: tip + 1 }`.
4. Process replayed records via callback, advancing tip.
5. On `CaughtUp`: transition to live processing.
6. Continue processing live records via callback.

**Tip persistence:** flush tip to `tip_file` every 10ms (batched
with other I/O). On crash, replay from last persisted tip. Records
are idempotent or deduped by `seq` at the consumer.

**Reconnect:** on disconnect, reconnect with backoff (1s/2s/4s/8s,
max 30s). Resume from `tip + 1`.

---

## 7. Transport

Two transport modes, same WAL records:

- **Live path** (Gateway <-> Risk <-> ME): CMP/UDP. One
  WAL record per UDP datagram. Aeron-style NACK + flow
  control. See [CMP.md](CMP.md) section 3.
- **Replay/replication path**: WAL replication over TCP.
  Plain byte stream, optional TLS. See [CMP.md](CMP.md)
  section 4.

---

## 8. Recorder Pattern

Generic archival consumer. Same binary as any DXS consumer, with
different config. Subscribes to a producer's stream and writes to
daily archive files.

**Archive layout:**

```
archive/{stream_id}/{stream_id}_{YYYY-MM-DD}.wal
```

Same fixed-record format on disk. No transformation.

**Three recorder instances** (separate processes or config
sections):

| Instance | Stream | Source |
|----------|--------|--------|
| Market data | ME events (fills, BBO, orders) | Matching engine |
| Risk events | Risk state changes | Risk engine |
| Mark prices | MarkPriceEvent stream | Mark price aggregator |
| MARKETDATA | ME events (recovery/replay) | Matching engine |

MARKETDATA also connects as a DXS consumer for recovery (replay
from ME WAL on startup). See [MARKETDATA.md](MARKETDATA.md)
section 8.

**Daily rotation:** at UTC midnight, close current file, open new
file with next date. No retention limit on archive files (managed
externally).

**Config:**

```
RSX_RECORDER_STREAM_ID=1
RSX_RECORDER_PRODUCER_ADDR=10.0.0.1:9100
RSX_RECORDER_ARCHIVE_DIR=./archive
RSX_RECORDER_TIP_FILE=./archive/1.tip
```

**File organization:**

```
rsx-recorder/src/
    main.rs       -- entrypoint, config, daily rotation
```

Recorder reuses `DxsConsumer` from `rsx-dxs` for subscription.
The callback writes records to the daily archive file.

---

## 9. How DXS Replaces Existing Specs

| Current | DXS replacement |
|---------|----------------|
| ORDERBOOK.md WAL (section 2.8) | ME embeds WalWriter |
| ORDERBOOK.md recovery | ME embeds DxsReplay server |
| RISK.md replay from ME | Risk is DXS consumer of ME stream |
| RISK.md tip persistence | Consumer.tip persistence |
| WAL.md local buffer | WalWriter with 10ms flush |
| WAL.md replica sync | DxsReplay live tail mode |

---

## 10. WAL Replay Edge Cases

This section documents critical edge cases for WAL replay that
consumers must handle correctly.

### 10.1 Crash Mid-Rotation

**Scenario:** Writer crashes during file rotation (between rename
and opening new active file).

**State:** Active file exists but may be partially written. New
file not yet created.

**Handling:** Reader treats `{stream_id}_active.wal` as the last
file. CRC validation truncates at first invalid record. No data
loss — rotation is atomic (rename) or recoverable (partial write
detected by CRC).

**Test:** `write_flush_crash_recover_from_last_fsync` (TESTING-DXS.md)

### 10.2 Partial Record at EOF

**Scenario:** Writer crashes mid-write after writing header but
before completing payload write or fsync.

**State:** WAL file ends with valid header but truncated/missing
payload.

**Handling:** Reader detects `UnexpectedEof` when reading payload
after header. Logs warning and returns `None` (end of valid data).
Subsequent replay from tip+1 will skip the partial record.

**Invariant:** All complete records have valid CRC. Partial
records are never processed.

### 10.3 CRC Mismatch Mid-File

**Scenario:** Disk corruption or incomplete write causes CRC
mismatch on a record in the middle of a file.

**State:** File contains N valid records, then one corrupted
record, then potentially more valid records.

**Handling:** Reader computes CRC, compares to header. On
mismatch, logs warning and returns `None` (truncates at first
bad record). All subsequent records in file and later files are
ignored.

**Trade-off:** Conservative truncation vs attempting to skip bad
record. We truncate because: (1) simpler, (2) avoids processing
potentially inconsistent data, (3) WAL should never have mid-file
corruption in normal operation (fsync guarantees).

### 10.4 Unknown Record Type

**Scenario:** Reader encounters a record type it doesn't recognize
(future version, corruption, or version mismatch).

**State:** Header is valid (CRC matches payload), but `record_type`
field is not in reader's known set.

**Handling (v1):** Reader returns record as `RawWalRecord` without
failing. Consumer is responsible for handling unknown types:
- **Matching engine:** skip unknown types, log warning
- **Risk engine:** skip unknown types, log warning
- **Market data:** skip unknown types, log warning

**Version policy:** Additive changes (new record types) are safe.
Consumers ignore unknown types. Breaking changes (field reorder,
type change) require coordinated deployment (stop producers,
upgrade consumers, restart).

**Implementation:** Reader does not validate record type (returns
all records). Consumers filter in their callback.

**Future:** If strict version enforcement needed, add version
field to WalHeader and fail fast on version mismatch.

### 10.5 Gap in Sequence Numbers

**Scenario:** Consumer replays from seq N but first record in WAL
is seq M > N. Gap: (N, M).

**Cause:** WAL retention window expired (GC deleted old files), or
consumer offline longer than retention period.

**Handling:** Consumer should check first replayed seq against
requested `from_seq`. If gap detected, consumer must:
1. Request replay from ARCHIVE (cold storage) for missing range
2. Resume from hot WAL after archive replay completes
3. Fail if archive unavailable and gap is unacceptable

**Implementation:** Archive fallback not yet implemented (DXS.md
requirement D25). Current behavior: consumer replays from first
available seq, effectively skipping gap. Risk engine would have
inconsistent state.

**Mitigation:** Set retention window > max expected consumer
offline duration (default 10min). Monitor consumer lag.

### 10.6 Replay from Future Sequence

**Scenario:** Consumer requests `from_seq = 1000` but WAL
`last_seq = 500`.

**Cause:** Consumer tip file corrupted, manual override, or clock
skew.

**Handling:** Reader opens WAL at `from_seq`, finds no files
containing that seq, returns `None` immediately (caught up at
current tip). Consumer processes no records, sends CaughtUp with
`live_seq = 500`, then waits for new records.

**Invariant:** Replay from future is safe (no-op) but indicates
configuration error. Should log warning.

### 10.7 Active File Exists But Empty

**Scenario:** Writer created active file but crashed before first
append+flush.

**State:** `{stream_id}_active.wal` exists with size 0.

**Handling:** Reader opens file, attempts to read header, gets
`UnexpectedEof`, advances to next file (none), returns `None`.
No records replayed. Next writer append will reuse the empty
active file.

### 10.8 Interleaved Rotation During Replay

**Scenario:** Consumer is replaying historical WAL while writer
rotates files (deletes old, creates new).

**State:** Reader has file list snapshot. Writer GC may delete a
file the reader hasn't processed yet.

**Handling:** Reader opens files by path. If file deleted before
reader opens it, returns `Err` (file not found). Consumer should
reconnect and retry from last persisted tip.

**Mitigation:** Retention window provides buffer. If consumer lag
< retention window, files won't be GC'd during active replay.
Monitor consumer lag vs retention window.

**Future:** Advisory lock on WAL files during active replay, or
reference-counted file handles.

### 10.9 Multiple Active Files

**Scenario:** Writer crashed, operator manually renamed active
file, writer restarted and created new active file. Now two
active files exist.

**State:** `{stream_id}_active.wal` (new) and
`{stream_id}_active.wal.old` (orphaned).

**Handling:** Reader lists files with `_active.wal` suffix. Finds
one match (naming is deterministic). Orphaned file is ignored
unless manually renamed to rotated format.

**Operator action:** Rename orphaned active file to proper seq
range format: `{stream_id}_{first}_{last}.wal` (parse first/last
from file contents), or delete if known to be incomplete.

### 10.10 Concurrent Readers on Same WAL

**Scenario:** Multiple consumers (e.g., Risk replica + Recorder +
Market data) replay from same WAL directory concurrently.

**State:** Readers open separate file handles. No locking.

**Handling:** Filesystem provides concurrent read safety. Each
reader maintains independent position. Writers use append-only
mode with atomic fsync. No coordination needed.

**Invariant:** WAL files are immutable after rotation. Active file
is append-only. No reader-writer or reader-reader conflicts.

### 10.11 Tip Persistence Lag

**Scenario:** Consumer processes records faster than tip
persistence interval (10ms). Crash before tip flushed.

**State:** Consumer processed records up to seq 150, but persisted
tip = 100.

**Handling:** On restart, consumer replays from tip+1 = 101.
Records 101-150 are reprocessed. Consumer callback must be
idempotent or deduplicate by seq.

**Mitigation:** 10ms tip persistence interval bounds duplicate
replay window to ~10ms of events (hundreds to thousands of
records at target throughput).

**Implementation:** Consumer dedup by seq. Risk engine position
updates are idempotent (fill with seq N applied exactly once even
if replayed). Gateway ORDER_DONE ack is idempotent (client_order_id
dedup).

### 10.12 CaughtUp Marker Timing

**Scenario:** Consumer receives CaughtUp with `live_seq = 500`,
but writer appends records 501-510 during the CaughtUp send.

**State:** Consumer transitions to live mode at seq 501, but
records 501-510 already exist in WAL.

**Handling:** After CaughtUp, server transitions to live tail
mode. It opens new reader at `last_seq + 1` (501 in this case).
If records already exist in WAL (501-510), they are immediately
streamed. Consumer processes them, then waits for notify on new
records. No gap, no duplicate.

**Invariant:** CaughtUp.live_seq is inclusive (last seq delivered).
Next replay starts at live_seq+1. Live tail notify is edge-
triggered (wakes on new flush), so buffered records are delivered
immediately.

### 10.13 Network Partition During Live Tail

**Scenario:** DXS consumer in live tail mode. Network partition
or server restart causes TCP disconnect.

**State:** Consumer loses connection mid-stream. Records may have
been appended to WAL but not delivered.

**Handling:** Consumer detects disconnect (TCP error on read/write),
persists current tip, reconnects with backoff (1/2/4/8/30s).
Sends new ReplayRequest from tip+1. Server replays any missed
records, sends new CaughtUp, resumes live tail.

**Duplicate handling:** If consumer received records but didn't
persist tip before disconnect, records will be replayed. Consumer
dedup by seq ensures idempotency.

### 10.14 Writer Flush Lag Exceeds Bound

**Scenario:** Disk slow, fsync takes >10ms. Writer buffer fills
faster than flush rate.

**State:** Buffer exceeds backpressure threshold (2x max_file_size).

**Handling:** Writer append returns `Err(WouldBlock)`. Matching
engine stalls (stops processing new orders). This is intentional
backpressure to preserve 10ms durability bound. System latency
increases, but correctness maintained.

**Mitigation:** Use fast SSD with consistent <1ms fsync. Monitor
flush latency. Alert on >5ms p99.

### 10.15 Replay from Seq 0

**Scenario:** Consumer requests replay from seq 0 (fresh start,
no prior tip).

**State:** WAL may have files starting at seq > 0 (early files
GC'd), or WAL may be empty (no records yet).

**Handling:** Reader opens from seq 0. If files exist, starts at
first available seq (may be > 0). If no files exist, returns
`None` immediately (caught up). Consumer receives CaughtUp with
`live_seq = 0` (or first available seq), then waits for new
records.

### 10.16 Rotation Boundary Replay

**Scenario:** Consumer replays across a rotation boundary. Last
record in file N is seq 499, first record in file N+1 is seq 500.

**State:** Two files: `{stream_id}_1_499.wal` and
`{stream_id}_500_998.wal`.

**Handling:** Reader exhausts file N (last record seq 499),
advances to file N+1, seamlessly reads first record (seq 500).
No gap, no duplicate.

**Invariant:** `last_seq` in filename is inclusive. `first_seq`
in next file is `last_seq + 1` of previous file (if consecutive).
Gap in filename seq ranges is allowed (GC), but reader handles
via file-level transition.

---

## 11. Performance Targets

See `wal_bench.rs` and `encode_bench.rs` for measured targets.

---

## 12. File Organization

```
crates/rsx-dxs/src/
    lib.rs        -- public API: WalWriter, WalReader, DxsConsumer
    wal.rs        -- WalWriter, WalReader, file layout, GC
    server.rs     -- DxsReplayService TCP server
    client.rs     -- DxsConsumer, tip tracking, reconnect
    config.rs     -- env config parsing

crates/rsx-recorder/src/
    main.rs       -- recorder binary entrypoint, daily rotation
```
