# Tiles and Pinning

A "tile" is a thread that owns one CPU core, runs a tight
loop, and communicates with sibling threads only through
bounded SPSC rings. It never blocks on I/O. It never yields
to the scheduler. From the OS's perspective, it is always
runnable — so the kernel never takes it away.

The pattern trades CPU cores for predictable latency. You
spend one core to eliminate scheduler jitter, context switches,
and cache eviction caused by the OS migrating the thread.

## When to tile

The rule is: tile when the hot loop is compute-bound and you
need single-digit-microsecond tail latency. Use an async
runtime when the loop is I/O-bound across many file descriptors.

`rsx-risk` is the canonical tile. It runs one pinned busy-spin
thread that drains seven SPSC rings (orders from gateway, fills
from matching, BBO and mark-price updates) and drives the margin
state machine. Margin computation is pure computation: no I/O,
no syscall, no allocation. A 50–170 ns ring hop is the relevant
latency unit. Postgres write-behind lives on a separate tokio
sidecar thread and shares no locks with the pinned loop.

`rsx-matching` is a degenerate tile: the whole process is one
pinned thread. There are no SPSC rings because there is no other
thread to ring to. The loop drains UDP packets, runs the
matching algorithm, appends to the WAL writer inline, and sends
fills via UDP. Measured: 54 ns per single fill through the book.

Gateway and marketdata use monoio with io_uring. Both processes
handle many concurrent WebSocket connections — the gateway
muxes client orders onto the casting wire; marketdata fans
order-book events out to public subscribers. The bottleneck in
both cases is I/O multiplexing across N file descriptors, not
computation. io_uring batches the syscalls. A tile would have
to do the same WS parsing without gaining anything from
pinning.

SPSC ring measured performance: 50–170 ns per hop, from the
`rsx-book` benchmarks.

## Why an unpinned spinner is dangerous

An unpinned busy-spinner is worse than useless: it consumes 100%
of whatever core the OS assigns it and floats across cores under
CFS load balancing. When it lands on a core already owned by a
pinned hot-path process, it starves that process — the victim
stalls, its UDP socket falls behind, and packets start to drop.

So the rule has two parts. Pin the processes that need pinning
and document which cores they own. And keep off-path services
(mark, recorder) on a sleep loop, not a spin loop — they are not
on the order critical path and do not need busy-spin latency. An
unpinned sleeper uses ~0% CPU between events; an unpinned spinner
eats 100% and wanders.

The six-core layout in the root README formalizes this: cores 1
through 4 are owned by gateway, risk, matching, and marketdata
respectively. Core 0 carries the OS and the sleeping off-path
services. Core 5 is headroom.

## What you give up

Each tile owns a core whether it is busy or idle. A 4-tile
process on a 4-core machine leaves nothing for the OS. A
blocking operation inside a tile — a syscall that takes 1 ms —
is a 1 ms latency spike for every message that arrives while it
is blocked. The tile model requires that the hot loop contains
no blocking operations: no disk I/O, no synchronous network
calls, no mutexes that might be held elsewhere.

This is why Postgres write-behind lives in a sidecar: writing to
Postgres is a blocking operation and it must not be on the
pinned thread.

---

Deeper: [blog/18-100ns-matching.md](../../blog/18-100ns-matching.md),
[specs/2/45-tiles.md](../../specs/2/45-tiles.md)
