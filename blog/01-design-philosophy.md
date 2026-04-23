# Design Philosophy: Why We Built RSX From Scratch

RSX is a perpetual futures exchange built in Rust. Not a framework,
not a library -- a complete exchange: matching engine, risk engine,
gateway, market data, mark price aggregation, and WAL-based recovery.
We wrote it from spec to working system in three days.

This post explains the design decisions that shaped the system.

## Spec-First, Code Second

We wrote 35 specification documents before writing a single line of
Rust. The specs directory (`specs/2/`) contains everything: orderbook
data structures, matching algorithm, risk formulas, WAL format, CMP
wire protocol, deployment topology, testing strategy, and edge case
catalogs.

Why spec-first? Because an exchange has dozens of interacting
components with subtle invariants. If you start coding the matching
engine without specifying how fills propagate to the risk engine,
you discover the protocol mismatch at integration time. We
discovered it at spec review time instead.

The specs went through multiple refinement rounds. We wrote a
CRITIQUE.md that identified 36 design gaps -- ambiguous ack
semantics, missing dedup windows, underspecified backpressure
thresholds. All 36 were resolved in the specs before we wrote
the first `impl` block.

## Zero Heap on the Hot Path

The hot path is the sequence: order arrives at gateway, passes
risk check, enters matching engine, produces fills, fills flow
back to risk and gateway. Our target is <50us end-to-end and
<500ns for the matching engine alone.

Heap allocation is the enemy of predictable latency. A single
`malloc` can take microseconds if the allocator needs to request
pages from the kernel. Worse, it introduces jitter -- sometimes
fast, sometimes slow, depending on fragmentation.

Our approach: pre-allocate everything at startup.

The orderbook uses a slab allocator for all order storage:

```rust
pub struct Slab<T: SlabItem> {
    slots: Vec<T>,
    free_head: u32,
    bump_next: u32,
}
```

The `Vec<T>` is allocated once at startup with a fixed capacity.
New orders come from a free list (O(1) pop) or a bump pointer.
Cancelled orders return to the free list. No heap allocation
ever touches the matching path.

Price levels work the same way. The `CompressionMap` maps prices
to array indices:

```rust
pub struct CompressionMap {
    pub mid_price: i64,
    pub thresholds: [i64; 4],
    pub compressions: [u32; 5],
    pub base_indices: [u32; 5],
    pub zone_slots: [u32; 5],
}
```

Five zones with increasing tick compression. Zone 0 (within 5%
of mid) has 1:1 tick resolution. Zone 4 (50%+ away) compresses
heavily -- those prices rarely trade, so two slots suffice. The
total array size is bounded and allocated once.

## Fixed-Point Arithmetic

Every value in RSX is an `i64` in smallest units. No floats,
anywhere.

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Price(pub i64);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Qty(pub i64);
```

`#[repr(transparent)]` means these are zero-cost wrappers around
`i64` at the machine level. The type system prevents mixing prices
and quantities, but the CPU sees plain integers.

Conversion happens at the API boundary only:

```
price_raw = (human_price / tick_size) as i64
qty_raw   = (human_qty / lot_size) as i64
```

For BTC-PERP with tick_size $0.01: `Price(5000001)` = $50,000.01.

Why not floats? Floating point is non-deterministic across
architectures. The same multiplication can produce different
results on x86 vs ARM due to extended precision registers and
compiler optimizations. In an exchange, this means two replicas
processing the same fills can diverge. Fixed-point eliminates
this class of bugs entirely.

Overflow checking happens at order entry (the cold path), not
during matching. We use `checked_mul` for notional calculations
at the risk boundary, where the cost is acceptable.

## io_uring Over epoll

Gateway and market data are the network-facing components. They
accept WebSocket connections, parse orders, and fan out market
data updates. Every microsecond of I/O latency adds directly to
the end-to-end path.

Traditional async Rust (tokio) uses epoll: the application asks
the kernel "are any of my file descriptors ready?" on every poll
cycle. Each question is a syscall. Syscalls cost 1-5us on modern
Linux due to context switches and speculative execution
mitigations.

io_uring (via monoio) eliminates most of these syscalls. The
kernel and application share a pair of ring buffers in memory.
The application writes submission queue entries (SQEs) describing
what I/O it wants. The kernel writes completion queue entries
(CQEs) describing what finished. Multiple I/O operations are
batched in a single submission, and completions arrive without
a syscall.

We use monoio for all critical-path network I/O. Auxiliary
tasks (telemetry, archival, external API calls) can use tokio
where library support matters more than latency.

## Tile Architecture

Each RSX process is internally organized as "tiles" -- pinned
threads, each responsible for one concern, connected by SPSC
(single-producer single-consumer) ring buffers.

```
Matching Engine process:
+===============================================+
|  +-------+  SPSC  +---------+  SPSC  +------+ |
|  |  Net  |------->| Matching|------->| WAL  | |
|  | tile  |<-------| tile    |------->|Writer| |
|  |(monoio|  fills |         | events | tile | |
|  +-------+        +---------+        +------+ |
+===============================================+
     CMP/UDP                    TCP
   Risk Engine              Recorder
```

Within a process: SPSC rings via `rtrb`. Same address space,
zero syscall overhead, 50-170ns per hop. Each consumer gets its
own ring, so a slow market data consumer cannot stall the risk
engine feed.

Between processes: CMP/UDP for the hot path, WAL replication over
TCP for the cold path.

Why SPSC instead of MPSC or a lock-free queue? Because each tile
runs on a dedicated pinned core. There is exactly one producer and
one consumer per ring. SPSC rings have the lowest latency of any
IPC primitive -- no compare-and-swap, no memory barriers beyond
what the cache coherence protocol provides.

The tile model also makes the system easy to reason about. Each
tile is a single-threaded loop. No shared mutable state, no locks,
no data races. The SPSC ring is the only communication channel,
and it preserves FIFO ordering.

## CMP: One Wire Format for Everything

CMP (C Message Protocol) is our wire format. A CMP message is a
16-byte WAL header followed by a `#[repr(C, align(64))]` payload:

```
struct WalHeader {       // 16 bytes
    record_type: u16,
    len: u16,
    crc32: u32,
    _reserved: [u8; 8],
}
```

The same bytes are written to WAL files on disk, sent over UDP
between processes, streamed over TCP for replay, and read into
memory structs. No serialization, no deserialization, no format
transformation.

```
WAL bytes = disk bytes = wire bytes = memory bytes
```

This is not a novel idea -- financial systems have used fixed-record
formats for decades. But it is a deliberate rejection of protobuf,
flatbuffers, and similar schema-based serialization. Those formats
add encoding/decoding overhead on every message. When your target
is <500ns per match, even a 100ns serialization step is 20% of
your budget.

## Backpressure, Not Drops

RSX never drops data silently. When a consumer falls behind, the
producer stalls.

- WAL buffer full? Matching engine stalls on order processing.
- SPSC ring full? Producer waits (bare busy-spin, no `spin_loop`).
- Postgres write-behind lag > 100ms? Risk stalls fill processing.
- Gateway overloaded? Explicit rejection to the user.

This is a deliberate tradeoff. Stalling increases latency for
the current request but preserves the invariant that fills are
never lost. In an exchange, a lost fill means incorrect positions,
incorrect margin, and potential insolvency. A delayed fill means
one user waits an extra millisecond.

The backpressure propagation chain:

```
User -> Gateway -> Risk -> ME
         ^          ^       ^
         |          |       |
      reject    stall    stall
     on timeout on ring  on WAL
                 full     full
```

## What These Decisions Cost

Every design choice has a price.

Fixed-record formats mean adding a field requires coordinated
deployment of all producers and consumers. Slab allocators mean
capacity must be provisioned upfront -- you cannot handle a
sudden 10x increase in orders without restarting with a larger
slab. Pinned cores mean the system needs dedicated hardware; you
cannot share cores with other workloads. SPSC rings mean the
topology is fixed at compile time.

These are acceptable tradeoffs for an exchange. The hardware cost
of dedicated cores is negligible compared to the financial risk of
unpredictable latency. The operational cost of coordinated
deployments is manageable with 6 processes. The capacity limit of
a slab allocator is a feature, not a bug -- it provides natural
backpressure and prevents the system from consuming unbounded
memory during a traffic spike.

The result is a system where every operation on the hot path has
a bounded, predictable cost. No allocator contention, no
serialization overhead, no lock contention, no syscall latency
spikes. The matching engine processes a fill in <500ns because
there is nothing in the path that can surprise it.
