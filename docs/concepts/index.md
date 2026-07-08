# Concepts

RSX is a derivatives exchange and a study in fast distributed
code. The goal — sub-10 µs latency from client to matching
engine and back — is aspirational today. The in-process
round-trip floor is 7.8 µs at the median (p50) and 22.3 µs at
the 99th percentile (p99); cross-process it sits around 1.1 ms.
The benchmark tables in the root README show exactly where that
time goes.

That gap is the point. Each concept here is a load-bearing
design choice made to close it — or, in some cases, a
documented tradeoff that deliberately sacrifices one
dimension to keep another cheap.

Read these pages when you want to understand *why* the code
looks the way it does. For *what* the code does, read the
specs. For *how fast it goes*, read the benchmark tables.

## Pages

- [casting](01-casting.md) — One `repr(C)` layout is the on-disk
  WAL, the UDP frame, and the TCP replay stream (no serialization);
  plus a NAK reliability layer over UDP with no flow control, so a
  slow consumer never stalls the producer.
- [tiles-and-pinning](02-tiles-and-pinning.md) — Pinned busy-spin
  hot loops spend 1 core to remove scheduler wakeups, context
  switches, and core migration from 60-110 ns compute paths.
- [spsc-rings](03-spsc-rings.md) — rtrb SPSC rings pass data between
  sibling threads in 50-170 ns; ring-full means producer stall,
  not silent drop.
- [network-edge-io](04-network-edge-io.md) — Gateway and marketdata use
  monoio/io_uring because many-connection edges are syscall-bound;
  batching beats busy-spinning there.
- [slab-and-compression](05-slab-and-compression.md) — Pre-allocated
  slab arena plus a five-zone CompressionMap: a 100M-slot address space
  down to ~617k level slots (~40 MB) with 2–5 ns lookup; recentering
  keeps the touch exact, and the book is rebuilt by WAL replay.
- [fixed-point](06-fixed-point.md) — All prices and quantities are
  `i64` in smallest units. Float is wrong for an exchange.
- [asymmetric-durability](07-asymmetric-durability.md) — Fills are
  durable (WAL-flushed, ≤10 ms loss bound). Orders are ephemeral (lost
  on crash; client retries). Positions are derived (replay fills
  to rebuild). Not all data deserves the same treatment.
- [backpressure-not-drop](08-backpressure-not-drop.md) — When a
  bounded buffer fills, the producer stalls. Silent drops are
  invisible failures. Visible stalls are fixable.
- [sharding-axes](09-sharding-axes.md) — Two orthogonal scale-out
  axes: Risk shards by user_id, ME shards by symbol. Gateway
  is stateless. Adding users and adding symbols are independent.
- [trading-terminal](10-trading-terminal.md) — The rsx-tui terminal:
  protobuf-over-QUIC transport (binary, congestion-aware, broker-free)
  and keyboard-driven input (home-row hotkeys, not pointer targeting).
- [glossary](11-glossary.md) — RSX terms (casting, WAL, vshard,
  tile, BBO, mark vs index…), one line each + read-more links.

---

Deeper: [README.md](../../README.md),
[specs/index.md](../../specs/index.md)
