# SPSC Rings

SPSC rings are how tiles pass data without syscalls or locks. The
tradeoff is strict topology: 1 producer, 1 consumer, bounded capacity,
and backpressure when the ring is full.

## Why one producer and one consumer

RSX uses `rtrb` single-producer/single-consumer rings for
intra-process IPC. With exactly 1 writer and 1 reader, push and pop
are wait-free bounded operations: no mutex, no scheduler wake, no
kernel call. The measured hop cost is 50-170 ns.

The ring keeps its head and tail counters on separate cache lines,
so producer and consumer do not repeatedly invalidate the same cache
line. That avoids false sharing: the producer mostly writes tail,
the consumer mostly writes head, and each side reads the other's
counter only to check fullness or emptiness.

## One ring per consumer

A tile does not broadcast through 1 shared queue. It gives each
consumer its own SPSC ring, so a slow consumer fills only its own
buffer. Other consumers keep draining their rings and do not inherit
that stall.

When a ring fills, the producer stalls. That is deliberate
backpressure, not packet loss. The slow path becomes visible as
ring occupancy and producer stall time, which ties directly to
[backpressure-not-drop](08-backpressure-not-drop.md).

## What SPSC is not

SPSC rings are intra-process IPC only. They are not a network
transport and they do not cross process boundaries. Gateway-to-Risk,
Risk-to-ME, and ME-to-marketdata use casting over UDP; rings connect
sibling threads inside one process.

The cost is shape. Multiple producers need multiple rings or an
arbiter. Multiple consumers need fan-out rings. Capacity must be
chosen up front, and a full ring stalls the producer by design.

---

Deeper: [rsx-risk/notes/spsc.md](../../rsx-risk/notes/spsc.md),
[specs/2/43-testing-smrb.md](../../specs/2/43-testing-smrb.md),
[specs/2/45-tiles.md](../../specs/2/45-tiles.md),
[docs/concepts/02-tiles-and-pinning.md](../../docs/concepts/02-tiles-and-pinning.md),
[docs/concepts/08-backpressure-not-drop.md](../../docs/concepts/08-backpressure-not-drop.md)
