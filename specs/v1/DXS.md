# DXS — Direct Exchange Streaming

Brokerless WAL streaming. Each producer IS the server for its own
stream. Consumers connect directly to producers. No central broker.

The WAL disk format = protobuf wire format = what gets streamed.
No transformation between storage and network.

Crate: `rsx-dxs`. Embedded by all producers and consumers.

---

## 1. WAL Record Format

Each record on disk is a length-delimited protobuf message. The
protobuf definitions serve as both the wire format AND the storage
format.

```protobuf
message WalRecord {
  uint64 seq_no = 1;
  uint64 timestamp_ns = 2;
  uint32 stream_id = 3;
  oneof payload {
    FillEvent fill = 10;
    BBOEvent bbo = 11;
    OrderInserted order_inserted = 12;
    OrderCancelled order_cancelled = 13;
    OrderDone order_done = 14;
    MarkPriceEvent mark_price = 20;
    // risk events: 30+
  }
}
```

On disk: `[4-byte len][protobuf bytes][4-byte len][protobuf bytes]...`

Over gRPC: same protobuf messages streamed directly.

The 4-byte length prefix is little-endian u32. Maximum record size
is 64KB (fits any single event with margin). Length does not include
itself.

---

## 2. File Layout

```
wal/{stream_id}/{stream_id}_{first_seq}_{last_seq}.wal
```

- Rotate by size: 64MB default.
- Retention: 10min for hot replay.
- No file header, no index. Sequential read only.
- Filenames encode seq range for O(1) file selection.

**Rotation:** when current file exceeds 64MB, close it (rename
with `last_seq` in filename), open a new file. The active file
uses a temporary name `{stream_id}_active.wal` until rotation.

**GC:** delete files where `last_seq` timestamp is older than
retention window. GC runs on rotation (no timer needed).

---

## 3. WalWriter

Append-only writer embedded in each producer.

```rust
struct WalWriter {
    stream_id: u32,
    next_seq: u64,
    buf: Vec<u8>,         // write buffer, flushed periodically
    file: File,           // current WAL file
    file_size: u64,       // bytes written to current file
    first_seq: u64,       // first seq_no in current file
    wal_dir: PathBuf,
    max_file_size: u64,   // 64MB default
    retention_ns: u64,    // 10min default
}
```

**Append:** serialize WalRecord to buf with length prefix. Assign
monotonic `seq_no` (producer-local, no coordination). O(1) memcpy.

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

Sequential reader for WAL files.

```rust
struct WalReader {
    stream_id: u32,
    wal_dir: PathBuf,
    current_file: Option<File>,
    current_offset: u64,
    files: Vec<WalFileInfo>,  // sorted by first_seq
}

struct WalFileInfo {
    path: PathBuf,
    first_seq: u64,
    last_seq: u64,
}
```

**Open from seq_no:** list files, parse filenames, binary search
for the file containing `target_seq`. Seek within file by reading
length-prefixed records until `seq_no >= target_seq`.

**Iteration:** read 4-byte len, read len bytes, decode protobuf.
Returns `Option<WalRecord>` — `None` at EOF.

**File transition:** when current file is exhausted, open next
file in sorted order. Returns `None` when all files exhausted
(reader is caught up).

---

## 5. Replay Server

gRPC service embedded in each producer. Serves WAL records to
consumers.

```protobuf
service DxsReplay {
  rpc Stream(ReplayRequest) returns (stream WalRecord);
}

message ReplayRequest {
  uint32 stream_id = 1;
  uint64 from_seq_no = 2;
}
```

**Protocol:**

1. Consumer sends `ReplayRequest` with `from_seq_no`.
2. Server opens WalReader at `from_seq_no`.
3. Server streams WalRecords from WAL files.
4. When reader exhausts all files (caught up to live): server
   sends a WalRecord with a `CaughtUp` marker payload, then
   transitions to broadcasting new records as they are appended.
5. Live broadcast: server registers as a listener on WalWriter.
   Each flush notifies listeners. Server reads new records and
   streams them.

**CaughtUp marker:**

```protobuf
message CaughtUp {
  uint64 live_seq_no = 1;  // seq_no of last replayed record
}
```

Added to the `WalRecord.payload` oneof (field 50).

**Concurrency:** one gRPC handler per connected consumer. Each
handler has its own WalReader. Live broadcast uses a notify
mechanism (e.g., `tokio::sync::Notify` or eventfd).

**Transport:** gRPC over TCP. Latency is not critical for replay.

---

## 6. Consumer

Embedded in each consumer process. Manages connection to a
producer's DxsReplay service, tracks processing tips.

```rust
struct DxsConsumer {
    stream_id: u32,
    producer_addr: SocketAddr,
    tip: u64,              // last processed seq_no
    tip_file: PathBuf,     // persisted tip
    callback: Box<dyn FnMut(WalRecord)>,
}
```

**Startup sequence:**

1. Load tip from `tip_file` (0 if missing).
2. Connect to producer's DxsReplay service.
3. Send `ReplayRequest { stream_id, from_seq_no: tip + 1 }`.
4. Process replayed records via callback, advancing tip.
5. On `CaughtUp`: transition to live processing.
6. Continue processing live records via callback.

**Tip persistence:** flush tip to `tip_file` every 10ms (batched
with other I/O). On crash, replay from last persisted tip. Records
are idempotent or deduped by `seq_no` at the consumer.

**Reconnect:** on disconnect, reconnect with backoff (1s/2s/4s/8s,
max 30s). Resume from `tip + 1`.

---

## 7. Transport for Live Path

For the inner hot-path connections (ME -> risk, risk -> gateway),
the live event path uses SPSC rings within the same host (unchanged
from current design). DXS serves the **cross-host** and **replay**
paths.

The replay/streaming path uses gRPC. For cross-host live streaming
where latency matters, gRPC over QUIC provides lower latency than
TCP (0-RTT reconnect, no head-of-line blocking). Regular gRPC over
TCP is acceptable for replay where latency is not paramount.

---

## 8. Recorder Pattern

Generic archival consumer. Same binary as any DXS consumer, with
different config. Subscribes to a producer's stream and writes to
daily archive files.

**Archive layout:**

```
archive/{stream_id}/{stream_id}_{YYYY-MM-DD}.wal
```

Same protobuf length-delimited format on disk. No transformation.

**Three recorder instances** (separate processes or config
sections):

| Instance | Stream | Source |
|----------|--------|--------|
| Market data | ME events (fills, BBO, orders) | Matching engine |
| Risk events | Risk state changes | Risk engine |
| Mark prices | MarkPriceEvent stream | Mark price aggregator |

**Daily rotation:** at UTC midnight, close current file, open new
file with next date. No retention limit on archive files (managed
externally).

**Config:**

```toml
[recorder]
stream_id = 1
producer_addr = "10.0.0.1:9100"
archive_dir = "./archive"
```

**File organization:**

```
crates/rsx-recorder/src/
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

## 10. Performance Targets

| Operation | Target |
|-----------|--------|
| WAL append (in-memory) | <200ns |
| WAL flush (fsync) | <1ms per 64KB batch |
| WAL read (sequential) | >500 MB/s |
| Replay 100K records | <1s |
| Recorder sustained write | >100K records/s |
| Tip persist | every 10ms, batched |

---

## 11. File Organization

```
crates/rsx-dxs/src/
    lib.rs        -- public API: WalWriter, WalReader, DxsConsumer
    wal.rs        -- WalWriter, WalReader, file layout, GC
    server.rs     -- DxsReplay gRPC service
    client.rs     -- DxsConsumer, tip tracking, reconnect
    recorder.rs   -- Recorder (daily archival callback)
    config.rs     -- TOML config structs

crates/rsx-recorder/src/
    main.rs       -- recorder binary entrypoint
```
