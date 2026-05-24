---
status: partial
---

# Tile architecture

A "tile" is a pinned thread doing one thing, communicating
with sibling threads through SPSC rings. Tiles trade
language and runtime simplicity for predictable latency:
no scheduler, no async wakeup, no allocator on the hot
path. They sit between processes (which are isolated by
OS) and async tasks (which share a runtime).

This document describes the tile pattern, the parts of RSX
that use it today, the parts that don't, and why.

The previous status field said "shipped." That overstated
it. Tiles are shipped in the **risk engine** (the heaviest
user) and in **mark price aggregation**. The **matching
engine** uses a pinned-thread single-loop variant (one
"tile" = the whole process). **Gateway** and **marketdata**
use monoio async, not tiles, by deliberate choice. Status
is now "partial" to reflect that.

## Table of contents

- [1. Why tiles](#1-why-tiles)
- [2. The pattern](#2-the-pattern)
- [3. Per-process status](#3-per-process-status)
- [4. Inter-process communication](#4-inter-process-communication)
- [5. Threading inventory](#5-threading-inventory)
- [6. Why some processes aren't tiled](#6-why-some-processes-arent-tiled)
- [7. Performance characteristics](#7-performance-characteristics)
- [8. Future work](#8-future-work)
- [Cross-references](#cross-references)

---

## 1. Why tiles

A tile is a `std::thread::spawn` that runs a tight loop on
a dedicated CPU core, drains one or more SPSC input rings,
performs computation, and writes to one or more SPSC output
rings. The pattern dates to LMAX Disruptor (2011) and is
standard in Seastar / ScyllaDB and most HFT trading systems.

What it gives you:
- **No scheduler latency.** The thread is pinned and
  busy-spinning; the kernel won't take it away from the
  core for ≥10 ms because it's never actually blocked.
- **L1/L2-warm hot data.** All state owned by a tile lives
  on its core's caches.
- **Backpressure for free.** A full output ring stalls
  the producer; the consumer can't be DoSed.
- **Single-writer invariant.** Each ring has one producer
  and one consumer; the lock-free protocol is `rtrb`'s
  cache-line-aligned head/tail.

What you give up:
- **You eat one core per tile.** A 4-tile process needs
  4 cores it actually owns.
- **Cross-tile coordination is rings, not async.** No
  `await`, no `select!` — explicit drain loops.
- **A blocking operation in a tile poisons the whole
  process.** A syscall that takes 1 ms is a 1 ms latency
  spike.

So the rule is: tile when you need predictable
microsecond-scale latency on a hot loop; use async
elsewhere.

## 2. The pattern

```rust
fn risk_tile(
    mut order_in:   rtrb::Consumer<OrderRequest>,
    mut fill_in:    rtrb::Consumer<FillEvent>,
    mut order_out:  rtrb::Producer<OrderResponse>,
    mut persist_out: rtrb::Producer<PersistEvent>,
) {
    core_affinity::set_for_current(MY_CORE);
    let mut state = RiskState::new();
    loop {
        while let Ok(o) = order_in.pop() {
            state.handle_order(&o, &mut order_out,
                               &mut persist_out);
        }
        while let Ok(f) = fill_in.pop() {
            state.handle_fill(&f, &mut persist_out);
        }
        // tick liquidations, mark price, BBOs ...
    }
}
```

The tile drains every ring on every iteration. Output
rings are bounded; if `order_out` is full, the producing
loop stalls (`order_out.push()` returns `Err`). The
matching engine and risk shard implement explicit
backpressure handling for these stalls.

## 3. Per-process status

The actual runtime layout, with file references for each
claim. Read this before believing the architecture
diagrams.

### 3.1 Matching engine — `rsx-matching` (pinned single loop)

```
rsx-matching/src/main.rs:193-202
    if let Ok(core_str) = env::var("RSX_ME_CORE_ID") {
        if let Ok(core_id) = core_str.parse::<usize>() {
            let ids = core_affinity::get_core_ids()
                .unwrap_or_default();
            if let Some(id) = ids.get(core_id) {
                core_affinity::set_for_current(*id);
                info!("pinned to core {}", core_id);
            }
        }
    }
```

Matching is **one core-pinned thread**, no SPSC rings
within the process. Inside that loop:

1. Drain casting UDP recv (orders from risk).
2. Run matching algorithm against the orderbook.
3. Append events to WAL writer (inline call, not a ring).
4. Send fills via casting UDP to risk (and to marketdata).

Why no rings: there's only one thread doing computational
work, so there's nothing to send across a ring to. The
WAL writer is an inline data structure with periodic
fsync; the replication TCP server runs on a separate
`std::thread::spawn` (not pinned) and reads from the
already-flushed WAL files.

This is sometimes called a "degenerate tile" — the whole
process is one tile.

### 3.2 Risk shard — `rsx-risk` (full tile architecture)

The heaviest user of tiles. One pinned thread for the risk
state machine, plus 7 SPSC rings for input and output:

| Ring                       | Capacity | File:line                       |
|----------------------------|----------|---------------------------------|
| `PersistEvent`             | 8 192    | `rsx-risk/src/main.rs:239`      |
| `FillEvent`                | 4 096    | `rsx-risk/src/main.rs:405`      |
| `OrderRequest` (primary)   | 2 048    | `rsx-risk/src/main.rs:407`      |
| `MarkPriceUpdate`          | 256      | `rsx-risk/src/main.rs:409`      |
| `BboUpdate`                | 256      | `rsx-risk/src/main.rs:411`      |
| `OrderResponse`            | 2 048    | `rsx-risk/src/main.rs:413`      |
| `OrderRequest` (replica)   | 2 048    | `rsx-risk/src/main.rs:415`      |

Core pinning at `rsx-risk/src/main.rs:291-304`. Postgres
write-behind is a separate `tokio` runtime in a sidecar
thread (it does blocking IO, can't live on the pinned
core).

This is the canonical RSX tile arrangement.

### 3.3 Mark — `rsx-mark` (partial tile)

```
rsx-mark/src/main.rs:118
    rtrb::RingBuffer::<SourcePrice>::new(1024);
```

The aggregator main loop is a synchronous busy-spin that
drains one SPSC ring per external price source (Binance,
Coinbase, …). The sources themselves are async tasks on
a `tokio` runtime that scrape WebSockets and push into
the rings. Aggregator → casting/UDP send is inline.

The aggregator is not currently core-pinned. The next
ship cycle migrates the source tasks off `tokio` and
adds core affinity. Tracked in `.ship/12-SHOWCASE-HONEST/`
task 18.

### 3.4 Gateway — `rsx-gateway` (monoio async, not tiled)

Gateway is one `monoio` runtime per thread (single-thread
per-core scaling in deployment, but a single thread in
default dev config). Inside:

- Accept loop spawns one task per WebSocket connection.
- Each task `read_frame → validate → CmpSender::send` to
  the risk shard.
- Fill responses come back from risk via casting, route to
  the right WS task via a per-connection broker channel.

There is no SPSC ring within gateway. The justification:
WebSocket parsing dominates the per-frame cost, and
io_uring batches the syscalls; a tile would have to do
the same WS parsing. The flow-control buffer between
gateway and risk is the casting wire window (§5 of `4-cast.md`),
not a ring.

### 3.5 Marketdata — `rsx-marketdata` (monoio async, not tiled)

Same pattern as gateway: `monoio` runtime, async WS
broadcast tasks, casting UDP recv for fill/insert/cancel
events from ME. No SPSC rings, no core pinning.

The `replay.rs` startup path is on `tokio` (one-shot
catch-up before going live); not on the hot path.

## 4. Inter-process communication

Within a process, where rings exist, they're rtrb SPSC.
Between processes, casting/UDP for the hot path and TCP+WAL
for the cold path. See `4-cast.md` for the wire protocol.

```
Gateway --[casting/UDP]--> Risk --[casting/UDP]--> ME
Gateway <--[casting/UDP]-- Risk <--[casting/UDP]-- ME
                       Risk --[SPSC]-----> PG write-behind
                                    ME --[fsync+notify]--> WAL files
                          WAL files --[TCP fan-out]--> {recorder, mktdata}
                          ME --[casting/UDP]--> Marketdata
                          Mark --[casting/UDP]--> Risk
```

## 5. Threading inventory

A complete count of OS threads by process, default config:

| Process       | Pinned | Async (tokio/monoio) | SPSC rings |
|---------------|--------|----------------------|------------|
| rsx-matching  | 1      | 1 (replication TCP)   | 0          |
| rsx-risk      | 1      | 1 (tokio: PG)        | 7          |
| rsx-gateway   | 0      | 1 (monoio)           | 0          |
| rsx-marketdata| 0      | 1 (monoio)           | 0          |
| rsx-mark      | 0      | 1 (tokio: HTTP/WS)   | 1+         |
| rsx-recorder  | 0      | 1 (tokio: TCP)       | 0          |

The aspirational "everything is a tile" picture in earlier
revisions of this spec was wrong. What's actually shipped
is hybrid: tile-architected where it pays off (risk),
async where I/O multiplexing dominates (gateway,
marketdata), single-loop where there's only one thing to
compute (matching).

## 6. Why some processes aren't tiled

Not every process needs tiles. The tradeoff is:

- **Use tiles** when the hot loop is compute-bound and you
  want bounded, single-digit-microsecond tail latency.
- **Use async** when the loop is I/O-bound across many
  fds, and io_uring batching beats one-thread-per-fd.

Risk is compute-bound (margin checks, fill application,
liquidation triggers) so it's a tile. Gateway is I/O-bound
across N WebSocket connections, so it's monoio. Mark is
I/O-bound across M external feeds, so its scrapers are
tokio.

If gateway grew to a workload where WS parsing was no
longer the bottleneck (e.g. binary FIX-like protocol),
tiling it would make sense. Today it doesn't.

## 7. Performance characteristics

Measured CPU costs of the tile primitives (from
`rsx-book/benches`, `rsx-gateway/benches`):

| Operation                                                    | ns      |
|--------------------------------------------------------------|---------|
| SPSC `Producer::push` (rtrb)                                 | 50–170  |
| SPSC `Consumer::pop` (rtrb)                                  | 50–170  |
| Match single fill (orderbook)                                | 54      |
| Protocol-record encode (StatusMessage / Nak / Heartbeat)     | 43      |
| Protocol-record decode (one record)                          | 9       |
| `FillRecord` encode                                          | 23      |
| `WalWriter::append` (Vec extend, no disk I/O)                | 31      |

These are the building blocks. End-to-end **GW → ME → GW**
under load is **not currently gated** by an automated
harness — see `22-perf-verification.md` and
`.ship/12-SHOWCASE-HONEST/` task F1. Expect numbers but
don't trust them until the harness lands.

## 8. Future work

Tracked in `.ship/12-SHOWCASE-HONEST/`:

- **F1: end-to-end latency harness.** Wire ts_ns
  timestamps in gateway + ME into a probe that returns
  GW→ME→GW microseconds. Convert the design budget into a
  measurement.
- **18: mark off tokio.** Move source scrapers to monoio
  TCP, add core pinning to the aggregator.
- **17: monoio UDP for gateway.** Gateway owns the casting
  `UdpSocket`; replace with monoio io_uring SQEs for
  batched send/recv. rsx-dxs itself stays runtime-free
  (invert-ownership: caller passes bytes, not socket).
- **userspace networking (DPDK / AF_XDP).** Long-horizon.
  The tile / async split above stays the same; only the
  network driver changes.

## Cross-references

- `specs/2/4-cast.md` — casting wire protocol and flow control
- `specs/2/10-replication.md` — replication (TCP fan-out)
- `specs/2/20-network.md` — Process topology, ports
- `specs/2/22-perf-verification.md` — Bench gate, harness plan
- `specs/2/48-wal.md` — WAL flush, fsync, rotation
