# Aeron

Open-source Java/C++ reliable UDP transport by Real Logic
(Martin Thompson, Todd Montgomery). The direct design ancestor
of rsx-cast casting. Widely deployed in HFT and trading systems;
acquired by Adaptive Financial Consulting in 2022.

- Repo: https://github.com/aeron-io/aeron (Apache-2.0)
- Wire spec: https://github.com/real-logic/aeron/wiki/Transport-Protocol-Specification
- Rust bindings (used by our bench): https://github.com/gsrxyz/rusteron

## Wire format

Aeron's frame layout is term-buffer-oriented. Each stream has
three rotating 64 MB **term buffers** (configurable via
`term-length`). A position in the stream is
`term_id × term_length + term_offset`. Loss detection, flow
control, and replay all key off of position rather than a
flat sequence number — the term-rotation abstraction lets the
receiver reclaim memory aggressively while keeping a window of
replay-able data.

**Data frame header (32 bytes):**

```
 0-3    frame_length      (header + payload, little-endian)
 4      version
 5      flags             (FRAGMENT_BEGIN, END, EOS)
 6-7    type              (DATA=0x01, PAD=0x02, NAK=0x03, SM=0x04, …)
 8-11   term_offset       (byte offset within the term buffer)
12-15   session_id
16-19   stream_id
20-23   term_id
24-31   reserved_value (8 bytes)
32+     payload
```

Encoding: little-endian throughout.

**casting difference.** Our `WalHeader` is 16 bytes (the on-disk
WAL record header doubles as the wire header) with a flat
`u64 seq`, a `u16 record_type`, a `u16 len`, and a CRC32C. No
`term_id` / `term_offset` / `session_id`. Trade-off:

- Aeron's term layout makes replay zero-copy from RAM in
  large strides — but the retransmit horizon is whatever fits
  in the term buffers (default ~192 MB / stream).
- casting's flat seq + WAL file layout makes the disk file the
  retransmit horizon (4 h retention by default). Slower per
  retransmit (random-access disk read), but the horizon is
  measured in hours of traffic rather than megabytes.

## Loss detection: NAK from receiver

Aeron is NAK-based. The receiver tracks the highest contiguous
position and detects a gap when an out-of-order frame arrives
at a higher `(term_id, term_offset)`. It sends a NAK back to
the sender naming the missing range.

- **Unicast**: NAK sent immediately. Single receiver, no
  implosion risk.
- **Multicast**: NAK sent after a randomized backoff so that
  only one receiver per gap actually sends the NAK
  ("NAK suppression"). Prevents NAK implosion.
- The sender retransmits from its in-memory term buffer.

The model is identical in casting: receiver detects a gap on
`seq`, sends `Nak{from_seq, count}` back to the sender, sender
retransmits. The retransmit path is ~1 RTT. casting is unicast
only — no NAK suppression backoff because there can be only
one receiver per casting stream.

## Retransmit horizon

This is where the two protocols diverge most.

**Aeron**: retransmit comes from the **term buffer** the
sender still has in memory. Once a position has been
overwritten by the rotating term buffers, the live retransmit
path can't recover it. For durable replay, Aeron Archive (a
separate component) records streams to disk; an archive
replay subscription pulls from the archive instead of the
live publication. This is a clean separation — the live wire
protocol is RAM-bounded and fast; the archive is a different
service with its own SUBSCRIBE / REPLAY protocol.

**casting/DXS**: retransmit is two-tier in the same component.

1. **Hot tier**: a 4096-slot pre-allocated send ring inside
   `CastSender`. NAKs for recent sequences are served from
   this ring with zero allocation, zero disk I/O.
2. **Cold tier**: a NAK whose `from_seq` predates the hot
   ring falls through to `read_record_at_seq` on the WAL
   file. Retransmit horizon = WAL retention (default 4 h).

The WAL is the source of truth for the application *and* the
retransmit reservoir for the transport. There is no archiver
sidecar — the producer process owns its own durability.

## Flow control

Aeron has multiple flow-control strategies (`max`, `min`,
`tagged`) configurable per publication. The sender's position
is capped at `min(receiver positions) + window` where window
defaults to half the term length. Receivers send
`StatusMessage` frames advertising their consumption position;
the sender uses these to compute the send-side window.

casting has **no wire-level flow control** — the receiver sends no
window-advertisement frame, and the only control record on the
wire is a NAK (on a detected gap) plus an idle-only heartbeat.
An earlier `StatusMessage`/receiver-window mechanism was removed
in commit 87b223e; casting is single-receiver-per-stream and
handles overrun one layer up instead: the WAL writer stalls the
producer when flush lag exceeds 10 ms or its buffer fills, and
the receiver's bounded reorder buffer caps how far ahead the
sender can run. This is a real narrowing vs Aeron — casting cannot
throttle a fast sender to a slow receiver mid-stream on the wire;
it relies on the WAL-writer backpressure and the trusted-LAN
capacity assumption (spec §10.4).

## Durability: integrated vs sidecar

| | Aeron | casting/DXS |
|---|---|---|
| Durability | Aeron Archive (separate process / API) | WAL embedded in CastSender |
| Sender startup | Connect to driver, no disk | Open WAL file (mmap'd) |
| Crash recovery | Replay from archive | Replay WAL from last tip |
| Wire = disk? | No (term buffer vs archive recording format) | Yes (WalHeader + payload, identical bytes) |

casting collapses the archive into the protocol. The WAL file you
write to disk *is* the wire format — `dd if=/path/to/wal | nc`
would be a syntactically valid replay stream. This is the
"wire = disk = stream" claim that motivates the design.

## Media driver

Aeron is structured around a separate **media driver** process
that owns all UDP sockets and shared-memory term buffers.
Applications communicate with the driver via lock-free SPSC
rings in shared memory (`/dev/shm/aeron-$USER/cnc.dat`). This
gives:

- Multiple clients sharing one driver → one set of sockets,
  one set of term buffers, lower per-client overhead.
- A driver crash doesn't take out client state immediately
  (client conductors detect via keep-alive timeout).
- An extra IPC hop on every send and every receive
  (app → driver shm → kernel UDP → driver shm → app).

**casting has no driver**. `CastSender::send()` calls `sendto()`
directly from the application thread. Cost: no IPC hop, but
each process owns its own UDP socket and WAL file. Suits
RSX's tile architecture (one process per role, pinned
thread, dedicated socket).

## Performance

### Published Aeron numbers (Real Logic / AWS, 2025)

Source: https://aws.amazon.com/blogs/industries/aeron-on-aws-2025-performance-benchmark-results/

Hardware: AWS c6in.16xlarge (Ice Lake, ENA networking, kernel
bypass disabled in the "Open Source" column).

| Load | Percentile | Open Source | Premium (kernel bypass) |
|---|---|---|---|
| 100 k msg/s | P50 | 21–22 µs | 24–25 µs |
| 100 k msg/s | P99 | 32–43 µs | 29–30 µs |
| 1 M msg/s | P50 | 30–35 µs | 30–31 µs |
| 1 M msg/s | P99 | 57–84 µs | 39–40 µs |

These are *bare-metal-class* numbers: dedicated cores per
driver agent, large c6in instance, isolated load generators,
RTT computed from the embedded message timestamp (in-handler).

### Our local bench

`rsx-cast/benches/compare_aeron.rs` — loopback ping/pong over
Aeron UDP, 64-byte payload, embedded media driver in the same
process, no core pinning.

| Setup | P50 RTT | P99 RTT (approx) | Note |
|---|---:|---:|---|
| Aeron UDP loopback (this bench, 6-core box, no pinning) | ~305 µs | ~570 µs | criterion total closure time |
| Aeron IPC (shared memory, this bench) | ~830 ns | ~1 µs | non-default; see source for caveat |
| casting RTT (`cast_rtt_bench`, same box) | ~9.3 µs | n/a | two CastSender/Receiver pairs, loopback (re-run 2026-07-01) |
| casting send body (`cast_send_breakdown_bench`) | 3.87 µs | n/a | sendto-side only |
| Aeron AWS open source (c6in.16xlarge) | 21–22 µs | 32–43 µs | published, pinned cores |

**Why our Aeron UDP number is 10–30× worse than the
published one**: on a 6-core machine without core pinning,
the driver agent thread + PONG echo thread + PING ping thread
+ criterion measurement thread + OS background tasks
oversubscribe the scheduler. The driver's idle strategy
spins, but every preemption costs us hundreds of microseconds
of round-trip. The IPC variant doesn't suffer because the
kernel UDP path drops out of the critical section. This is
not Aeron's protocol overhead — it is our environment.

**Why casting RTT is lower in our setup** even though Aeron is
generally faster in production: casting has no driver IPC hop.
On loopback with all threads in one process, `sendto()`
direct from the application is faster than going
`app → driver SHM → sendto → driver SHM → app` by exactly
the SHM-ring + scheduler-wakeup cost.

In a properly resourced deployment (≥8 cores, pinned, real
NIC, sustained load), Aeron's published numbers reflect what
the protocol actually does. Treat our 305 µs as a
"laptop-class" data point, not a competitive benchmark.

## Guarantees comparison

| Property | Aeron | casting/DXS |
|---|---|---|
| Reliable delivery | Yes | Yes |
| Loss detection | NAK (receiver) | NAK (receiver) |
| Retransmit source | term buffers (RAM, ~192 MB default) | hot ring (4096 slots, RAM) + cold WAL (disk) |
| Retransmit horizon | term-buffer lifetime (seconds) | WAL retention (4 h default) |
| Durability | Aeron Archive (separate process) | WAL embedded in sender |
| Wire = disk | No | Yes |
| FIFO per stream | Yes | Yes |
| Multi-receiver | Yes (UDP multicast, multi-destination cast) | No (unicast only) |
| Flow control | Configurable (`max` / `min` / `tagged`) | None on the wire; WAL-writer backpressure + bounded reorder buffer |
| Congestion control | Optional (CUBIC) | None |
| Frame header | 32 bytes | 16 bytes (`WalHeader`) |
| IPC topology | Driver process + clients via SHM rings | None (sendto direct from app thread) |
| Session setup | SETUP / handshake | None (sendto, zero setup) |
| Trust model | Configurable (TLS in v1.45+, raw UDP otherwise) | Trusted LAN only (no auth, no encryption) |
| Language (impl) | C++ media driver, Java client, C client | Rust (native) |
| Production deployments | Decades in HFT (LMAX, citadel, exchanges) | RSX exchange (this repo) |

## Where Aeron is genuinely more capable

casting simplified Aeron for one trust assumption (LAN), one
topology (unicast), and one language (Rust). Honest
side-by-side:

- **Multicast / multi-destination cast.** Aeron does fan-out
  at the wire level; casting fans out at the sender (one socket
  per receiver). For 100 receivers, Aeron multicast sends
  one packet; casting unicast sends 100.
- **Maturity.** Aeron is decades old, with production
  deployments at the largest exchanges. casting is new.
- **Wire-level flow control.** Aeron lets you pick
  `min`/`max`/`tagged` per publication; casting has none on the
  wire — a fast sender can outrun a slow receiver until the
  reorder buffer or WAL-writer backpressure intervenes.
- **TLS option.** Aeron 1.45+ supports DTLS; casting delegates
  TLS to the layers around it (L3 firewall, gateway).
- **Position abstraction.** Aeron's term-based position
  model is more flexible for replay/seek operations against
  large in-memory windows; casting's flat seq + WAL is simpler
  but doesn't support sub-record seeking.

## Where casting is intentionally narrower

- **No archiver.** The producer process is its own archive.
  One fewer service to deploy, monitor, recover.
- **No driver.** No IPC hop, no shared-memory ring between
  app and transport. Suits a single-process-per-role tile
  architecture.
- **Rust-native.** No JVM, no GC pauses, no JNI / SBE
  layer.
- **One trust model.** Trusted LAN. The system-spec
  (specs/2/4-cast.md §10.4) delegates auth to the gateway
  (JWT) and the L3 network (firewall, VPC). casting is
  intentionally unauthenticated.

## Running the bench

```bash
cargo bench -p rsx-cast --bench compare_aeron
```

Prerequisites (Debian/Ubuntu):

```bash
sudo apt install -y cmake libclang-dev clang uuid-dev libbsd-dev
```

The `rusteron-client` / `rusteron-media-driver` crates are
configured with `features = ["precompile", "static"]` in
`Cargo.toml`. This downloads a precompiled Aeron C driver
binary from the rusteron release artifacts on first build
(no cmake-of-Aeron required at compile time, though Debian
12's stock cmake 3.25 wouldn't satisfy Aeron's `>=3.30`
requirement anyway). System libs `libuuid` and `libbsd` are
needed at link time.

The bench source documents an IPC variant
(`bench_aeron_ipc`) that is intentionally not in the default
criterion group — running both UDP and IPC variants in one
process triggers a C-side
`MediaDriver has been shutdown` race during driver
teardown/relaunch. Smoke-measured separately, Aeron IPC RTT
is **~830 ns** on this hardware.

## Sources

- Aeron repository: https://github.com/aeron-io/aeron
- Transport spec: https://github.com/real-logic/aeron/wiki/Transport-Protocol-Specification
- "Aeron: Open-source high performance messaging" — Martin Thompson, Strange Loop 2014: https://www.youtube.com/watch?v=tM4YskS94b0
- LMAX Disruptor paper (the design lineage that produced Aeron): https://lmax-exchange.github.io/disruptor/disruptor.html
- AWS 2025 benchmark: https://aws.amazon.com/blogs/industries/aeron-on-aws-2025-performance-benchmark-results/
- Real Logic blog (general Aeron coverage): https://www.real-logic.co.uk/
- rusteron (Rust bindings used by our bench): https://github.com/gsrxyz/rusteron
- Adaptive Financial Consulting acquisition (2022): https://weareadaptive.com/2022/09/06/adaptive-acquires-real-logic/
