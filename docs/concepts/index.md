# Concepts

RSX is a portfolio-margin derivatives exchange. These pages explain the
design choices behind its transport, order book, risk path, and process
layout.

Read these pages for why the code has its current shape. Read the specs for
the design contracts, the architecture files for current implementation, and
dated `reports/` for measurements.

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
- [slab-and-compression](05-slab-and-compression.md) — A preallocated slab
  arena and five-zone compression map bound level storage; recentering keeps
  the touch exact, and WAL replay rebuilds the book.
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
- [glossary](11-glossary.md) — RSX terms (casting, WAL, vshard,
  tile, BBO, mark vs index…), one line each + read-more links.

---

Deeper: [README.md](../../README.md),
[specs/index.md](../../specs/index.md)
