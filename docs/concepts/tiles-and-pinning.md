# Tiles and Pinning

A tile is a pinned OS thread that owns 1 CPU core and runs a
busy-spin hot loop. The tradeoff is blunt: spend a whole core to
remove scheduler jitter, wakeup latency, and cache migration from a
compute-bound loop.

## When to tile

Tile when one pinned thread owns compute-bound state and its
service time is measured in tens or hundreds of ns. Do not tile an
I/O multiplexer just because it is hot; use monoio/io_uring when
the loop is waiting on many fds.

There are 3 categories.

First: a single pinned loop is a tile. `rsx-matching` is the
cleanest case because the process contains exactly 1 hot thread, so
the tile fills the process. There is no sibling thread and no SPSC
ring inside the matching process. The loop drains 1 casting/UDP
order, checks dedup, appends the WAL record inline, runs the book
match in about 54-65 ns, publishes WAL/cast events inline, and sends
the fill back over casting/UDP. It is one in-sync sequence, not N
concurrent tasks.

`rsx-risk` is also a tile, but the process has 2 threads: one
pinned tile-thread and one tokio-thread. The tile-thread owns the
risk state, drains casting/UDP, uses 3 SPSC rings today
(`PersistEvent` 8 192, `OrderResponse` 2 048, accepted
`OrderRequest` 2 048), and does the 110 ns pre-trade check. The
tokio-thread is plain write-behind: it runs a current-thread Tokio
runtime, drains the `PersistEvent` ring, and pays the blocking
Postgres cost. It is not a sidecar process and not a tile.

Second: a monoio reactor is not a tile and not a single loop in the
matching sense. Gateway and marketdata multiplex many WebSocket fds
concurrently on monoio/io_uring. Tile-NO: their bottleneck is I/O
multiplexing plus kernel crossings across N fds, not arithmetic in
cache. A pinned busy-spin thread would still make the same kernel
crossings and would not turn a 3.5 us send syscall into a 60 ns
match. Their model is [network-edge-io](network-edge-io.md).

Third: sleepers are off-path and unpinned. Mark and recorder sleep
between events and use about 0% CPU while idle. They are not on the
order critical path, so spending 1 core on a busy-spin loop buys
little.

The rule is: tile compute-bound hot state; use monoio for I/O-bound
fan-in/fan-out; leave sleepers unpinned. monoio is the right default
for I/O-bound work. If the target is both the lowest latency floor
and the highest event-rate ceiling, the last mile is a bespoke
io_uring loop tuned to the workload: no generic reactor, direct
SQE/CQE handling, SQPOLL, registered buffers, multishot recv, and
GSO. That is the I/O-side version of hand-rolling a tile; see
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
marketdata; core 5 is spare margin. Core 5 is not kernel headroom,
because core 0 already carries the OS and sleepers. It is burst
capacity, or a future SO_REUSEPORT shard, or a future SQPOLL kernel
thread. It is not required for correctness: a tight 5-core box can
drop it if it accepts less margin.

## What you give up

Each tile owns a core whether it is busy or idle. A blocking
operation inside the hot loop turns directly into tail latency: a
1 ms syscall is a 1 ms stall for every message that arrives behind
it. The tile model requires no blocking I/O, no synchronous network
calls, and no mutex held by another thread.

That is why Risk sends Postgres writes to a tokio-thread through
an SPSC ring. The pinned tile-thread does computation; the
tokio-thread pays the blocking database cost inside the same process.

A pinned compute tile also must not block on io_uring waits.
`io_uring_enter` can block when submitting and waiting for
completions, so "add io_uring to the matching tile" is not free. The
socket and polling must live either on a separate I/O thread that
owns the ring and hands bytes to the tile over an SPSC ring, or in
SQPOLL mode where the kernel polls the submission ring and the tile
does not make the enter syscall. The compute loop itself stays
non-blocking; the I/O wait lives somewhere allowed to block or burn
a kernel polling thread. See [network-edge-io](network-edge-io.md).

---

Deeper: [blog/18-100ns-matching.md](../../blog/18-100ns-matching.md),
[reports/20260704_book-bench.md](../../reports/20260704_book-bench.md),
[specs/2/45-tiles.md](../../specs/2/45-tiles.md),
[docs/concepts/spsc-rings.md](../../docs/concepts/spsc-rings.md),
[docs/concepts/network-edge-io.md](../../docs/concepts/network-edge-io.md)
