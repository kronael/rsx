# rsx-dxs

Log-backed reliable UDP transport (CMP) + TCP cold-path replay
(DXS) for fixed-format binary records.

**Wire bytes = disk bytes = stream bytes.** No serialization
step. NAK retransmits read from the WAL itself, so the
retransmit horizon equals **log retention**, not buffer size.

## How fast

| Operation | p50 |
|---|---:|
| `WalWriter::append` (in-memory) | **31 ns** |
| `CmpSender::send` body | **3.87 µs** (99% `sendto` syscall) |
| `CmpSender → CmpReceiver` one-way (loopback) | **3.95 µs** |
| Round-trip (sender → echo → sender) | **10.3 µs** |
| `WalWriter::flush + fsync` (durability) | **651 µs** (amortised over 10 ms batch) |
| Cold-tier NAK retransmit (`read_record_at_seq`) | **23.5 ms @ 10 K records** |

Source: `cargo bench --workspace`. Detailed attribution in
[../docs/benches.md](../docs/benches.md) (project-level) and
[ARCHITECTURE.md](ARCHITECTURE.md) (this crate).

## Why this exists

Inside an exchange, every microsecond of encoding is a
microsecond off the matching engine. Existing options put a
length-prefixed framing layer on top of records that are
*already* framed (gRPC over HTTP/2, QUIC frames, Aeron sessions).
`rsx-dxs` skips it: the same 16-byte header serves disk format,
wire format, and stream format.

## What it gives you

- **CMP/UDP** — point-to-point reliable UDP. Aeron-inspired:
  receiver sends `StatusMessage` (every 10 ms) advertising a
  flow-control window; sender sends `CmpHeartbeat` (every
  10 ms) so idle gaps are detectable; receiver sends `Nak` on
  gap detection; sender retransmits.
- **Two-tier retransmit.** First the in-memory `send_ring`
  (~4 K records, ~µs to re-send). On miss, fall back to
  `wal::read_record_at_seq` — a random-access read from the WAL
  file that holds that seq. This is the load-bearing claim:
  retransmit is bounded by **WAL retention**, not RAM.
- **DXS/TCP** — same record bytes, reliable transport.
  Used for cold-start replay (`DxsConsumer::run`) and
  archival/replication. Optional rustls TLS.
- **Domain-agnostic.** `rsx-dxs` knows nothing about
  fills/orders/marks. It moves bytes that implement
  [`CmpRecord`]. The exchange's domain records live in
  `rsx-messages`.

## Wire format

Every CMP datagram and every WAL record is:

```
+------------------+-------------------------------+
| WalHeader (16B)  | payload (<= 65535B, repr(C))  |
+------------------+-------------------------------+
  record_type: u16
  len:         u16
  crc32c:      u32   (Castagnoli, payload only)
  version:     u8    (wire-format version; legacy=0, current=1)
  reserved:    7B    (zero on receive)
```

A single `version` byte lives in the previously-reserved
space. Adding a new record type does NOT bump the version
(record types are an open set); the version is reserved for
format-breaking changes that a v1 receiver could not safely
parse. Receivers reject unknown versions.

Payloads are `#[repr(C, align(64))]`. Sequence number is the
first `u64` of every data record (per the [`CmpRecord`] trait).

The hot send path (`CmpSender::send` / `send_raw`) does
**zero heap allocations** — the in-memory `send_ring` is
preallocated at construction and reused for every frame.
The receive path (`CmpReceiver::try_recv`) currently
allocates one `Vec<u8>` per in-order packet; a zero-copy
variant (caller-supplied `&mut [u8]`) is future work.

## Install

```toml
[dependencies]
rsx-dxs = { git = "https://github.com/kronael/rsx" }
```

Or as a workspace member if you've vendored the repo:

```toml
[dependencies]
rsx-dxs = { path = "path/to/rsx-dxs" }
```

A standalone working example lives in
[examples/cmp_smoke.rs](examples/cmp_smoke.rs). Run it:

```bash
cargo run --example cmp_smoke
```

## Quick start (sender)

```rust
use rsx_dxs::{CmpSender, WalWriter};
use rsx_messages::FillRecord;  // or your own CmpRecord

let mut wal = WalWriter::new(stream_id, &wal_dir, None,
                             64 * 1024 * 1024,
                             48 * 60 * 60 * 1_000_000_000)?;
let mut sender = CmpSender::new(dest_addr, stream_id, &wal_dir)?;

let mut fill = FillRecord { /* ... */ };
sender.send(&mut fill)?;        // assigns seq, sends, caches
sender.tick()?;                  // periodic heartbeat
sender.recv_control();           // process status/nak from peer
```

## Quick start (receiver)

```rust
use rsx_dxs::CmpReceiver;

let mut rx = CmpReceiver::new(bind_addr, sender_addr, stream_id)?;
loop {
    rx.tick();                   // periodic status to sender
    while let Some((hdr, payload)) = rx.try_recv() {
        // dispatch by hdr.record_type — transport doesn't care
    }
}
```

## Guarantees

- **Delivery**: every record is delivered in sequence to every receiver
  that stays connected, or the sender is notified it can't (gap timeout).
- **Order**: strict sequence number monotonicity; receiver blocks until
  gaps are filled via NAK retransmit.
- **Durability** (with DXS/TCP path): records are fsync'd to WAL on the
  sender before any downstream consumer can consider them committed.
  Default batch flush: every 10 ms; configurable.
- **Retransmit horizon**: bounded by WAL retention (default 48 h), not
  by the sender's RAM. Cold retransmits read directly from log files.
- **No phantom acks**: the send path does not acknowledge a record to the
  application until it is in the WAL and on the wire ring.

## Requirements and assumptions

**These are non-negotiable; violate them and all bets are off.**

- **Trusted LAN only.** No authentication, no encryption on the CMP/UDP
  path. Peers are assumed to be on a firewalled internal network (VPC,
  namespace, or dedicated L2 segment). For public internet, use QUIC.
- **Stable network.** CMP is tuned for a loss rate ≤ 0.01% and jitter
  ≤ 100 µs. On a WAN with real loss, retransmit storms will dominate
  and throughput collapses. Use KCP or QUIC for lossy paths.
- **Fixed-size, stable `repr(C)` payloads.** Wire format = disk format.
  Fields cannot be added without bumping `WalHeader.version`. If your
  schema changes often, use a self-describing format.
- **Little-endian host.** Compile-time assertion; will not build on BE.
- **Point-to-point (v1).** One sender → one receiver per stream.
  Multicast fan-out is v2 (see `specs/2/51-cmp-v2-multicast.md`).

## When NOT to use this

- Multi-language consumers (use protobuf/FlatBuffers/JSON)
- Public internet (use QUIC; no TLS, no congestion control here)
- Schema that changes often (zero-copy = field-stable structs)
- Big-endian targets (compile-time enforced LE)
- One-to-many fan-out today (v2 multicast is planned, not shipped)

## See also

- [ARCHITECTURE.md](ARCHITECTURE.md) — this crate's internal design
- [`../specs/2/4-cmp.md`](../specs/2/4-cmp.md) — protocol spec, byte-exact
- [`../specs/2/48-wal.md`](../specs/2/48-wal.md) — WAL flush rules, retention, rotation
- [`../specs/2/10-dxs.md`](../specs/2/10-dxs.md) — TCP replay protocol details
- [`../specs/2/35-testing-cmp.md`](../specs/2/35-testing-cmp.md) + [`../specs/2/36-testing-dxs.md`](../specs/2/36-testing-dxs.md) — test specs
- [`../rsx-messages/`](../rsx-messages/) — RSX exchange domain records built on top
- [`../facts/syscall-latency.md`](../facts/syscall-latency.md) — why the `sendto` floor is what it is

## License

MIT OR Apache-2.0 at your option.
