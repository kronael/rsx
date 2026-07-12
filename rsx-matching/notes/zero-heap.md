# Zero-heap hot path

**Domain term.** A *heap allocation* is asking the OS/allocator for
memory at runtime (`malloc`, `Vec::new`, `Box::new`). It costs 20–80 ns,
can block on a global lock under contention, and scatters data across
memory so the CPU cache misses (`rsx-book/notes/arena.md`). On a busy-spin
loop that wants a ~266 ns accept path, a single per-order allocation is a
large, jittery tax.

## Problem

A naive match loop allocates all over the per-order path: a `Vec` for the
freshly received UDP datagram, a `Vec` to collect the fills a match
produced, a fresh timestamp syscall per stage. Each allocation is
20–80 ns and its free is more, the freed memory pollutes cache, and the
allocator lock adds tail latency exactly when load is highest.

## Fix

The matching loop allocates **nothing per order**. Every buffer is
pre-sized once; the per-order path only reads and writes into memory it
already owns.

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
- **Pre-allocated index.** The `(user,oid)→handle` cancel index is one
  `FxHashMap` grown as needed, not rebuilt per order
  (`notes/cancel-index.md`).

```
recv buf (borrowed) ─▶ decode in place ─▶ match into fixed [Event;N]
                                          ─▶ serialize each to stack Framed
   no Vec              no Vec               no Vec, no per-order malloc
```

The order *slots* themselves are slab-allocated in `rsx-book` (`O(1)`
alloc/free from a pre-allocated arena — `rsx-book/notes/arena.md`), so
even resting an order touches no `malloc`.

## Cost it removes

Per-order allocator calls (20–80 ns each, plus frees), allocator-lock
tail latency under load, and the cache pollution of short-lived heap
objects. This is part of why the full accept path measures 266 ns
(`notes/depth-independent.md` for the number and its caveats).

## Cite

- `main.rs` (zero-copy `try_recv_with`, cached clock);
  `wal.rs::publish_events`; `dedup.rs::refresh_clock`.
- `rsx-book/notes/arena.md` (slab arena for order slots);
  `../CLAUDE.md` "Zero heap allocation on hot path".
