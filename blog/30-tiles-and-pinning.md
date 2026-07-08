# Tiles: One Core, One Loop, No Scheduler

> **Draft — AI-generated, not yet human-edited (slop, for now).** Part of the
> intended educational core; read the specs and code for anything authoritative.

A **tile** is one OS thread pinned to one CPU core, running a tight loop that
never blocks. It owns its state exclusively — no locks, no shared mutability,
no allocator on the hot path — and talks to sibling threads only through
bounded SPSC ring queues. From the kernel's view it is always runnable, so the
scheduler never takes the core away.

RSX runs the matching engine and the risk engine as tiles. Matching matches an
order in ~60 ns at any book depth; risk accepts an order in ~110 ns; an SPSC
ring hop between tiles is 50–170 ns. Those numbers only exist because the loop
never leaves the core.

## Why pinning is the whole game here

The work a tile does is **compute-bound**: fixed-point arithmetic over state
that fits in L1/L2 cache. A match is a few comparisons and pointer walks; a
margin check sums a user's positions. There is no I/O, no syscall, no
allocation. When the entire cost of an operation is a few dozen nanoseconds of
cache-resident arithmetic, the *only* things that can hurt you are the things
the OS does to a normal thread:

- **Scheduler preemption.** A time-sliced thread gets parked mid-loop; the next
  order waits microseconds for it to be scheduled back. A pinned busy-spinner is
  never parked — nothing blocks, so the kernel has no reason to preempt it.
- **Core migration.** The scheduler load-balancer moves a floating thread to a
  different core, and its warm L1/L2 cache evaporates — every access is a cold
  miss until it refills. A pinned thread never migrates; its state stays hot.
- **Context-switch cost.** Even a "cheap" switch is hundreds of ns of TLB and
  register churn — an order of magnitude more than the work itself.

Pin the thread and busy-spin, and all three disappear. You spend one whole core
to buy determinism: the p99 sits within ~1.5× of the p50 instead of spiking to
milliseconds when the scheduler happens to look away.

## What you give up, and the rule that follows

A tile owns a core whether it is busy or idle — a 100 %-CPU spinner. So the
model has one hard rule: **the loop must never block.** No disk I/O, no
synchronous network call, no mutex that might be held elsewhere, no `await`. A
single 1 ms `read()` inside a tile is a 1 ms latency spike for every message
queued behind it.

That is why blocking work lives on a **sidecar**: the risk engine's Postgres
write-behind runs on a separate tokio thread, fed by an SPSC ring, off the
pinned loop. The tile does the solvency-critical arithmetic in RAM on the
critical path; the sidecar persists asynchronously. Cross-tile coordination is
explicit ring drains, not shared locks — one producer, one consumer, cache-line
aligned, no false sharing.

## When a tile is the wrong tool

The tile wins because the cost *is* the compute and pinning removes everything
else. Flip that — make the cost **I/O** — and the logic inverts. The gateway
and marketdata are I/O-bound: their per-request cost is dominated by *syscalls*
(sending and receiving packets, reading and writing client sockets), and a
pinned busy-spin thread cannot make a syscall cheaper. Spinning a core there
buys nothing; the lever is batching syscalls, which an io_uring reactor does and
a spin loop does not. Those processes run on monoio, not as tiles — and that is
a deliberate choice, not an unfinished one. The concept page
[network-edge-io](../docs/concepts/05-network-edge-io.md) works through why.

The one-line test: **if the hot loop is compute-bound and cache-resident, tile
it and pin it. If it is I/O-bound, don't — batch the syscalls instead.**

See also: `docs/concepts/03-tiles-and-pinning.md`, `specs/2/45-tiles.md`,
[The Matching Engine That Runs at 100ns](18-100ns-matching.md).
