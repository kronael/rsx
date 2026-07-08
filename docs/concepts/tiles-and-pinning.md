# Tiles and Pinning

A tile is a pinned OS thread that owns 1 CPU core and runs a
busy-spin hot loop. The tradeoff is blunt: spend a whole core to
remove scheduler jitter, wakeup latency, and cache migration from a
compute-bound loop.

## When to tile

Tile when the hot loop is compute-bound and its service time is
measured in tens or hundreds of nanoseconds. Use monoio/io_uring
when the loop is I/O-bound across many file descriptors.

`rsx-risk` is the canonical tile. One pinned busy-spin thread drains
7 SPSC rings and drives the margin state machine. Margin work is
pure computation: no I/O, no syscall, no allocation. The pre-trade
check measures about 110 ns p50.

`rsx-matching` is a separate process, not a tile; a tile is a thread
inside a process. It has the same scheduling rationale: 1 pinned
busy-spin thread owns the book, appends WAL records inline, and
matches a single fill in about 54-65 ns.

Gateway and marketdata are the counterexample. They are monoio
services because their bottleneck is many WebSocket sockets and
kernel crossings, not compute. Their I/O model belongs in
[network-edge-io](network-edge-io.md).

## What pinning removes

A context switch is hundreds of ns of register and TLB churn. A
runtime park/unpark wakeup is on the us scale. Core migration under
the OS load balancer evicts the L1/L2 cache that a hot loop spent
warming.

Tokio's work-stealing is the right default for many I/O tasks: idle
workers park, tasks can move, and cores are shared. Those features
are invisible taxes on a 60 ns match. A pinned busy-spinner never
parks and never migrates, so the p99 stays near the p50; book bench
tails show p99/p50 around 1.1-1.5x on the tight paths.

The price is 100% CPU while idle. If 4 busy-spin loops own 4 cores,
the OS has 0 spare cores unless deployment reserves one.

## Why an unpinned spinner is dangerous

An unpinned busy-spinner consumes 100% of whatever core CFS gives
it and can float across cores under load balancing. If it lands on
a core owned by a pinned hot-path process, it starves that process:
the victim stalls, UDP receive falls behind, and packets drop.

So the rule has 2 parts. Pin every busy-spin hot loop and document
which core it owns. Keep off-path services such as mark and recorder
on sleep loops; they are not on the order critical path and do not
need busy-spin latency. An unpinned sleeper uses about 0% CPU
between events; an unpinned spinner eats 100%.

The 6-core README layout formalizes this: core 0 carries the OS and
off-path sleepers; cores 1-4 carry gateway, risk, matching, and
marketdata; core 5 is headroom.

## What you give up

Each tile owns a core whether it is busy or idle. A blocking
operation inside the hot loop turns directly into tail latency: a
1 ms syscall is a 1 ms stall for every message that arrives behind
it. The tile model requires no blocking I/O, no synchronous network
calls, and no mutex held by another thread.

That is why Risk sends Postgres writes to a tokio sidecar through
an SPSC ring. The pinned loop does computation; the sidecar pays the
blocking database cost.

---

Deeper: [blog/18-100ns-matching.md](../../blog/18-100ns-matching.md),
[reports/20260704_book-bench.md](../../reports/20260704_book-bench.md),
[specs/2/45-tiles.md](../../specs/2/45-tiles.md),
[docs/concepts/spsc-rings.md](../../docs/concepts/spsc-rings.md),
[docs/concepts/network-edge-io.md](../../docs/concepts/network-edge-io.md)
