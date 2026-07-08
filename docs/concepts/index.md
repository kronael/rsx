# Concepts

RSX is a derivatives exchange and a study in fast distributed
code. The goal — sub-10 µs latency from client to matching
engine and back — is aspirational today. The in-process
round-trip floor is 7.5 µs p50 / 16.9 µs p99; cross-process
it sits around 1.1 ms. The benchmark tables in the root
README show exactly where that time goes.

That gap is the point. Each concept here is a load-bearing
design choice made to close it — or, in some cases, a
documented tradeoff that deliberately sacrifices one
dimension to keep another cheap.

Read these pages when you want to understand *why* the code
looks the way it does. For *what* the code does, read the
specs. For *how fast it goes*, read the benchmark tables.

## Pages

- [wal-is-wire-is-stream](01-wal-is-wire-is-stream.md) — One
  `repr(C)` layout serves as the on-disk WAL, the UDP wire
  frame, and the TCP replay stream. No serialization step.
- [reliable-udp](02-reliable-udp.md) — casting: NAK-based gap
  recovery over UDP, idle-only heartbeats, deliberately no
  flow control. Slow consumers fall back to TCP replay.
- [tiles-and-pinning](03-tiles-and-pinning.md) — Pinned busy-spin
  hot loops spend 1 core to remove scheduler wakeups, context
  switches, and core migration from 60-110 ns compute paths.
- [spsc-rings](04-spsc-rings.md) — rtrb SPSC rings pass data between
  sibling threads in 50-170 ns; ring-full means producer stall,
  not silent drop.
- [network-edge-io](05-network-edge-io.md) — Gateway and marketdata use
  monoio/io_uring because many-connection edges are syscall-bound;
  batching beats busy-spinning there.
- [slab-and-compression](06-slab-and-compression.md) — Pre-allocated
  slab arena plus a five-zone CompressionMap: a 20M-level book
  in ~15 MB with 2–5 ns lookup and zero malloc on the hot path.
- [fixed-point](07-fixed-point.md) — All prices and quantities are
  `i64` in smallest units. Float is wrong for an exchange.
- [asymmetric-durability](08-asymmetric-durability.md) — Fills are
  durable (fsync before forwarding). Orders are ephemeral (lost
  on crash; client retries). Positions are derived (replay fills
  to rebuild). Not all data deserves the same treatment.
- [backpressure-not-drop](09-backpressure-not-drop.md) — When a
  bounded buffer fills, the producer stalls. Silent drops are
  invisible failures. Visible stalls are fixable.
- [sharding-axes](10-sharding-axes.md) — Two orthogonal scale-out
  axes: Risk shards by user_id, ME shards by symbol. Gateway
  is stateless. Adding users and adding symbols are independent.
- [tui-quic-protobuf](11-tui-quic-protobuf.md) — The terminal uses
  protobuf-over-QUIC so the client edge stays binary, compact,
  congestion-aware, and broker-free.
- [keyboard-driven-terminal](12-keyboard-driven-terminal.md) —
  A ratatui trading terminal turns order entry into home-row
  hotkeys instead of serial pointer targeting.
- [glossary](13-glossary.md) — RSX terms (casting, WAL, vshard,
  tile, BBO, mark vs index…), one line each + read-more links.

---

Deeper: [README.md](../../README.md),
[specs/index.md](../../specs/index.md)
