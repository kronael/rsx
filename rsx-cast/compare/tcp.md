# TCP

Stream baseline. TCP provides reliable, ordered, connection-oriented
delivery over IP. RFC 9293 is the consolidated current spec.
The benchmark answers two questions: how fast is a kernel TCP
loopback round-trip, and why does rsx-cast use TCP for the cold
WAL-replay path but not for live order flow.

## Wire model

TCP is a byte stream. There is no message framing in the wire
protocol — the application must impose its own. A single
`write()` of 64 bytes can arrive as one `read()` of 64 bytes,
two `read()`s of 32 bytes each, or a `read()` of 64 bytes
followed by extra bytes from the next message. Receivers must
loop until the expected count is satisfied.

rsx-cast's TCP cold path (DXS replay) reuses the exact same
16-byte `WalHeader` for framing that the UDP hot path uses.
Wire = disk = stream — the header tells the receiver how many
bytes the next record is.

## Protocol

OS kernel TCP stack. Three-way handshake (SYN, SYN-ACK, ACK)
to establish — costs 1 RTT before the first payload byte.
Reliable delivery via cumulative ACK + selective ACK (SACK,
RFC 2018). Retransmit horizon is the in-flight window only —
nothing on disk. Congestion control is CUBIC by default on
Linux (`/proc/sys/net/ipv4/tcp_congestion_control`); BBR
selectable per-socket via `TCP_CONGESTION` socket option.
Nagle's algorithm coalesces small writes (RFC 896) — disabled
by `TCP_NODELAY`. Without nodelay, the bench measures Nagle's
40 ms delayed-ACK interaction, not TCP itself.

## Guarantees

| Dimension | TCP | rsx-cast casting |
|---|---|---|
| Delivery | Reliable (ACK + retransmit) | Reliable (NAK + WAL retransmit) |
| Ordering | In-order byte stream | Per-stream FIFO (seq monotonic) |
| Framing | None — app-layer required | 16-byte `WalHeader` + fixed payload |
| Loss detection | Cumulative ACK + SACK (sender side) | Seq gap → NAK (receiver side) |
| Retransmit source | In-flight window (RAM only) | Hot ring + cold WAL (4 h) |
| Connection setup | 3-way handshake (1 RTT) | None — `sendto`, zero setup |
| Congestion control | CUBIC/BBR (kernel) | None (trusted LAN) |
| Head-of-line blocking | Yes (at API; lost segment freezes later in-flight bytes until retransmit) | Yes (at API; reorder_buf delays delivery until gap fills) |
| Gap-detection latency on busy stream | 3 dup-ACKs → fast retransmit | 1 packet arrival → immediate NAK |
| Gap-detection latency on idle stream | RTO timer (Linux min ~200 ms) | next heartbeat → immediate NAK |
| Durability | None | WAL on disk |

## Why casting uses TCP for cold path but not hot

**Cold path (DXS replay over TCP).** A consumer that has
fallen behind beyond casting's hot ring asks the recorder for
historical records by seq range. This is:

- Bulk sequential — throughput matters, per-message latency
  does not.
- Long-lived — one connection per consumer, the handshake
  cost is amortised across millions of records.
- Already behind — head-of-line blocking is fine; the
  consumer is catching up, ordering is what it wants.
- Reliable end-to-end without the receiver needing to
  implement NAK tracking against historical WAL.

TCP is the right answer here. The kernel does the work the
application would otherwise duplicate.

**Hot path (casting/UDP).** Live order flow has different
constraints:

- Latency-sensitive: <50 µs GW→ME→GW budget. The 3-way
  handshake alone burns a budget on every reconnect.
- Multiple independent streams (one per symbol, multiple
  consumers). TCP forces them through one byte stream and
  head-of-line-blocks across symbols (casting gives one socket
  per symbol stream, so gaps in symbol A don't block B).
- Faster gap recovery on idle streams: casting's heartbeat
  detects gaps without waiting for new data; TCP needs
  RTO_min (~200 ms on Linux). On busy streams both recover
  in ~1 RTT, but casting fires NAK on the first out-of-order
  arrival while TCP needs 3 duplicate ACKs.
- Loss is rare on a trusted 10 GbE fabric. NAK from the
  receiver costs zero on the no-loss path; ACK from the
  receiver costs a packet per message regardless.
- Congestion control has nothing to do — the fabric has
  fixed capacity, not a competing flow problem.

**On head-of-line blocking, honestly.** A casting receiver
holds out-of-order packets in `reorder_buf` (default 512
slots) and returns `None` from `try_recv()` until the gap
is filled — same end-result as TCP at the API. The wins
above are about *how fast* the gap is filled, not about
delivering out-of-order data. Within one casting stream FIFO
is the contract (specs/2/6-consistency.md §"Across
consumers"). The single-stream-per-symbol design is what
lets order flow on symbol A keep moving when symbol B
drops a packet.

Measured penalty depends on the I/O model. Kernel-blocking
TCP with `TCP_NODELAY` + spin (`compare_all::tcp_nodelay_128b`)
costs ~14 µs RTT — within ~1.5× of casting's ~9.3 µs. Async TCP
through a reactor (the iggy project at apache/iggy#606) measures
~100–1 000 µs — ~10–100× casting. The async path is what most
production order-flow servers actually pay; the spin-loop path
is the kernel floor with no reactor overhead.

The TCP-vs-casting gap is not principally about TCP being
"slow" — it is about head-of-line blocking, the one-stream
funnel, and the reactor cost of all-but-spin async TCP
clients. casting keeps each symbol in its own UDP flow with no
shared queue between them.

## Benchmark

`benches/compare_all.rs::tcp_nodelay_128b` (run with `cargo bench
-p rsx-cast --bench compare_all`) — Criterion, loopback, 128-byte
payload, std `TcpListener` / `TcpStream` with `TCP_NODELAY` on
both ends, under the shared `EchoClient` harness alongside
raw_udp / kcp / quinn. The standalone `compare_tcp.rs` was folded
into `compare_all.rs` in commit 836cfb1.

The connection is established once in setup, before the
timed loop. The 3-way handshake is not measured — this is
the best-case TCP latency once a session is open. This
matches the QUIC bench convention (one connection, many
iterations) and is what real long-lived consumers like
DXS replay see in production.

The receiver reads exactly 128 bytes per iteration in a
`read_exact`-style loop. Partial recvs are real on TCP and
the bench must drain them correctly; a naive single `read()`
would race and break the round-trip count under load.

| Transport | Loopback p50 |
|---|---|
| Raw UDP | ~9.9 µs (measured, 2026-07-01) |
| rsx-cast casting | ~9.3 µs (measured, 2026-07-01) |
| TCP nodelay (this bench) | ~14 µs (measured, 2026-05-24) |
| Tokio async TCP (reactor) | ~100–1 000 µs (published, iggy#606) |
| Quinn QUIC | ~37 µs measured; ~200–2 000 µs published |

The std-blocking + spin variant is much faster than the
tokio-async TCP path because there's no reactor wake-up
between syscalls. Both are useful: this bench shows the
kernel TCP floor, the tokio one shows what an async
application actually pays.

## Sources

- RFC 9293 — TCP, current consolidated spec
- RFC 2018 — TCP Selective Acknowledgment (SACK)
- RFC 896 — Nagle's algorithm
- Cardwell et al., "BBR: Congestion-Based Congestion Control",
  ACM Queue 2016 — BBR design and CUBIC comparison
- apache/iggy#606 — TCP vs UDP localhost RTT benchmark
- `tcp(7)` Linux man page — `TCP_NODELAY`, `TCP_CONGESTION`
