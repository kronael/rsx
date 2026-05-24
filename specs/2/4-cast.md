---
status: shipped
---

# CMP — C Message Protocol

CMP carries WAL records over UDP between Gateway, Risk, and
Matching Engine. The same bytes go to disk (WAL), to the
network (CMP/UDP), and into shared memory. No serialization
step. Sequence numbers, NAK-based gap recovery, sender-stall
flow control.

This document specifies the wire format byte-by-byte, the
reliability and flow-control semantics, and the limits of
the current implementation. Where the spec previously
hand-waved or contradicted the code, this revision states
the actual behaviour.

Implementation: `rsx-dxs/src/cmp.rs`,
`rsx-dxs/src/protocol.rs` (CmpRecord trait + control messages),
`rsx-dxs/src/header.rs`. Domain wire records: `rsx-messages/`.

## Table of contents

- [1. What CMP is and isn't](#1-what-cmp-is-and-isnt)
- [2. Wire format](#2-wire-format)
- [3. Sequence numbers and CmpRecord](#3-sequence-numbers-and-cmprecord)
- [4. Control messages](#4-control-messages)
- [5. Flow control](#5-flow-control)
- [6. Loss recovery (NAK)](#6-loss-recovery-nak)
- [7. WAL replication over TCP (cold path)](#7-wal-replication-over-tcp-cold-path)
- [8. Comparison with related protocols](#8-comparison-with-related-protocols)
- [9. Performance](#9-performance)
- [10. Known limits and design tradeoffs](#10-known-limits-and-design-tradeoffs)
- [11. Configuration](#11-configuration)
- [Cross-references](#cross-references)

---

## 1. What CMP is and isn't

CMP is **the C-struct wire format** plus **the UDP transport
glue** (sequencing, flow control, gap recovery) that carries
RSX's WAL records between processes on the hot path.

The transport layer (`rsx-dxs`) is **domain-agnostic** — it
moves any record that implements `CmpRecord` (a `repr(C)`
type with a `seq: u64` at offset 0). RSX's exchange records
(`FillRecord`, `BboRecord`, …) live in the `rsx-messages`
crate and are not visible to the transport.

CMP is **not** a general-purpose framework. It assumes:
- One sender per stream, one receiver per stream
  (point-to-point, not multicast). For fan-out, run multiple
  senders.
- Trusted internal network. No authentication, no encryption.
  External clients use the WebSocket JSON path on the gateway.
- Little-endian x86_64 (or aarch64 LE). Compile-time check.
- All endpoints are RSX processes built from the same repo.

What CMP gives up to be fast:
- No connection handshake (just `bind` + `sendto`).
- No congestion control. The dedicated network is sized for
  peak load; senders stall on flow-control window only.
- No fragmentation. Each WAL record is one UDP datagram.
- No schema evolution beyond appending new record types.

## 2. Wire format

Every CMP datagram is a 16-byte WAL header followed by a
fixed-size, `#[repr(C, align(64))]` payload. **WAL bytes =
disk bytes = wire bytes = memory bytes.**

```
struct WalHeader {        // 16 bytes, manual encode
    record_type: u16,     // see RECORD_* constants
    len: u16,             // payload length in bytes
    crc32: u32,           // CRC32C of payload
    version: u8,          // wire-format version (V0 or V1)
    _reserved: [u8; 7],   // reserved, must be zero
}
```

All fields little-endian. `version` at offset 8 carries the
wire-format version (`0` = legacy zero-reserved layout,
accepted on read for back-compat; `1` = current, written by
all new senders). Receivers reject unknown versions. The
remaining 7 reserved bytes must be zero. See §10.2 for
schema-evolution semantics.

### Payload size

`len` is `u16`, so the protocol bound is **65 535 bytes**
per datagram. The receiver enforces this implicitly: any
datagram larger than `WalHeader::SIZE + 65 535 = 65 551`
bytes cannot be parsed because `len` cannot represent it.
The send buffer is sized at 65 536 bytes (`PACKET_BUF_SIZE`,
`cmp.rs:23`).

In **practice**, every CMP record currently in use is
≤ 64 bytes (one cache line) — `OrderRequest`, `Fill`,
`OrderInsertedRecord`, etc. are all
`#[repr(C, align(64))]` with explicit padding to 64 bytes.
So all live datagrams fit in one MTU, no fragmentation
risk, and the cache-line alignment matches the alignment
of the in-memory layout.

The 65 535 ceiling exists so that future record types
(e.g. snapshot blobs) can grow beyond 64 bytes without a
wire-format change, while the current hot path stays
cache-line-sized.

### CRC

`crc32` is CRC32C (Castagnoli) over the payload only —
**not** over the header. Receivers recompute and discard
mismatches silently (`cmp.rs:373-376`). This catches bit
errors on the wire, not malicious tampering; CMP has no
authentication.

### Endianness and platform

`#[repr(C)]` with explicit field order. Compile-time assert
on `cfg(target_endian = "little")`. Big-endian is not
supported.

## 3. Sequence numbers and CmpRecord

Data records implement the `CmpRecord` trait
(`records.rs:15-28`):

```rust
pub trait CmpRecord: Copy + Sized {
    const RECORD_TYPE: u16;
    fn seq(&self) -> u64;
    fn set_seq(&mut self, seq: u64);
    fn record_type() -> u16 { Self::RECORD_TYPE }
}
```

The first 8 bytes of every data payload are `seq: u64`,
monotonic per stream, assigned by `CmpSender::send` at
transmission (`cmp.rs:94`). The receiver uses
`extract_seq(payload: &[u8])` (`wal.rs::extract_seq`) to
read it back without unpacking the rest.

Control messages (StatusMessage, Nak, CmpHeartbeat) do
**not** implement `CmpRecord` and have no `seq` field.
They're identified solely by `record_type` in the header.

Sequence numbers start at 1 (`cmp.rs:67`) and are never
reused within a stream. On sender restart with the same
`stream_id`, a fresh sequence space starts at 1; receivers
detect this from heartbeats and resync.

## 4. Control messages

Three control record types, fixed-size, 64-byte aligned:

```
RECORD_STATUS_MESSAGE = 0x10  // receiver -> sender
RECORD_NAK            = 0x11  // receiver -> sender
RECORD_HEARTBEAT      = 0x12  // sender -> receiver
```

### StatusMessage (every 10 ms, receiver → sender)

```rust
#[repr(C, align(64))]
pub struct StatusMessage {
    pub consumption_seq: u64,    // last fully-received seq
    pub receiver_window: u64,    // seq delta receiver allows
    pub _pad1: [u8; 48],
}
```

`receiver_window` is a **count of records** (a sequence-
number delta), not bytes. The sender stalls when
`next_seq > consumption_seq + receiver_window`
(`cmp.rs:88-91`). With the default window of 65 536
(`config.rs::default_window`), at the live record size
of ~64 B that's about 4 MB of in-flight data.

The previous spec said "bytes" — that was wrong. The code
has always treated this field as a record count.

### Nak (on gap detection, receiver → sender)

```rust
#[repr(C, align(64))]
pub struct Nak {
    pub from_seq: u64,    // first missing seq
    pub count: u64,       // number of consecutive missing
    pub _pad1: [u8; 48],
}
```

### CmpHeartbeat (every 10 ms, sender → receiver)

```rust
#[repr(C, align(64))]
pub struct CmpHeartbeat {
    pub highest_seq: u64,    // last seq sent
    pub _pad1: [u8; 56],
}
```

Heartbeats let an idle receiver detect a gap that ends
the stream (no new data records arrive to expose the
missing tail).

## 5. Flow control

Sender keeps a window of `receiver_window` records ahead
of `consumption_seq`. On `send`:

```
limit = peer_consumption_seq + peer_window
if next_seq > limit && limit > 0 {
    return Ok(false);  // caller must retry or drop
}
```

`limit > 0` means "until we get the first StatusMessage,
don't stall" — bootstrap path so the very first records
flow before the receiver has a chance to advertise a
window.

The receiver advertises its window in every StatusMessage
(every 10 ms). It can shrink the window by advertising a
smaller value; the sender will stall until the window
reopens.

There is no congestion control beyond this. The deployed
network is dimensioned for peak; if receivers can't keep
up, the sender backpressure stops the matching engine.

## 6. Loss recovery (NAK)

UDP has no retransmit. CMP layers a NAK loop on top.

### Detection

The receiver tracks `expected_seq` (next seq it wants)
and `highest_seen`. A gap is detected two ways:

1. A **data record** arrives with `seq > expected_seq`.
   The receiver stages it in the reorder ring (see below)
   and calls `maybe_nak()`.
2. A **heartbeat** arrives with `highest_seq > expected_seq`
   while no new data is arriving. The heartbeat handler
   calls `maybe_nak()`.

`maybe_nak()` is the only NAK emitter. It sends a NAK for
the **oldest contiguous missing run** starting at
`expected_seq`, rate-limited by `nak_retry_us` (default
100 µs). Every later gap is gated behind the oldest by the
FIFO contract, so re-NAKing later gaps before the head
clears would be wasted work.

NAKs are dedup-suppressed on the sender side too:
`CmpSender` tracks `ring_last_retx_ns[slot]` and skips
retransmits within `retx_dedup_window_us` (default 1 ms).
The two windows compose — a receiver re-NAK and a sender
re-retransmit can't pile up.

### Retransmit source — two-tier

When the sender receives a NAK, it walks `from_seq ..
from_seq + count`. For each seq:

1. **Hot tier — `send_ring`.** Three preallocated
   `Box<[T]>` slabs indexed by `seq & MASK`:
   `ring_seqs: Box<[u64; 4096]>`,
   `ring_lens: Box<[u16; 4096]>`,
   `ring_frames: Box<[u8; 4096 * 128]>`. Slot index is
   `seq & SEND_RING_MASK` (capacity is a power of two —
   bitwise AND, no modulo). One-shot allocation at
   construction; **zero heap allocations on the send path**.
   Lookup checks the slot's seq matches the requested seq
   before re-sending; on mismatch the ring has wrapped past
   that seq and the cold tier (WAL) is consulted.
   Records larger than `SEND_RING_FRAME_BYTES = 128` bypass
   the ring entirely and force NAK to fall through to WAL.
   Covers the common case of a NAK arriving within ~400 ms
   of the lost record at typical rates.

2. **Cold tier — WAL random-access.** On ring miss, the
   sender calls `read_record_at_seq(stream_id, seq,
   wal_dir, None)` (`rsx-dxs/src/wal.rs`). The function
   identifies the right file by its filename
   (`{stream_id}_{first_seq}_{last_seq}.wal`), opens it,
   and scans forward record-by-record until it hits the
   target seq. Cost: one file open + a sequential scan
   of up to one rotation worth (≤ 64 MB). On modern NVMe
   that's microseconds for cached pages, milliseconds for
   cold ones. The active file is searched too if the
   target post-dates the last rotation.

Only when both tiers miss does the sender log and drop:
the seq has been GC'd past WAL retention (default 10
minutes). At that point the receiver's only recourse is a
full TCP replay from `tip + 1`.

The previous spec described retransmit as "fetches
missing records from WAL" without the two-tier story —
that wording was actually correct in spirit but the code
only had the in-memory ring. The two-tier path is now
shipped and unit-tested
(`rsx-dxs/tests/wal_test.rs::read_record_at_seq_*`).

### Reorder buffer + FAULTED escalation

The receiver buffers out-of-order packets in a fixed-size
ring mirroring the sender's `send_ring`:

```rust
const REORDER_CAPACITY: usize = 2048;  // power of 2
const REORDER_MASK: u64 = (REORDER_CAPACITY - 1) as u64;
const REORDER_FRAME_BYTES: usize = 256;

reorder_seqs:   Box<[u64; 2048]>
reorder_lens:   Box<[u16; 2048]>
reorder_frames: Box<[u8; 2048 * 256]>
```

Slot index is `seq & REORDER_MASK`. Empty slot iff
`reorder_seqs[slot] == 0`. Memory: 512 KB per receiver,
pre-allocated at construction; **zero heap allocations on
the hot receive path**.

`REORDER_CAPACITY = 2048` sets the maximum in-flight gap
window. At 10 k pps that's ~200 ms of burst tolerance; at
1 k pps, 2 s — comfortable margin above realistic LAN
hiccups. Bigger gaps escalate to FAULTED + DXS replay.

### Three-tier delivery contract

Within a stream, CMP guarantees **strict FIFO**: the
application sees seqs in monotonic order with **no silent
skip** path. Loss recovery happens in three tiers:

1. **NAK (in-band).** Receiver detects the gap, fires NAK,
   sender retransmits from `send_ring` (hot) or WAL (cold).
   Bounded by `nak_retry_us × max_nak_retries`
   (100 µs × 8 = 800 µs default total recovery budget).

2. **FAULTED.** If the gap persists past the in-band budget,
   OR an arriving out-of-order packet would collide with a
   different seq already in its slot (gap >
   `REORDER_CAPACITY`), the receiver enters sticky FAULTED
   state and surfaces `CmpRecv::Faulted { last_delivered_seq,
   gap_start, gap_end_inclusive }` to the consumer. No
   silent advance: the receiver keeps returning `Faulted`
   on every `try_recv` call until the consumer calls
   `reset_after_replay(new_tip)`.

3. **DXS replay (out-of-band).** The consumer handles
   `Faulted` by switching to TCP-based WAL replay from
   `last_delivered_seq + 1` via `DxsConsumer`. Once caught
   up, it calls `CmpReceiver::reset_after_replay(new_tip)`
   to clear the FAULTED state and resume normal in-band
   delivery from `new_tip + 1`.

This contract preserves invariant #1 (FIFO per stream)
absolutely. The old 512-slot `BTreeMap` reorder buffer
silently advanced past the gap on overflow — a real
correctness bug. v4 removed that path.

### Reset semantics

`reset_after_replay(new_tip)` sets `expected_seq = new_tip + 1`
so live-tail delivery resumes from the right place. It also
clears the FAULTED flag and drops stale reorder-ring entries.

`highest_seen` is **monotonic**: if `new_tip` is below the
current `highest_seen` (e.g. a heartbeat or stray OOO packet
advanced `highest_seen` past the replay's stop point while
the consumer was draining DXS), the method leaves
`highest_seen` unchanged. Lowering it could re-arm the gap
detector against seqs the consumer has already applied via
replay and silently re-deliver them — a FIFO violation. The
gap detector compares `expected_seq` against `highest_seen`,
so keeping `highest_seen` ≥ `expected_seq` is what allows
the receiver to detect and NAK forward gaps; only forward
progress is observable to the consumer.

## 7. WAL replication over TCP (cold path)

Same WAL records, different transport: TCP byte stream
with optional rustls TLS. Used for replay (recover from
crash), replication (warm replicas), and archival
(rsx-recorder). Throughput-oriented, not latency-oriented.

Implementation: `rsx-dxs/src/server.rs` (`DxsReplayService`),
`rsx-dxs/src/client.rs` (`DxsConsumer`).

```
Consumer                          Producer
   |--[ReplayRequest]--TCP--------->|
   |   {stream_id, from_seq}        |
   |<--[WalRecord]----TCP-----------|
   |<--[WalRecord]----TCP-----------|
   |<--[RECORD_CAUGHT_UP]----TCP----|
   |   (live tail follows)          |
   |<--[WalRecord]----TCP-----------|
```

ReplayRequest is itself a WAL record:

```rust
#[repr(C, align(64))]
struct ReplayRequest {
    stream_id: u32,
    _pad0: u32,
    from_seq: u64,
    _pad1: [u8; 48],
}
```

No additional framing — the 16 B WAL header is its own
length-prefix. Read-side: `read_exact(16)` →
`read_exact(len)`. Reconnect uses exponential backoff
(1 / 2 / 4 / 8 / max 30 s) and resumes from `tip + 1`
based on the last-persisted tip.

## 8. Comparison with related protocols

CMP isn't novel; it's a particular fixed-point in the
design space. The comparison clarifies what's different.

| Property            | CMP/UDP        | Aeron          | kcp            | QUIC           | gRPC/HTTP2     |
|---------------------|----------------|----------------|----------------|----------------|----------------|
| Connection setup    | none (sendto)  | session SETUP  | none           | TLS handshake  | TLS+SETTINGS   |
| Wire format         | repr(C)        | length+type    | custom hdr     | varint frames  | HPACK+protobuf |
| Per-record overhead | 16 B header    | 32 B header    | 24 B header    | 5–10 B varint  | 9 B + HPACK    |
| Reliability         | NAK (4 K ring) | NAK (term log) | ARQ (sliding)  | ACK + retx     | TCP            |
| Retransmit source   | ring + WAL     | term log file  | in-mem queue   | in-mem queue   | TCP buffer     |
| Flow control        | seq window     | term position  | window         | per-stream cw  | per-stream cw  |
| Congestion control  | none           | static rate    | BBR-like       | BBR/Cubic      | TCP CC         |
| Multicast           | no             | yes            | no             | no             | no             |
| Encryption          | none           | optional       | none           | mandatory      | TLS            |
| Multi-language      | Rust only      | yes (codegen)  | C+wrappers     | many           | many           |
| Suitable for        | trusted LAN    | LAN+WAN        | gaming/UDP VPN | internet       | RPC            |

What's actually borrowed:
- **NAK-based recovery + heartbeat-driven gap exposure**
  is the Aeron protocol pattern.
- **Sequence-window flow control** is Aeron-style as well
  (Aeron uses a position field; same idea expressed in seq
  numbers).
- **`#[repr(C)]` zero-copy wire** is standard HFT practice.
- **Per-consumer streams** instead of pub/sub is a tile-
  architecture decision (one slow consumer can't stall a
  fast one).

What's **simplified vs Aeron**: CMP has no dedicated
term/page log on disk for retransmit — the application
WAL is reused (cold tier). No multi-destination multicast,
no congestion control, no session setup, no encryption.
The simplification is deliberate: less code, lower latency
on the happy path, acceptable loss recovery for an
internal LAN.

What's **simpler than QUIC**: no TLS, no per-stream
multiplexing, no head-of-line avoidance (we don't need
it — one sender, one receiver per stream). CMP would be
strictly worse than QUIC over the public internet.

## 9. Performance

### Encode / decode

Measured by `rsx-dxs/benches/wal_bench.rs`:

| Op                                          | ns      |
|---------------------------------------------|---------|
| `WalWriter::append` (Vec extend, no disk I/O) | 31    |
| WAL flush + fsync (64 KB)                   | ~24 000 |
| Sequential read (10 K recs)                 | TBD     |
| Replay (100 K recs)                         | TBD     |

`rsx-dxs/benches/cmp_bench.rs` + `rsx-messages/benches/encode_bench.rs`:

| Op                                                       | ns  |
|----------------------------------------------------------|-----|
| Protocol-record encode (StatusMessage / Nak / Heartbeat) | 43  |
| Protocol-record decode (one record)                      | 9   |
| `FillRecord` encode                                      | 23  |

These are the load-bearing measurements behind the
sub-microsecond CPU cost of one CMP frame. They do **not**
include the syscall overhead of `sendto` / `recvfrom`
(typically 500–1 000 ns on modern Linux), so the on-the-
wire round trip per packet is dominated by the syscall +
NIC, not the encode.

### End-to-end latency

The "<50 µs GW→ME→GW" target is **not currently gated by
an automated harness**. See `specs/2/22-perf-verification.md`
for the harness plan. The published unit-bench numbers
(matching, encode/decode, WAL append) sum to a budget that
fits inside 50 µs, but a continuous-integration harness
that asserts the round-trip is in
`.ship/12-SHOWCASE-HONEST/` and is **not** yet landed.

Until then, treat the 50 µs number as a **design budget**,
not a measurement.

### Future: monoio UDP transport

CMP currently uses `std::net::UdpSocket` (non-blocking,
`cmp.rs:16,62`). The rest of the gateway and marketdata
stacks use `monoio` for io_uring. Replacing CMP's UDP
sockets with monoio io_uring SQEs would eliminate one
syscall per send/recv on the hot path. This is tracked
as future work; the wire format and protocol semantics
would not change.

## 10. Known limits and design tradeoffs

These are the rough edges. They're documented so a
reader can decide whether CMP fits their use case.

### 10.1 Endianness

All fields little-endian. Compile-time `cfg` check. No
big-endian support. Acceptable: x86_64 and aarch64-LE
cover everything we deploy on.

### 10.2 Schema evolution

Cannot remove or reorder fields in existing record types
without breaking all readers. New record types are
additive; readers ignore unknown `record_type`s.

The header carries a `version: u8` at offset 8 (see §2).
Adding a new record type does **not** bump the version —
record types are additive. Bumping the version is reserved
for changes that would break a v1 reader (header layout,
CRC algorithm, alignment promises). A version bump requires
a coordinated stop-redeploy across senders and receivers:
upgrade all readers first, then flip senders.

The legacy `V0` value (zero) is the pre-version-byte format
and is accepted on read for back-compat with WALs written
before this scheme landed; new writes never emit it.
Receivers reject unknown versions outright.

### 10.3 Retransmit horizon: WAL retention

The fast path is the 4 096-entry `send_ring` (one
rotation's worth of cache-line records). Beyond that,
NAK retransmit reads from the WAL via the two-tier path
described in §6 — the practical horizon is therefore
**WAL retention**, not the ring size. Default retention
is 10 minutes on the hot WAL plus whatever the recorder
keeps in archive (effectively unbounded). A NAK for a
seq older than retention is genuinely unrecoverable on
the UDP path; the receiver must fall back to TCP replay
from `tip + 1`.

A per-file `(seq, file_offset)` index would replace the
linear scan with O(log n) lookup. The current code's
sequential-scan-within-one-file is already fast enough
on typical NAK volumes (microseconds on cached pages),
so the index hasn't been built yet.

### 10.4 No encryption, no auth

CMP is for trusted intra-datacenter traffic. External
clients hit the WebSocket JSON gateway, which terminates
TLS and validates JWT.

### 10.5 No multicast, no fan-out

One sender, one receiver per stream. Fan-out is N
independent streams (e.g. ME → marketdata, ME → recorder
each get their own UDP stream). This makes per-consumer
backpressure clean: a slow recorder can't stall the live
marketdata path.

### 10.6 Wire-format DoS surface

The `WalHeader::from_bytes` decoder is the obvious attack
surface for a malicious or buggy peer. The receiver
discards datagrams smaller than 16 B, validates `len`
against datagram size (`cmp.rs:365-368`), and validates
CRC. There is no `cargo-fuzz` target on it yet — tracked
in `.ship/12-SHOWCASE-HONEST/` task E2.

### 10.7 NAK count is clamped (was: unclamped)

A NAK with `count = u64::MAX` would loop the sender's
retransmit path. `CmpSender::handle_nak` clamps to
`SEND_RING_CAPACITY` (4 096) — anything beyond is already
unreachable on the hot tier and the cold tier (WAL) can
be requested via TCP replay. The clamp is logged when it
fires so a stuck or malicious peer is observable.
Small and pending; not a documented vulnerability because
the deploy assumes trusted peers.

### 10.8 No tcpdump/curl debuggability

Binary on the wire. The `rsx-cli wal dump` tool decodes
records to JSON for offline inspection. We debug from
WAL files, not packet captures.

## 11. Configuration

```bash
# CMP/UDP (hot path)
RSX_CAST_UDP_ADDR=127.0.0.1:9100   # receiver bind addr
                                  # sender derives dest

# WAL replication over TCP (cold path)
RSX_REPL_ADDR=127.0.0.1:9200
RSX_REPL_TLS=false
RSX_REPL_CERT_PATH=./certs/repl.pem
RSX_REPL_KEY_PATH=./certs/repl.key
```

CmpConfig (`rsx-dxs/src/config.rs`):

| Field                    | Default   | Meaning                                                   |
|--------------------------|-----------|-----------------------------------------------------------|
| `heartbeat_interval_ms`  | 100       | max idle before heartbeat; data sends reset the timer (heartbeats fire only on idle streams) |
| `send_ring_limit`        | 4 096     | sender-side retransmit cache slots                        |
| `sender_bind_addr`       | 0.0.0.0:0 | sender source addr for UDP socket                         |
| `nak_retry_us`           | 100       | receiver NAK debounce interval (oldest gap)               |
| `max_nak_retries`        | 8         | retries on oldest gap before FAULTED                      |
| `retx_dedup_window_us`   | 1 000     | sender per-seq retransmit dedup window                    |

`default_window` and `status_interval_ms` were removed when
StatusMessage / flow-control was dropped (commit `87b223e`).
Exchange-grade NAK+UDP transports don't have backpressure;
slow consumers recover via DXS replay, not by stalling the
producer.

## Cross-references

- `specs/2/10-replication.md` — DXS streaming server (TCP) on top of CMP
- `specs/2/48-wal.md` — WAL flush rules, retention, rotation
- `specs/2/20-network.md` — Process topology, port assignments
- `specs/2/45-tiles.md` — Tile architecture (within-process IPC)
- `specs/2/18-messages.md` — Record-type catalogue
- `specs/2/22-perf-verification.md` — Bench gate, latency harness
