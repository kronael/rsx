# Zero-heap hot path

**Domain term.** A *heap allocation* is asking the OS/allocator for
memory at runtime (`malloc`, `Vec::new`, `Box::new`). It costs 20–80 ns,
can block on a global lock under contention, and scatters data across
memory so the CPU cache misses (`rsx-book/notes/arena.md`). On a busy-spin
sub-microsecond accept loop, a single per-order allocation is a
large, jittery tax.

## Problem

A naive match loop allocates all over the per-order path: a `Vec` for the
freshly received UDP datagram, a `Vec` to collect the fills a match
produced, a fresh timestamp syscall per stage. Each allocation is
20–80 ns and its free is more, the freed memory pollutes cache, and the
allocator lock adds tail latency exactly when load is highest.

## Fix

The receive, book, and event-buffer path uses preallocated storage and creates
no short-lived collection per order. The growable dedup and `(user,oid)` order
maps are the deliberate exception: inserting a new key can allocate when a map
expands. This is not a whole-process zero-allocation claim.

- **Zero-copy receive.** `cast_receiver.try_recv_with(|hdr, payload| …)`
  (`main.rs:482`) runs the whole order body *inside* the callback,
  borrowing the receiver's recv buffer — no per-message `Vec`. The
  callback decodes the `OrderMessage` in place (`decode_payload`).
- **Fixed event buffer, drained in place.** A match writes its fills into
  the book's pre-allocated `[Event; MAX_EVENTS]` (65 536, heap-boxed
  *once* at startup; owned by `rsx-book`, see `ARCHITECTURE.md` § "Event
  Fanout"). `publish_events` iterates `book.events()` and serializes each
  into a stack `Framed` — no intermediate collection of fills.
- **Cached clocks.** The loop samples `Instant::now()` once every 1024
  spins and reuses it (`CLOCK_REFRESH_SPINS`, `main.rs:312`); the dedup
  tracker reads that cached clock (`dedup.rs::refresh_clock`) instead of
  a syscall per order. The hour-scale dedup window and 10 ms flush
  tolerate the coarse tick.
- **Persistent index.** The `(user,oid)→handle` cancel index is one
  `FxHashMap` grown as needed, not rebuilt per order
  (`notes/cancel-index.md`).

```
recv buf (borrowed) ─▶ decode in place ─▶ match into fixed [Event;N]
                                          ─▶ serialize each to stack Framed
   no Vec              no Vec               no short-lived Vec
```

The order *slots* themselves are slab-allocated in `rsx-book` (`O(1)`
alloc/free from a pre-allocated arena — `rsx-book/notes/arena.md`), so
resting an order does not allocate its slot. Dedup/order-index growth may still
call the allocator.

## Cost it removes

Short-lived receive/event/order-slot allocations, allocator-lock tail latency
under load, and their cache pollution. Map capacity growth remains possible
and should be sized/measured separately. The current accept-path number and
its caveats live in `reports/20260716_book-matching-publication.md`.

## Cite

- `main.rs` (zero-copy `try_recv_with`, cached clock);
  `wal.rs::publish_events`; `dedup.rs::refresh_clock`.
- `rsx-book/notes/arena.md` (slab arena for order slots);
  `../CLAUDE.md` "Zero heap allocation on hot path".
