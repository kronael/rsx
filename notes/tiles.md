# Tiles

The "tile" is the in-process IPC pattern used everywhere RSX needs
predictable microsecond-scale latency without a network multiplexer
in the inner loop. This is the concept reference: per-crate
ARCHITECTURE.md files link here instead of repeating the rationale.

## What a tile is

A tile is one OS thread pinned to a specific CPU core via
`core_affinity::set_for_current`, running a tight busy-spin loop.
It owns its state exclusively — no locks, no shared mutability,
no allocator pressure on the hot path. Communication with other
tiles is through SPSC ring queues (rtrb): one producer, one
consumer, lock-free, cache-line-aligned head/tail.

A process can have one tile (matching engine — the whole process
is one pinned loop) or several (risk engine — one hot tile plus a
tokio sidecar for blocking Postgres I/O, connected by seven SPSC
rings).

## Why tiles

- **No scheduler latency.** The thread is pinned and busy-spinning;
  the kernel does not preempt it because nothing blocks.
- **L1/L2-warm hot state.** Everything the tile touches sits on
  its core's caches.
- **Zero heap on the hot path.** Pre-allocated event buffers,
  slab arenas, fixed `Box<[T]>` slabs in `CastSender`.
- **Backpressure for free.** A full output ring stalls the
  producer; consumers cannot be DoS'd by an upstream burst.
- **Deterministic hand-off.** SPSC push/pop runs ~50–170 ns. No
  syscalls, no futex.

What you pay:

- **One core per tile.** A 4-tile process needs 4 cores it
  actually owns; otherwise the busy-spin starves the rest of the
  system.
- **No `await`, no `select!`.** Cross-tile coordination is
  explicit ring drains. The whole iteration is one straight
  function call.
- **A blocking syscall poisons the tile.** A 1 ms `read()` is a
  1 ms latency spike on every event behind it. Blocking I/O
  belongs on a sidecar (tokio thread), not in the tile.

## SPSC ring choice

`rtrb` for all intra-process IPC. Wait-free SPSC ring buffer,
~50–170 ns per push/pop on the host hardware. The producer never
blocks except on a full ring — at that point either the upstream
tile is too fast or the downstream consumer is stuck. RSX uses
`push_spin()` (bare busy-loop, no `spin_loop()` hint) because the
producing thread is itself pinned and dedicated.

Cache-line alignment is mandatory. All ring payloads are
`#[repr(C, align(64))]`; head/tail counters are on separate cache
lines to avoid false sharing.

## Where RSX uses tiles

- **Matching engine** (`rsx-matching`) — degenerate tile: the
  whole process is one pinned loop. casting I/O, dedup, matching,
  WAL append, casting fan-out all happen on the same core.
- **Risk engine** (`rsx-risk`) — canonical full tile. One pinned
  hot thread plus a tokio persist sidecar over a `PersistEvent`
  ring. Seven SPSC rings carry fills, orders, mark prices, BBOs,
  responses.
- **Mark price aggregator** (`rsx-mark`) — pinned aggregation
  loop fed by per-source SPSC rings from tokio WS tasks.
- **Marketdata shadow book** (`rsx-marketdata`) — single-owner
  shadow-book state inside a monoio reactor. Not a strict tile
  because the dominant cost is WS fan-out; the runtime is async.

## When NOT to use a tile

- **Network I/O multiplexing in the inner loop.** Hundreds of WS
  clients, SSE fan-out, or accept loops belong on monoio. A tile
  cannot afford an `accept()` syscall per iteration.
- **Blocking I/O.** Postgres write-behind, file I/O on a
  sidecar — never on the pinned thread.
- **Dynamic scheduling.** Anything that wants priorities, work
  stealing, or fairness across many short tasks. Use tokio.
- **Cold paths.** A code path that runs once per minute does not
  justify a dedicated core.

## References

- [`rsx-risk/notes/spsc.md`](../rsx-risk/notes/spsc.md) — SPSC ring
  buffer (rtrb) latency hierarchy and the lock-free protocol
- [`rsx-book/notes/align.md`](../rsx-book/notes/align.md) — why
  `#[repr(C, align(64))]` on every hot-path struct
- [`rsx-book/notes/arena.md`](../rsx-book/notes/arena.md) — slab
  arena allocator pattern (zero-heap order storage)
- [`rsx-book/notes/hotcold.md`](../rsx-book/notes/hotcold.md) —
  hot/cold field separation inside cache-line-sized records
- [`rsx-risk/notes/uds.md`](../rsx-risk/notes/uds.md) — UDS vs
  shared memory tradeoffs for the sidecar boundary
- [`rsx-cli/notes/pq.md`](../rsx-cli/notes/pq.md) — priority queue
  pattern (related)
- [`specs/2/45-tiles.md`](../specs/2/45-tiles.md) — architecture
  spec with per-process status
- [`specs/2/43-testing-smrb.md`](../specs/2/43-testing-smrb.md) —
  SPSC ring testing
