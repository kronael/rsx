# rsx-log

An off-hot-path logging primitive: a per-thread wait-free SPSC
ring feeds a single drain thread that turns records into
`tracing` events.

The producer side is a ~20–30 ns wait-free push of a fixed-shape
`Record` onto a thread-local `rtrb` ring — no allocation, no
lock, no syscall, no clock read on the hot path. A dedicated
drain thread wakes on an interval, sweeps every registered
consumer, and dispatches each record to `tracing::event!` off
the hot path. A full ring drops the record and bumps a counter
rather than blocking the producer, so a pinned busy-loop tile
never stalls on telemetry.

## What it provides

- **`Record`** — a fixed-shape, `Copy` log record (a `Kind`, a
  `&'static str` label, four `u64` payload slots). No heap
  pointers, so a push allocates nothing.
- **`Kind`** — record discriminant; today only `Kind::Latency`.
- **`push(record)`** — hot-path-safe wait-free SPSC push onto
  this thread's ring. The first push per thread lazily allocates
  the ring (~µs) and registers its consumer; steady state is the
  fast path.
- **`latency_sample!(stage, oid_hi, oid_lo, t0_ns)`** — the
  intended entry point for latency tracing. It **compiles to
  nothing** unless the *calling* crate enables its own
  `latency-trace` feature — no clock read, no push, the argument
  expressions aren't even evaluated. Zero hot-path cost in
  production builds.
- **`latency::emit(...)`** — the function the macro expands to;
  reads the clock, computes the µs delta, and pushes. Call it
  via the macro, not directly.
- **`now_ns()`** — wall-clock (CLOCK_REALTIME) nanoseconds, a
  *shared* clock so stage samples from different processes
  correlate.
- **`start_drainer(interval_ms)`** — spawn the drain thread.
  Call once per process, near the top of `main()`, *after*
  `tracing_subscriber` is initialized.

## Usage

The producer enables `latency-trace` in its own manifest and
sprinkles samples on the path it wants to profile:

```toml
# in the consuming crate's Cargo.toml
[features]
latency-trace = []
```

```rust
use rsx_log::latency_sample;

// In main(), after tracing_subscriber::fmt::init():
rsx_log::start_drainer(100); // drain every 100 ms

// On the hot path — compiles away unless `latency-trace` is on:
latency_sample!("risk_out", oid_hi, oid_lo, order.ts_ns);
```

Any crate that invokes `latency_sample!` **must** declare the
`latency-trace` feature (`[features] latency-trace = []`), or the
`#[cfg]` inside the macro trips the `unexpected_cfgs` lint.

## Why a separate crate

`rsx-types` is the foundation crate and must not pull `rtrb` /
`tracing` into every downstream component. `rsx-log` is opt-in:
only producers that emit structured log records depend on it. It
is the tile pattern applied to telemetry — single-writer ring,
single-reader drain, bounded buffer for free backpressure.

## Design guarantees

- **Zero hot-path cost when disabled.** `latency_sample!` behind
  a caller feature flag expands to nothing; the compiler removes
  the call and its argument evaluation.
- **~20–30 ns per push when enabled.** One wait-free SPSC push
  into a preallocated ring.
- **Bounded, non-blocking.** The ring is `RING_CAP` (8192) slots
  per thread. A full ring drops the record and increments a
  global counter; the producer never blocks. The drain thread
  logs the drop count (once per interval) as a `tracing::warn!`.
- **Single-writer / single-reader per ring.** Each thread owns
  its producer; the drain thread is the sole consumer of every
  registered ring.

## Dependencies

- `rtrb` — wait-free SPSC ring buffer.
- `tracing` — the drain thread's output sink.

No async runtime on either side; the drain thread is a plain
`std::thread`.

## Testing

```
cargo test -p rsx-log
```

Unit tests live in `src/lib.rs` (`#[cfg(test)]`): a ring
round-trip and a drop-counter-on-overflow check. They share
process-global state (the consumer registry + drop counter) and
serialize via an internal guard so `make test`'s multi-threaded
runner stays deterministic.

## MSRV

Rust 1.78+ on stable, edition 2021. No nightly features.

## See also

- ARCHITECTURE.md — the ring registry, the drain loop, and the
  feature-gate mechanism.

## License

Internal-use crate within the wider rsx exchange project.
Licensed under the MIT license. Not published to crates.io;
distribution is the maintainer's decision.
