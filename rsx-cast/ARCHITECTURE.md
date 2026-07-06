# rsx-cast Architecture

**In plain terms:** this is the pipe that carries every order and fill
between the exchange's machines without losing one. If a packet drops,
the receiver asks for it again — and the resend comes from the same
on-disk log the system already keeps for replay and audit, so there is
no second copy to drift out of sync. One bytestream does three jobs:
live wire, disk, replay.

Domain-agnostic reliable transport. WAL writer/reader, replication
TCP replay server/client, and casting (C Message Protocol)
UDP sender/receiver.

Byte-exact protocol specs live in the exchange repo:
[4-cast](https://github.com/kronael/rsx/blob/master/specs/2/4-cast.md) (UDP/NAK),
[10-replication](https://github.com/kronael/rsx/blob/master/specs/2/10-replication.md) (TCP catch-up),
[48-wal](https://github.com/kronael/rsx/blob/master/specs/2/48-wal.md) (on-disk format).

## Domain-agnostic transport

rsx-cast has zero workspace dependencies. The crate carries
only the framing (`WalHeader`), the transport-level records
(heartbeat, NAK, replay request, caught-up,
replication-not-available), and the [`CastRecord`] trait that
domain payloads must implement.

```
$ cargo tree -p rsx-cast --edges normal | grep '^[├└]── rsx-'
(empty — no rsx- crates in normal deps)
```

The wider rsx exchange's domain records (`FillRecord`,
`BboRecord`, `OrderInsertedRecord`, …) live in a separate
crate that sits on top of rsx-cast; nothing in this crate
depends on them. Any consumer crate can define its own
`#[repr(C, align(64))]` records that impl `CastRecord` and
ride the same transport.

## Module layout (`rsx-cast/src/`)

| File | Purpose |
|------|---------|
| `header.rs` | 16-byte `WalHeader`. Version byte at offset 0 (`V1` = current; `V0` retired in `64dda88`). Reserved bytes per layout below; `_pad0`, `_pad1`, `_reserved` must be zero. |
| `records.rs` | `CastRecord` trait + protocol records (`CastHeartbeat`, `Nak`, `ReplicationRequest`, `CaughtUpRecord`, `ReplicationNotAvailable`). Compile-time size/align asserts on each. |
| `encode_utils.rs` | Generic helpers: `compute_crc32` (CRC32C / Castagnoli), `as_bytes`, `encode_record`, `decode_payload<T: Copy>`. No domain knowledge. |
| `cast.rs` | `CastSender` + `CastReceiver` (UDP, sync). Two-tier NAK: preallocated send_ring (hot) → WAL `read_record_at_seq` (cold). |
| `wal.rs` | `WalWriter` (10ms flush, 64MB rotate, 4h retention GC) + `WalReader` + `read_record_at_seq`. |
| `replication_server.rs` | `ReplicationService` (TCP, TLS mandatory). Sends `ReplicationNotAvailable` when `from_seq` precedes oldest WAL seq. |
| `replication_client.rs` | `ReplicationConsumer` (TCP replay, sync). Multi-endpoint: tries endpoints newest→oldest; advances on `ReplicationNotAvailable`. Backoff 1/2/4/8/30s ±20% jitter. |
| `config.rs` | `CastConfig`, `TlsConfig`. Every field documents its env var. |

## Transport paths

- **casting/UDP** (hot path): Aeron-inspired NAK recovery, but
  without flow control. `CastSender` assigns monotonic seq,
  sends, and caches the encoded frame in a preallocated ring.
  `CastReceiver` detects gaps from out-of-order delivery or
  from idle-tail heartbeat skew and sends `Nak`. Sender
  retransmits from the ring; if the seq has aged out, falls
  back to `read_record_at_seq` against the WAL. Retransmit
  horizon = WAL retention, not buffer size. Slow consumers do
  not pace the sender — receiver-side overflow drops, sender
  never stalls.
- **replication/TCP** (cold path): `ReplicationService` streams
  historical records from `WalReader` then transitions to a
  live tail on `WalWriter::add_listener` notifications.
  `ReplicationConsumer` resumes from a persisted tip with backoff
  on disconnect.

## casting sender ring (cast.rs)

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
sender loop on `u64::MAX`.

The **receive** path has two entry points: `try_recv_with`
delivers the payload as a `&[u8]` into the receiver's internal
packet buffer via an `FnOnce(WalHeader, &[u8])` callback (zero
allocation — use on the hot path); `try_recv` is a convenience
wrapper that returns an owned `Vec<u8>` per in-order packet.
Out-of-order frames land in a separate 2048-slot reorder ring
(`reorder_seqs` / `reorder_lens` / `reorder_frames`); overflow
returns `Reconnect`.

## WAL record format

```
struct WalHeader {       // 16 bytes
    version:     u8,     // offset 0      (V1 = 1; V0 retired)
    _pad0:       u8,     // offset 1
    record_type: u16,    // offset 2..4
    len:         u16,    // offset 4..6   (payload bytes)
    _pad1:       u16,    // offset 6..8
    crc32:       u32,    // offset 8..12  (CRC32C of payload)
    _reserved:   [u8; 4],// offset 12..16 (must be zero)
}
```

Payload is `#[repr(C, align(64))]`, little-endian. Data
records carry `seq: u64` at offset 0 (enforced via
`CastRecord`).

**Version policy.** Adding a new record_type does NOT bump
the wire version — record types are additive. Bumping V1 →
V2 is reserved for changes that would break a V1 reader
(re-layout, different CRC algorithm) and requires a
coordinated stop-redeploy. V0 (legacy zero) was retired in
`64dda88` when the version byte moved to offset 0; readers
no longer accept it.

## WalWriter internals

- **Append**: assigns monotonic seq, encodes into in-memory
  buf. O(1) memcpy.
- **Flush**: producer's tick calls `flush()` every 10ms;
  writes buf to active file + fsync.
- **Rotation**: on flush, if `file_size >= max_file_size`
  (64MB default), close active file, rename with seq range,
  open new active file, run GC.
- **GC**: `prune_old_segments` runs at the end of every
  `rotate()`. mtime-based; deletes rotated segments older than
  `RETENTION_NS` (4h; const in `wal.rs`). Best-effort — an
  unlink failure is logged and skipped, never propagated. The
  active file is never touched.
- **Backpressure**: append blocks when buf > 2x
  `max_file_size`.

File layout:

```
wal/{stream_id}/{stream_id}_{first_seq}_{last_seq}.wal
wal/{stream_id}/{stream_id}_active.wal
```

Filenames encode the seq range. `read_record_at_seq` picks the
segment whose `[first_seq, last_seq]` covers the target (linear
scan of the file list, bounded by retention), then scans that
one file for the record. No file header, no index.

## Replay Protocol (replication) — server.rs

```
1. Consumer connects over TLS (mandatory; rustls + aws-lc-rs).
2. Consumer sends ReplicationRequest { stream_id, from_seq }
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

`ReplicationConsumer` retries disconnects with exponential backoff
(1, 2, 4, 8, 30 seconds) and ±20% jitter (no `rand` dep —
nanosecond mod 1000). Backoff index resets on a successful
stream.

## Trust model

The two transports are asymmetric by design:

- **casting/UDP is intentionally unauthenticated and plaintext**
  — "trusted internal network, no authentication, no encryption"
  (spec 4-cast §10.4). This is the hot order-flow path; adding
  per-frame crypto there would tax the zero-copy ingress for no
  gain on a trusted LAN.
- **replication/TCP mandates TLS** (rustls + aws-lc-rs). The cold
  catch-up/federation hop can cross a wider trust boundary
  (cross-host, cross-DC replay), and it is off the latency
  critical path, so encrypting it is cheap insurance. There is no
  plaintext fallback: `ReplicationService::new` /
  `ReplicationConsumer::new` require a cert (server) / CA
  (client), and `TlsConfig::from_env` errors when certs are
  absent.

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
WAL bytes = disk bytes = casting/UDP bytes = replication/TCP bytes
         = struct bytes in memory
```

The same `#[repr(C, align(64))]` payload appears in all
four contexts. CRC32C (Castagnoli) in the header covers the
payload only.

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

All p50 unless noted. Single 6-core box, Linux 6.1, loopback,
closed-loop, casting/raw-UDP threads pinned. Headline latency and
WAL-flush figures are the **2026-07-03** run
(`reports/20260703_cast-benches.md`); encode/decode, sequential
read, and cold-tier random-read are from `cast_bench` /
`wal_random_read_bench` (earlier pass, same host). See
[BENCHES.md](BENCHES.md) for per-bench attribution,
[`compare/README.md`](compare/README.md) for the same-harness
comparison against Aeron / MoldUDP64 / SoupBinTCP / Quinn / KCP /
raw UDP / TCP, and
[`facts/cast-vs-udp-overhead.md`](https://github.com/kronael/rsx/blob/master/facts/cast-vs-udp-overhead.md)
for the dated attribution breakdown. casting's loopback RTT
(8.80 µs) sits at the raw-UDP floor (8.75 µs) — the protocol adds
~0 µs; ~99 % of the send body is the `sendto` syscall.

| Operation | Measured | Bench |
|---|---:|---|
| Protocol-record encode (`Nak` / `CastHeartbeat`) | 43 ns | `cast_bench` |
| Protocol-record decode | 9 ns | `cast_bench` |
| `WalWriter::prepare` + `append_framed` (`Vec` extend, no disk I/O) | 36 ns | `wal_bench` |
| WAL flush + fsync, 1 record | 363 µs | `wal_fsync_bench` (real disk, core-pinned) |
| WAL flush + fsync, 100 records | 475 µs | `wal_fsync_bench` — fsync dominates |
| WAL flush + fsync, 1 000 records | 940 µs | `wal_fsync_bench` — fsync still dominant |
| WAL flush + fsync, 10 000 records | 4.82 ms | `wal_fsync_bench` — append overhead visible |
| WAL sequential read | ~700 MB/s | `wal_bench` |
| casting RTT, loopback, 128 B | 8.80 µs | `cast_rtt_bench` |
| casting one-way, loopback, 128 B | 4.74 µs | `cast_one_way_bench` |
| Raw UDP RTT (baseline), loopback, 128 B | 8.75 µs | `compare_all::raw_udp_128b` |
| frame + send body (`Framed::pack` + `send_framed`) | ~3.6 µs (~99 % `sendto`) | `cast_send_breakdown_bench` |
| Cold-tier NAK retransmit (`read_record_at_seq`), 10 K records | 10.4 ms | `wal_random_read_bench` |
| Cold-tier NAK retransmit (`read_record_at_seq`), 100 K records | 80.6 ms | `wal_random_read_bench` |

## Connection topology

```
Gateway --[casting/UDP]--> Risk --[casting/UDP]--> ME
Gateway <--[casting/UDP]-- Risk <--[casting/UDP]-- ME
                                      ME --[SPSC]--> WalWriter
                              WalWriter --[notify]--> ReplicationService
                                      ME --[casting/UDP]--> Marketdata
Mark --[replication/TCP]------> Risk
Recorder --[replication/TCP]--> ME
```

## Consumers

| Consumer | Source | Purpose |
|----------|--------|---------|
| Risk | ME WAL | Fill ingestion, position update |
| Risk | Mark WAL | Mark price feed |
| Marketdata | ME WAL | Shadow book bootstrap |
| Recorder | ME WAL | Daily archival |

## Architectural Decisions

**Runtime: none — transport library.** `rsx-cast` is
domain-agnostic and runtime-agnostic. All types —
`CastSender`, `CastReceiver`, `WalWriter`, `WalReader`,
`ReplicationService`, `ReplicationConsumer` — are synchronous.
Callers drive them from whatever loop suits their needs:
a pinned tile spin loop, a tokio task, or a monoio reactor.
No async wrappers are shipped; the crate carries no runtime
dependency.

This is intentional: consumers pick the runtime that fits
their stage. Matching engine drives `CastSender` from a
pinned tile loop with no reactor at all. Gateway and
marketdata own the UDP socket and pass raw bytes to
`CastReceiver` (invert-ownership pattern — see `cast.rs`).
Recorder drives `ReplicationConsumer` blocking from its own thread.
The transport sits under all of them without preference.
