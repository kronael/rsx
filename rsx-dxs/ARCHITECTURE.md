# rsx-dxs Architecture

Domain-agnostic reliable transport. WAL writer/reader, DXS
TCP replay server/client, and CMP (C Message Protocol)
UDP sender/receiver.

Specs: [`specs/4-cmp.md`](specs/4-cmp.md),
[`specs/10-dxs.md`](specs/10-dxs.md),
[`specs/48-wal.md`](specs/48-wal.md).

## Domain-agnostic transport

rsx-dxs has zero workspace dependencies. The crate carries
only the framing (`WalHeader`), the transport-level records
(heartbeat, status, NAK, replay request, caught-up), and the
[`CmpRecord`] trait that domain payloads must implement.

```
$ cargo tree -p rsx-dxs --edges normal | grep '^[├└]── rsx-'
(empty — no rsx- crates in normal deps)
```

The wider rsx exchange's domain records (`FillRecord`,
`BboRecord`, `OrderInsertedRecord`, …) live in a separate
crate that sits on top of rsx-dxs; nothing in this crate
depends on them. Any consumer crate can define its own
`#[repr(C, align(64))]` records that impl `CmpRecord` and
ride the same transport.

## Module layout (`rsx-dxs/src/`)

| File | Purpose |
|------|---------|
| `header.rs` | 16-byte `WalHeader`. Version byte at offset 8 (`V0` = legacy zero, `V1` = current). Reserved bytes 9..16 must be zero. |
| `protocol.rs` | `CmpRecord` trait + four protocol records (`CmpHeartbeat`, `Nak`, `ReplayRequest`, `CaughtUpRecord`). Each has compile-time `size_of` + `align_of` asserts. Re-exported as `records` for back-compat. `StatusMessage` and the associated flow-control window were removed in `87b223e` — CMP has no per-receiver pacing now. |
| `encode_utils.rs` | Generic helpers: `compute_crc32`, `as_bytes`, `encode_record`, `decode_payload<T: Copy>`. No domain knowledge. |
| `cmp.rs` | `CmpSender` + `CmpReceiver` (UDP). Two-tier NAK retransmit: preallocated send_ring (hot) → WAL `read_record_at_seq` (cold). |
| `wal.rs` | `WalWriter` (10ms flush, 64MB rotate, retention GC) + `WalReader` + `read_record_at_seq` for random access. |
| `server.rs` | `DxsReplayService` (TCP, optional TLS). Verifies version byte + CRC before any `unsafe` cast. |
| `client.rs` | `DxsConsumer` (TCP replay) with exponential backoff 1/2/4/8/30s and ±20% jitter. |
| `config.rs` | `CmpConfig`, `TlsConfig`. Every field documents its env var. |

## Transport paths

- **CMP/UDP** (hot path): Aeron-inspired NAK recovery, but
  without flow control. `CmpSender` assigns monotonic seq,
  sends, and caches the encoded frame in a preallocated ring.
  `CmpReceiver` detects gaps from out-of-order delivery or
  from idle-tail heartbeat skew and sends `Nak`. Sender
  retransmits from the ring; if the seq has aged out, falls
  back to `read_record_at_seq` against the WAL. Retransmit
  horizon = WAL retention, not buffer size. Slow consumers do
  not pace the sender — receiver-side overflow drops, sender
  never stalls.
- **DXS/TCP** (cold path): `DxsReplayService` streams
  historical records from `WalReader` then transitions to a
  live tail on `WalWriter::add_listener` notifications.
  `DxsConsumer` resumes from a persisted tip with backoff
  on disconnect.

## CMP sender ring (cmp.rs)

Three `Box<[T]>` slabs, indexed by `seq & SEND_RING_MASK`:

```
ring_seqs:   Box<[u64]>   capacity 4096   slot's current seq (0 = empty)
ring_lens:   Box<[u16]>   capacity 4096   encoded frame length
ring_frames: Box<[u8]>    capacity 4096 * 128 B
```

Zero allocations on the hot **send** path. On NAK, the sender
checks `ring_seqs[slot] == seq` (cache hit) or falls back to
`read_record_at_seq` (cache miss). NAK counts are clamped to
`SEND_RING_CAPACITY` so a malicious peer can't make the
sender loop on `u64::MAX`. The receive path
(`CmpReceiver::try_recv`) currently allocates one `Vec<u8>`
per in-order packet; a caller-supplied `&mut [u8]` variant
is future work.

## WAL record format

```
struct WalHeader {       // 16 bytes
    record_type: u16,    // offset 0..2
    len:         u16,    // offset 2..4  (payload bytes)
    crc32:       u32,    // offset 4..8  (CRC32 of payload)
    version:     u8,     // offset 8     (V0 = 0, V1 = 1)
    _reserved:   [u8; 7],// offset 9..16 (must be zero)
}
```

Payload is `#[repr(C, align(64))]`, little-endian. Data
records carry `seq: u64` at offset 0 (enforced via
`CmpRecord`).

**Version policy.** Adding a new record_type does NOT bump
the wire version — record types are additive. Bumping V1 →
V2 is reserved for changes that would break a V1 reader
(re-layout, different CRC algorithm) and requires a
coordinated stop-redeploy. V0 (legacy zero) is still
accepted on read; never emitted on write.

## WalWriter internals

- **Append**: assigns monotonic seq, encodes into in-memory
  buf. O(1) memcpy.
- **Flush**: producer's tick calls `flush()` every 10ms;
  writes buf to active file + fsync.
- **Rotation**: on flush, if `file_size >= max_file_size`
  (64MB default), close active file, rename with seq range,
  open new active file, run GC.
- **GC**: mtime-based; delete files older than retention
  (10min default).
- **Backpressure**: append blocks when buf > 2x
  `max_file_size`.

File layout:

```
wal/{stream_id}/{stream_id}_{first_seq}_{last_seq}.wal
wal/{stream_id}/{stream_id}_active.wal
```

Filenames encode the seq range — O(1) file selection for
`read_record_at_seq`. No file header, no index.

## DXS replay protocol (server.rs)

```
1. Consumer connects (optional TLS).
2. Consumer sends ReplayRequest { stream_id, from_seq }
   as one framed record.
3. Server validates header version, validates CRC, then
   casts the payload (in that order — no unsafe before
   the integrity check).
4. Server opens WalReader at from_seq and streams raw
   WAL bytes (header + payload, no transformation).
5. On catch-up, emits CaughtUpRecord { live_seq }; consumer
   resumes at live_seq + 1.
6. Transitions to live broadcast driven by
   WalWriter::add_listener notifications.
```

`DxsConsumer` retries disconnects with exponential backoff
(1, 2, 4, 8, 30 seconds) and ±20% jitter (no `rand` dep —
nanosecond mod 1000). Backoff index resets on a successful
stream.

## Trust model

CMP is **intentionally unauthenticated**. See
[`specs/4-cmp.md`](specs/4-cmp.md) §10.4 ("Trusted internal
network. No authentication, no encryption.").

- External clients are authenticated at the **gateway**
  (JWT + TLS).
- Internal RSX peers are isolated at **L3** (firewall,
  VPC, namespace).
- A per-frame source-IP filter was prototyped and reverted
  (commit `bde3211`). Do not reintroduce — it duplicates the
  L3 owner and complicates the zero-copy ingress path.

If cross-DC peer auth is ever needed, the right place is a
sealed-frame extension under a new `WalHeader.version`, not
a retrofit on the V1 ingress.

## Wire-format invariant

```
WAL bytes = disk bytes = CMP/UDP bytes = DXS/TCP bytes
         = struct bytes in memory
```

The same `#[repr(C, align(64))]` payload appears in all
four contexts. CRC32 in the header covers the payload only.

## Idempotent replay

Consumers dedup by `seq`. Risk treats any record with
`seq <= tips[stream_id]` as a no-op. Tips persist every
10ms; recovery resumes from `tip + 1`.

## Edge cases

- Crash mid-rotation: active file recovered by CRC scan;
  trailing partial record truncated.
- Partial record at EOF: detected, truncated.
- CRC mismatch: conservative truncation at first bad record.
- Unknown record_type: returned raw, consumer skips.
- Unknown header version: rejected on TCP ingress, dropped
  on UDP control path.
- Gap beyond send_ring + WAL retention: NAK fails;
  consumer must use archive fallback.

## Measured performance

All p50 unless noted. Host: AMD Ryzen 9 5950X (6-core
slice), Linux 6.1, Rust release, threads pinned. See
[BENCHES.md](BENCHES.md) for per-bench attribution and
[`facts/cmp-vs-udp-overhead.md`](https://github.com/kronael/rsx/blob/master/facts/cmp-vs-udp-overhead.md)
for the dated authoritative numbers.

| Operation | Measured | Bench |
|---|---:|---|
| Protocol-record encode (`Nak` / `CmpHeartbeat`) | 43 ns | `cmp_bench` |
| `FillRecord` encode | 23 ns | parent `rsx-messages` `encode_bench` |
| Protocol-record decode | 9 ns | `cmp_bench` |
| `WalWriter::append` (`Vec` extend, no disk I/O) | 31 ns | `wal_bench` |
| WAL flush + fsync (64 KB batch — production amortisation) | 24 µs | `wal_fsync_bench` batch variant |
| WAL flush + fsync (single record — naive sync per append) | 651 µs | `wal_fsync_bench` single-record variant |
| WAL sequential read | ~700 MB/s | `wal_bench` |
| CMP RTT, loopback, 128 B | 11.26 µs | `cmp_rtt_bench` |
| Raw UDP RTT (baseline), loopback, 128 B | 9.89 µs | `compare_udp` |
| `CmpSender::send` body (per call) | ~4.07 µs (99 % `sendto`) | `cmp_send_breakdown_bench` |
| Cold-tier NAK retransmit (`read_record_at_seq`) | 23.5 ms @ 10 K records | `wal_random_read_bench` |

## Connection topology

```
Gateway --[CMP/UDP]--> Risk --[CMP/UDP]--> ME
Gateway <--[CMP/UDP]-- Risk <--[CMP/UDP]-- ME
                                      ME --[SPSC]--> WalWriter
                              WalWriter --[notify]--> DxsReplay
                                      ME --[CMP/UDP]--> Marketdata
Mark --[DXS/TCP]------> Risk
Recorder --[DXS/TCP]--> ME
```

## Consumers

| Consumer | Source | Purpose |
|----------|--------|---------|
| Risk | ME WAL | Fill ingestion, position update |
| Risk | Mark WAL | Mark price feed |
| Marketdata | ME WAL | Shadow book bootstrap |
| Recorder | ME WAL | Daily archival |
