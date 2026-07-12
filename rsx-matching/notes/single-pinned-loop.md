# A single pinned loop, not a tile

**Domain term.** A *tile* (`specs/2/45-tiles.md`) is RSX's
intra-process structure: one or more threads pinned to dedicated CPU
cores, handing work between each other over SPSC rings (single-producer
single-consumer lock-free queues, ~50–170 ns per hop). A *reactor* is an
async runtime (tokio/monoio) that multiplexes many I/O sources with an
event loop. Both exist to overlap independent work.

## Problem

The reflex is to build the ME like the other components: a tile with
input/output rings, or an async reactor over its sockets. But overlap
machinery only pays when there is something to overlap. The ME has one
receive socket (orders from risk) and two send sockets (events to risk
and marketdata) — nothing to multiplex. Adding rings would insert a
cross-thread hop (~50–170 ns) on the lowest-latency stage in the whole
system; adding a reactor would add wakeup/poll overhead for sockets that
are always either "one datagram ready" or "spin again".

## Fix

Matching is **one core-pinned thread running a busy-spin loop, no SPSC
rings** — the "degenerate tile" of `specs/2/45-tiles.md` §3.1 ("the whole
process is one tile"). All hot work runs inline on the pinned core:

```
loop {                                   // main.rs:456, one core
  cast_receiver.try_recv_with(|..| {     // orders/cancels in
     dedup → WAL accept → process_new_order → publish_events → index
  });                                    // all inline, cache-warm
  … flush_if_due / snapshot / config (off-path, time-gated) …
}
```

`RSX_ME_CORE_ID` pins the thread (`main.rs`, `setup_hot_thread`); the
loop warns if the core is not isolated (expect tail spikes). The one
piece of genuinely blocking I/O — polling Postgres for config every 10
minutes — is the only thing that touches a tokio runtime, and it runs off
the hot loop (`ARCHITECTURE.md` § "Architectural Decisions").
A separate thread runs the WAL-replication *server* sidecar
(`RSX_ME_REPLICATION_BIND_ADDR`), also off the hot path.

## Cost it removes

A cross-thread ring hop (~50–170 ns) and reactor bookkeeping on the
system's tightest stage. With nothing to multiplex, the cheapest reactor
is no reactor.

## When this would change

If the ME grew a second independent hot input (say, a separate cancel
firehose that must not head-of-line-block orders), a ring to decouple
them would start to pay. It does not today.

## Cite

- `specs/2/45-tiles.md` §3.1 (matching engine — pinned single loop;
  "degenerate tile"); `ARCHITECTURE.md` § "Execution: a single pinned
  loop", § "Main Loop".
- `../CLAUDE.md` "Networking Stack" (Risk vs ME I/O models).
