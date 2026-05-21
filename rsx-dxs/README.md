# rsx-dxs

Log-backed reliable UDP transport (CMP) + TCP cold-path replay
(DXS) for fixed-format binary records.

**Wire bytes = disk bytes = stream bytes.** No serialization
step. NAK retransmits read from the WAL itself, so the
retransmit horizon equals **log retention**, not buffer size.

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

## Quick start (sender)

```rust
use rsx_dxs::{CmpSender, WalWriter};
use rsx_messages::FillRecord;  // or your own CmpRecord

let mut wal = WalWriter::new(stream_id, &wal_dir, None,
                             64 * 1024 * 1024,
                             10 * 60 * 1_000_000_000)?;
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

## When NOT to use this

- Multi-language consumers (use protobuf/FlatBuffers/JSON)
- Public internet (use QUIC; no TLS, no congestion control here)
- Schema that changes often (zero-copy = field-stable structs)
- Big-endian targets (compile-time enforced LE)

## See also

- `specs/2/4-cmp.md` — protocol spec, byte-exact
- `specs/2/48-wal.md` — WAL flush rules, retention, rotation
- `specs/2/10-dxs.md` — TCP replay protocol details
- `blog/cmp.md` — design narrative
- `rsx-messages/` — RSX exchange domain records on top of this
