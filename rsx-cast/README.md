# rsx-dxs

Reliable UDP whose retransmit source IS the WAL the producer
writes for audit and replay. One byte layout for live frames,
disk records, and TCP replay streams — no serialization step
between them.

**CMP** (C Message Protocol) is the UDP path: NAK-based,
sender-side retransmit, fixed `#[repr(C)]` frames. **DXS** is
the TCP cold-path replay protocol over the same record bytes.
The NAK retransmit horizon equals **log retention** (default
48 h), not RAM. Trust model: trusted LAN only, single sender
per stream, no congestion control — public-internet use is
QUIC's job (see [When NOT to use this](#when-not-to-use-this)).

## How fast

| Operation | p50 | Bench / env |
|---|---:|---|
| `WalWriter::append` (in-memory) | **31 ns** | `wal_bench`, single thread |
| `CmpSender::send` body | **~4.07 µs** (99% kernel UDP send path) | `cmp_send_breakdown_bench`, 128 B payload + 16 B header |
| Raw UDP RTT (baseline) | **9.89 µs** | `compare_udp`, 128 B, two threads pinned to cores 2/3 |
| CMP RTT (sender → echo → sender) | **11.26 µs** | `cmp_rtt_bench`, 128 B, two threads pinned |
| `WalWriter::flush + fsync`, single record | **651 µs** | `wal_fsync_bench`, sync per append |
| `WalWriter::flush + fsync`, 64 KB batch | **24 µs** | `wal_fsync_bench`, amortised |
| Cold-tier NAK retransmit (`read_record_at_seq`) | **23.5 ms @ 10 K records** | `wal_random_read_bench`, scans backwards |

Host: AMD Ryzen 9 5950X (6-core slice), Linux 6.1, Rust release.
Authoritative dated measurements:
[`facts/cmp-vs-udp-overhead.md`](https://github.com/kronael/rsx/blob/master/facts/cmp-vs-udp-overhead.md).
Per-bench attribution: [BENCHES.md](BENCHES.md). Architectural
walk-through: [ARCHITECTURE.md](ARCHITECTURE.md).

**Footnotes.**
- **p99 not yet measured.** Aeron and Chronicle Queue publish
  p99 / p99.9; we publish p50 only. Run `cargo bench` yourself
  for your environment.
- **Loopback ≠ production.** The numbers above are spin-loop
  loopback microbenches with pinned threads. The exchange's
  cross-process p50 is ~1 128 µs — dominated by monoio's
  100 µs sleep-polls, tokio reactor schedules, and PG
  write-behind churn. Transport overhead is a ~10 µs slice of
  the cross-process cost; if you care about the full path,
  re-bench in your deployment.

## Why this exists

The bytes are never reformatted. The 16-byte header + payload
that the matching engine writes to its WAL is the same 16
bytes that the UDP packet carries, the same 16 bytes the
TCP replay stream carries, and the same 16 bytes the audit
archive holds. No serialize step, no encode step, no
length-prefix wrapper. Existing options put a framing layer
on top of records that are *already* framed (gRPC over
HTTP/2, QUIC frames, Aeron sessions, Chronicle wire); CMP
skips it.

The retransmit story falls out of this. When the receiver
NAKs a missing seq, the sender first checks the in-memory
ring (~4 K most recent frames) and on miss does an O(log N)
file-lookup + O(1) random read against the same WAL the
producer writes for audit. Retransmit horizon = WAL
retention, not RAM. This is "embedded WAL" vs. Aeron's
"separate Archive sidecar"; the protocol invention is small,
the packaging difference is the point.

## What it gives you

- **CMP/UDP** — point-to-point reliable UDP. Sender assigns
  monotonic `seq`, sends, and caches the encoded frame in a
  preallocated ring. Receiver detects gaps from sequence skips
  or from heartbeat-driven idle-tail checks; sends a `Nak`;
  sender retransmits from the ring or, on miss, from the WAL.
  An idle-only `CmpHeartbeat` (100 ms cadence; suppressed
  while data is flowing) covers gaps that would otherwise sit
  undetected at the tail of an idle stream. **No flow control,
  no congestion control** — receivers stall their consumer or
  drop reordered packets on overflow; senders never pause.
- **Two-tier retransmit (embedded, not sidecar).** First the
  in-memory `send_ring` (4 K most-recent frames, ~µs to
  re-send). On miss, fall back to
  `wal::read_record_at_seq` — a random-access read from the
  WAL file that holds that seq. Retransmit horizon = **WAL
  retention** (default 48 h), not RAM. Aeron's equivalent
  ships as a separate Archive process; CMP keeps the audit
  log and the retransmit cache in the same producer.
- **DXS/TCP** — same record bytes, reliable transport. Used
  for cold-start replay (`DxsConsumer::run`) and
  archival/replication. Optional rustls TLS.
- **Domain-agnostic.** `rsx-dxs` knows nothing about
  fills/orders/marks. It moves bytes that implement
  [`CmpRecord`]. The wider rsx exchange project layers its
  domain records (`FillRecord`, `BboRecord`, …) on top in a
  separate crate; nothing in this crate depends on them.

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

Internal-use crate; not published on crates.io. Use as a git
dependency and pin to a commit so future breaking changes
don't break your build:

```toml
[dependencies]
rsx-dxs = { git = "https://github.com/kronael/rsx", rev = "abc1234" }
```

A standalone working example lives in
[examples/cmp_smoke.rs](examples/cmp_smoke.rs). Run it:

```bash
cargo run --example cmp_smoke
```

## Quick start (sender)

```rust
use rsx_dxs::CmpSender;
use rsx_dxs::WalWriter;

// One WAL per stream. The CmpSender writes here itself; the
// outer process keeps a handle to read from it (for replay,
// for archival).
let mut wal = WalWriter::new(
    stream_id, &wal_dir,
    /* tip_persist_path */ None,
    /* max_file_size   */ 64 * 1024 * 1024,
    /* retention_ns    */ 48 * 60 * 60 * 1_000_000_000,
)?;
let mut sender = CmpSender::new(dest_addr, stream_id, &wal_dir)?;

let mut fill = my_crate::FillRecord { /* ... */ };
sender.send(&mut fill)?;   // assigns seq, writes to WAL, sends UDP, caches
wal.flush()?;              // call periodically (e.g. every 10 ms in a tick loop)
sender.tick()?;            // emits CmpHeartbeat if the stream has been idle
sender.recv_control();     // drains incoming NAKs and retransmits as needed
```

`tick()` and `recv_control()` are non-optional in steady state.
Heartbeats are how the **receiver** detects a stalled tail (no
data + no heartbeat ⇒ assume gap, NAK from last seen seq);
`recv_control()` is how the **sender** sees those NAKs and
retransmits. Both are cheap (≪ 100 ns when nothing's pending);
call them on whatever cadence your event loop has. The receiver
also uses `tick()` purely as a placeholder hook (currently a
no-op, but call-site stable for forward compat).

## Quick start (receiver)

```rust
use rsx_dxs::{CmpReceiver, CmpRecv};

let mut rx = CmpReceiver::new(bind_addr, sender_addr, stream_id)?;
loop {
    rx.tick();   // forward-compat hook; cheap, no-op today
    match rx.try_recv() {
        CmpRecv::Data(hdr, payload) => {
            // dispatch by hdr.record_type — transport doesn't care
        }
        CmpRecv::Empty => {}
        CmpRecv::Faulted { last_delivered_seq, .. } => {
            // gap too big for in-band recovery; see Pattern A below
            // for the canonical DXS-replay-then-reset response.
            break;
        }
    }
}
```

## Consumer patterns

Two canonical patterns. Pick one per stream; **don't mix
them for the same stream**.

### Pattern A — live-latency consumer (TCP bootstrap, UDP live)

For consumers that need µs-class latency on the live tail
(risk shards, marketdata, mark). Lifecycle:

```
                                              fault?
                                                ▲
 ┌──────────────┐    ┌──────────────┐    ┌─────┴────────┐
 │ TCP catch-up │───►│ UDP live     │───►│ TCP catch-up │──► UDP live ─►
 │ (DxsConsumer)│    │ (CmpReceiver)│    │ from new tip │
 └──────────────┘    └──────────────┘    └──────────────┘
   Phase 1, until        steady state         on FAULTED:
   CaughtUpRecord        — listen UDP         drain TCP to
                                              current, resume UDP
```

```rust
use rsx_dxs::{DxsConsumer, CmpReceiver, CmpRecv};

// 1. Bootstrap: drain historical via TCP from last-persisted tip.
let mut dxs = DxsConsumer::new(stream_id, dxs_addr.clone(),
                               tip_file.clone(), None)?;
dxs.run_once(|rec| { process(rec.header, rec.payload); true }).await?;
// TCP closes here. Steady state has zero TCP per consumer.

// 2. Live: listen UDP.
let mut rx = CmpReceiver::new(bind_addr, sender_addr, stream_id)?;
loop {
    rx.tick();
    match rx.try_recv() {
        CmpRecv::Data(hdr, payload) => process(hdr, payload),
        CmpRecv::Empty => continue,
        CmpRecv::Faulted { last_delivered_seq, .. } => {
            // 3. On FAULTED: reopen TCP from the persisted tip,
            //    drain to current, reset, resume UDP.
            let mut dxs = DxsConsumer::new(stream_id, dxs_addr.clone(),
                                           tip_file.clone(), None)?;
            dxs.run_once(|rec| { process(rec.header, rec.payload); true }).await?;
            rx.reset_after_replay(dxs.tip);
        }
    }
}
```

Steady state: zero TCP connection per consumer; producer
only pays UDP send cost. TCP cost is paid once on startup
and again only on the (rare) FAULTED escalation.

### Pattern B — TCP-only consumer

For consumers that don't need µs latency (archivers, replay
tools, analytics, cross-DC replication). DXS Phase 2 supports
a live tail over TCP, so a single `DxsConsumer` covers both
historical catch-up and live streaming, indefinitely.

```rust
use rsx_dxs::DxsConsumer;

let mut dxs = DxsConsumer::new(stream_id, dxs_addr, tip_file, None)?;
dxs.run(|record| {
    process(record.header, record.payload);
}).await?;
// Never returns under normal operation; reconnects with
// exponential backoff on TCP errors.
```

`rsx-recorder` ships as the canonical Pattern B consumer.
Trades latency (TCP head-of-line blocking, kernel cwnd) for
operational simplicity (one socket, no NAK state machine,
no UDP bind, no kernel rmem tuning).

### Choosing

| use case | pattern |
|---|---|
| Risk shard, matching consumer, marketdata fan-out | A |
| Anything inside the GW→ME→GW critical path | A |
| Archival recorder, replay-to-disk, ETL | B |
| Read-only analytics on historical data | B |
| Cross-DC replication (single TCP per peer) | B |

When in doubt: **B is simpler**. Switch to A only when tail-
latency measurements justify the extra moving parts.

## Guarantees

- **Order**: strict sequence-number monotonicity per stream.
  The receiver returns records in `seq` order; gaps block
  delivery until NAK retransmit fills them or the gap timeout
  expires.
- **Durability** (WAL): `WalWriter::flush` calls `fsync`. The
  producer's tick flushes every 10 ms by default; configurable.
  Records are on disk before any downstream consumer sees them
  via the TCP replay path. Bounded loss on crash: up to one
  flush interval (10 ms default) of pre-fsync records.
- **Retransmit horizon**: bounded by WAL retention (default
  48 h), not by RAM. Cold retransmits read directly from log
  files via `read_record_at_seq`.
- **Idempotent replay**: consumers dedup by `seq`. Records
  with `seq ≤ tips[stream_id]` are a no-op. Tips persist
  every 10 ms; recovery resumes from `tip + 1`.
- **At-least-once delivery over CMP/UDP**, deduplicated at
  the consumer via `seq` + tips. Replay over DXS/TCP is
  deterministic and resumes from `tip + 1`.

### Known caveats

- **Reorder-buffer overflow silently advances.** When the
  receiver's reorder buffer (default 512 entries) overflows
  while waiting for a gap to be NAK-filled, it currently
  clears the buffer and advances past the gap rather than
  surfacing a hard error to the consumer. The pending v4
  reliability spec replaces this with a bounded ring +
  explicit FAULTED state; until that lands the "Delivery"
  promise above has this hole.
- **FAULTED escalation is not implemented.** Specced (see
  `specs/4-cmp.md` §FAULTED) but the consumer side raises
  no signal today.

## Requirements and assumptions

**These are non-negotiable; violate them and all bets are off.**

- **Trusted LAN only.** No authentication, no encryption on
  the CMP/UDP path. Peers are assumed to be on a firewalled
  internal network (VPC, namespace, or dedicated L2 segment).
  Trust is delegated upward (to a gateway with JWT + TLS for
  external clients) and downward (to L3 — firewall, VPC,
  namespace — for internal peers). See `specs/4-cmp.md` §10.4.
  For public-internet transport, use QUIC.
- **Stable network.** CMP is tuned for loss rate ≤ 0.01% and
  jitter ≤ 100 µs. On a lossy WAN, retransmit storms will
  dominate; throughput collapses. Use KCP or QUIC there.
- **Fixed-size, stable `repr(C)` payloads.** Wire format =
  disk format. Fields cannot be added without bumping
  `WalHeader.version`. If your schema changes often, use a
  self-describing format (protobuf, FlatBuffers).
- **Little-endian host.** Compile-time assertion; will not
  build on BE.
- **Point-to-point, single sender per stream (v1).** One
  sender → one receiver per stream. Multicast fan-out is v2
  (specced, not shipped).
- **Slow consumers don't pace the sender.** There is no
  flow control. A slow receiver overflows its reorder buffer
  and silently drops; a slow consumer of the upstream
  receiver-side queue is the application's problem.

## When NOT to use this

- **Public internet** — no TLS on CMP, no congestion control.
  Use QUIC (Quinn) or HTTP/2.
- **Lossy or high-jitter paths** — NAK retransmit assumes
  ≤ 0.01% loss. WAN paths cause retransmit storms. Use KCP.
- **Schema that changes often** — wire = disk = repr(C)
  means changes are coordinated stop-redeploy events. Use
  protobuf / FlatBuffers / Cap'n Proto.
- **Multi-language consumers** — there is no IDL. Hand-write
  the repr(C) layout in each language, or use one of the
  above.
- **Big-endian targets** — compile-time enforced LE.
- **One-to-many fan-out today** — v2 multicast is planned,
  not shipped.
- **Cold-tier latency-sensitive replay** — `read_record_at_seq`
  is O(N) within a WAL segment file (23.5 ms @ 10 K records).
  Acceptable for cold-start replay or stale NAKs; not for
  realtime tail-of-stream recovery.
- **Per-packet zero-copy receive** — `CmpReceiver::try_recv`
  currently allocates one `Vec<u8>` per in-order packet. A
  caller-supplied `&mut [u8]` variant is future work.
- **No congestion control** — a sender that outpaces the
  link buries the receiver. The trust assumption is that
  capacity planning happens out-of-band.
- **Slow-consumer behavior is "drop silently"** — see the
  reorder-buffer caveat under Guarantees. If the consumer
  needs FAULTED + halt-and-rebuild semantics, wait for v4.

## MSRV

Edition 2021. No `rust-version` declared in `Cargo.toml`;
the crate builds against any current stable rustc. Internal
policy: MSRV follows the workspace's compiler, which tracks
stable closely.

## Tooling

- **WAL inspection.** A `wal dump` CLI for replaying /
  inspecting WAL files lives in the wider rsx exchange repo
  (the `rsx-cli` crate). It uses this crate as a dependency
  and reads any WAL written here.
- **Environment variables.** `CmpConfig::from_env` reads:
  - `RSX_CMP_REORDER_BUF_LIMIT` (default 512) — cap on
    out-of-order packets buffered while waiting for a NAK
    fill. Overflow drops the oldest gap and re-syncs.
  - `RSX_CMP_HEARTBEAT_INTERVAL_MS` (default 100) — sender
    heartbeat cadence; idle-stream only (data sends reset
    the timer).
  - `RSX_CMP_SENDER_BIND_ADDR` (unset by default) — pins the
    sender to a known port so receivers know where to send
    NAKs.
  - `RSX_REPL_TLS`, `RSX_REPL_CERT_PATH`, `RSX_REPL_KEY_PATH`
    — TLS on the DXS replay TCP socket.
- **Metrics.** No Prometheus. Counters (drops, NAK retransmits,
  reorder overflows) emit as structured `tracing` log lines;
  a separate shipper turns them into metrics out-of-band.

## Lineage

- **LBM** (29West / Informatica) — commercial NAK + UDP for
  market data; the ancestor of the exchange-grade family.
  https://www.informatica.com/products/data-integration/real-time-streaming.html
- **Aeron** (Real Logic) — the direct design ancestor. NAK-
  based reliable UDP, separate Archive process for cold
  retransmit. https://github.com/aeron-io/aeron
- **MoldUDP64** (Nasdaq) — the closest published peer in the
  HFT space; gap-fill via a separate retransmit server,
  sequence per session. https://www.nasdaq.com/docs/MoldUDP64.pdf
- **rtrb** (mgeier) — wait-free SPSC ring buffer that
  inspired our preallocated send-ring + the planned v4
  reorder-ring. https://github.com/mgeier/rtrb
- **`crc32fast`** — what CMP uses for header CRC. The
  Castagnoli polynomial choice follows iSCSI / SCTP.
  https://github.com/srijs/rust-crc32fast

## Alternatives

If CMP doesn't fit, the directly-relevant peers (with deeper
notes + reproducible benches in [`compare/`](compare/)):

- [**Aeron**](compare/aeron.md) — the design ancestor; UDP
  unicast/multicast + IPC + separate Archive for persistence.
  Java/C++; a Rust binding (`rusteron`) exists.
- [**MoldUDP64**](compare/moldudp64.md) — Nasdaq's public
  exchange protocol; multicast-only, separate gap-fill
  server.
- [**Quinn / QUIC**](compare/quinn.md) — modern reliable UDP
  with TLS + congestion control. Pure Rust. Use this for
  the public internet.
- [**Chronicle Queue**](compare/chronicle-queue.md) —
  persistent-log peer. Java; sub-µs IPC via mmap.
- [**KCP**](compare/kcp.md) — RUDP from the gaming world;
  loss-tolerant, ARQ-based, congestion control.
- [**LBM**](compare/lbm.md) — commercial (Informatica), no
  published bench, well-known in HFT.

[`compare/niche.md`](compare/niche.md) lists the long tail
(SoupBinTCP, BinaryNGB, Cap'n Proto RPC, Real-time Publish
Subscribe, ZeroMQ patterns, …).

## Breaking Changes

This crate has no public stable API yet. The wider rsx
exchange's
[CHANGELOG](https://github.com/kronael/rsx/blob/master/CHANGELOG.md)
covers cross-crate breaking changes; the current crate
version is **0.2.0** and the next bump (0.3) will land
together with the v4 reliability rewrite.

## See also

- [ARCHITECTURE.md](ARCHITECTURE.md) — this crate's internal
  design
- [BENCHES.md](BENCHES.md) — what each Criterion bench
  measures and how to run it
- [`specs/4-cmp.md`](specs/4-cmp.md) — protocol spec, byte-exact
- [`specs/48-wal.md`](specs/48-wal.md) — WAL flush rules,
  retention, rotation
- [`specs/10-dxs.md`](specs/10-dxs.md) — TCP replay protocol
  details
- [`facts/syscall-latency.md`](facts/syscall-latency.md) —
  why the `sendto` floor is what it is
- The wider [rsx exchange project](https://github.com/kronael/rsx)
  layers domain records (Fill, BBO, OrderInserted, …) on top
  in a separate crate — not bundled here.

## License

Internal-use crate within the wider rsx exchange project.
Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](https://www.apache.org/licenses/LICENSE-2.0))
- MIT license ([LICENSE-MIT](https://opensource.org/licenses/MIT))

at your option. Unless you explicitly state otherwise, any
contribution intentionally submitted for inclusion in
rsx-dxs by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or
conditions.
