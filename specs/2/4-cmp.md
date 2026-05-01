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

Implementation: `rsx-dxs/src/cmp.rs` (619 LoC),
`rsx-dxs/src/records.rs`, `rsx-dxs/src/header.rs`.

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
struct WalHeader {        // 16 bytes, repr(C)
    record_type: u16,     // see RECORD_* constants
    len: u16,             // payload length in bytes
    crc32: u32,           // CRC32C of payload
    _reserved: [u8; 8],   // reserved, must be zero
}
```

All fields little-endian. The reserved bytes are checked to
be zero on receive — they're available for future
extensions but no version field is allocated yet (see §10).

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

The receiver tracks `expected_seq` (next seq it wants).
A gap is detected two ways:

1. A **data record** arrives with `seq > expected_seq`.
   The receiver buffers it in a bounded reorder buffer
   (default 512 slots, `config.reorder_buf_limit`) and
   sends a NAK for `[expected_seq, seq)`.
2. A **heartbeat** arrives with `highest_seq > expected_seq`
   while no new data is arriving. The receiver NAKs the
   tail.

NAKs are **not** dedup-suppressed in the current
implementation — if loss recurs on the same range, multiple
NAKs may fly. This is a deliberate simplification; the
network is internal and loss is rare.

### Retransmit source — two-tier

When the sender receives a NAK, it walks `from_seq ..
from_seq + count`. For each seq:

1. **Hot tier — `send_ring`.** An in-memory
   `BTreeMap<u64, Vec<u8>>` holding the most recent sent
   frames (`cmp.rs:33-34`, limit 4096 entries,
   `cmp.rs:75`). O(log n) lookup, O(1) re-send. Covers
   the common case of a NAK arriving within ~400 ms of
   the lost record at typical rates.

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

### Reorder buffer

The receiver buffers up to `reorder_buf_limit` (default
512) out-of-order records while it waits for the NAK fill.
On overflow, the receiver advances `expected_seq` past the
gap, drops the missing records, and emits a structured-log
warning. The shadow-book / risk consumers handle
gap-advance via their own resync paths (CAUGHT_UP records
on TCP replay).

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
| Retransmit source   | in-mem ring    | term log file  | in-mem queue   | in-mem queue   | TCP buffer     |
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

What's **simplified vs Aeron**: CMP has no term/page log
on disk for retransmit (in-memory ring only), no
multi-destination multicast, no congestion control, no
session setup, no encryption. The simplification is
deliberate: less code, lower latency on the happy path,
acceptable loss recovery for an internal LAN.

What's **simpler than QUIC**: no TLS, no per-stream
multiplexing, no head-of-line avoidance (we don't need
it — one sender, one receiver per stream). CMP would be
strictly worse than QUIC over the public internet.

## 9. Performance

### Encode / decode

Measured by `rsx-dxs/benches/wal_bench.rs`:

| Op                          | ns      |
|-----------------------------|---------|
| WAL append (in-memory)      | 31      |
| WAL flush + fsync (64 KB)   | ~24 000 |
| Sequential read (10 K recs) | TBD     |
| Replay (100 K recs)         | TBD     |

`rsx-gateway/benches/gateway_bench.rs`:

| Op                          | ns  |
|-----------------------------|-----|
| CMP encode (one record)     | 43  |
| CMP decode (one record)     | 9   |

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
additive; readers ignore unknown `record_type`s. Breaking
changes need a coordinated deploy: stop all producers,
upgrade all readers, then producers.

There is no version field. The 8 reserved bytes in the
header are checked-zero on receive; if a future extension
needs them, both sides must roll forward together. This
is the cost of zero-copy wire bytes; we accept it.

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

### 10.7 NAK count is unclamped

A NAK with `count = u64::MAX` would loop in the sender's
retransmit path. The receiver implementation in this
repo never emits unbounded NAKs (it only NAKs gaps it
has seen), but a malicious peer could. The fix is a
per-NAK clamp at the sender (e.g., `count.min(4096)`).
Small and pending; not a documented vulnerability because
the deploy assumes trusted peers.

### 10.8 No tcpdump/curl debuggability

Binary on the wire. The `rsx-cli wal dump` tool decodes
records to JSON for offline inspection. We debug from
WAL files, not packet captures.

## 11. Configuration

```bash
# CMP/UDP (hot path)
RSX_CMP_UDP_ADDR=127.0.0.1:9100   # receiver bind addr
                                  # sender derives dest

# WAL replication over TCP (cold path)
RSX_REPL_ADDR=127.0.0.1:9200
RSX_REPL_TLS=false
RSX_REPL_CERT_PATH=./certs/repl.pem
RSX_REPL_KEY_PATH=./certs/repl.key
```

CmpConfig (`rsx-dxs/src/config.rs`):

| Field                   | Default | Meaning                              |
|-------------------------|---------|--------------------------------------|
| `default_window`        | 65 536  | seq-delta flow-control window        |
| `heartbeat_interval_ms` | 10      | sender → receiver heartbeat period   |
| `status_interval_ms`    | 10      | receiver → sender status period      |
| `reorder_buf_limit`     | 512     | receiver-side reorder buffer slots   |
| `send_ring_limit`       | 4 096   | sender-side retransmit cache slots   |
| `sender_bind_addr`      | 0.0.0.0:0 | sender source addr for UDP socket  |

## Cross-references

- `specs/2/10-dxs.md` — DXS streaming server (TCP) on top of CMP
- `specs/2/48-wal.md` — WAL flush rules, retention, rotation
- `specs/2/20-network.md` — Process topology, port assignments
- `specs/2/45-tiles.md` — Tile architecture (within-process IPC)
- `specs/2/18-messages.md` — Record-type catalogue
- `specs/2/22-perf-verification.md` — Bench gate, latency harness
