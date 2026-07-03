# rsx-log Architecture

Producer/consumer split: many hot threads push fixed-shape
records onto their own wait-free SPSC rings; one drain thread
sweeps every ring on an interval and emits `tracing` events. The
hot side does no allocation, no locking, and no clock read
(unless a sample is actually taken); all the cost lives on the
drain side, off the hot path.

## Data structures

- **`Record`** — `#[derive(Clone, Copy)]`, fixed shape: a `Kind`
  discriminant, a `&'static str` label (`stage_or_target`), and
  four `u64` slots (`a`/`b`/`c`/`d`). Static-str + integers only,
  so a push copies bytes and never touches the heap. For
  `Kind::Latency` the slots are order-id high/low, the µs delta,
  and the anchor timestamp.
- **Per-thread producer** — `thread_local!` holding
  `Option<rtrb::Producer<Record>>`, lazily initialized on the
  first `push` from that thread.
- **Consumer registry** — a process-global
  `OnceLock<Mutex<Vec<Consumer<Record>>>>`. When a thread's ring
  is created, its consumer half is pushed into this Vec so the
  drain thread can find it.
- **`DROPPED`** — a global `AtomicU64` counting records dropped
  on ring-full.

## Push path (hot)

`push(record)`:

1. Borrow the thread-local producer cell.
2. If `None`, call `init_thread_ring()` — allocate an
   `rtrb::RingBuffer::<Record>::new(RING_CAP)`, register the
   consumer half in the global Vec (brief mutex on this slow
   first-call-per-thread path only), keep the producer half.
3. `prod.push(record)`. On `Ok`, done. On `PushError::Full`,
   `DROPPED.fetch_add(1, Relaxed)` and drop the record.

Steady state (ring already initialized, not full) is a single
wait-free SPSC push: ~20–30 ns, no lock, no allocation.

`RING_CAP` is 8192. At ~6 emits/order × ~10 k orders/s × 100 ms
drain interval that is ~1.3 s of per-thread headroom before a
slow drain causes drops.

## Feature gate

`latency_sample!` wraps the `emit` call in
`#[cfg(feature = "latency-trace")]`. The feature is declared by
the *calling* crate, not by rsx-log — so a production build of a
consumer that leaves the feature off compiles the macro (and its
argument expressions, including the clock read) to nothing. This
is why the callers must each declare `latency-trace = []`, and
why a caller that invokes the macro without declaring the feature
trips `unexpected_cfgs`.

## Drain path (off hot path)

`start_drainer(interval_ms)` spawns a named `std::thread` that
loops:

1. `sleep(interval)`.
2. Lock the registry, drain every registered consumer's ring
   into a reusable `batch` Vec (`while let Ok(r) = cons.pop()`).
3. Swap out `DROPPED`; if non-zero, emit a `tracing::warn!` with
   the count.
4. Dispatch each batched record to `tracing::event!` via
   `dispatch()` — for `Kind::Latency`, a `tracing::info!` on the
   `latency` target with the stage, formatted order id, µs delta,
   and anchor.

The drain thread must start *after* `tracing_subscriber` is
initialized so its own emissions land in the process log.

## Why CLOCK_REALTIME

`now_ns()` reads wall-clock time (via the VDSO), not a monotonic
`Instant`. Latency stages are stamped in different processes
(gateway, risk, matching); a shared epoch clock lets a sample
taken in one process be subtracted from an anchor set in another.
A per-process `Instant` could not correlate across the process
boundary.

## Runtime

None required. Producers push from whatever thread they run on
(tile, monoio reactor, tokio task); the drain side is a plain
`std::thread`. No async executor on either side.
