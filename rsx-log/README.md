# rsx-log

Off-hot-path logging primitive. Per-thread SPSC ring +
dedicated drain thread → structured `tracing::*` events.

## What It Provides

- `Record` — fixed-shape log record (~20–30 ns push cost on
  the hot path)
- Per-thread `rtrb::Producer<Record>` registered once per
  thread lifetime
- `start_drainer(interval_ms)` — side thread that iterates
  every registered consumer, drains records, dispatches to
  `tracing::event!`
- Bounded ring; full ring drops with a counter rather than
  blocking the producer

## Why a Separate Crate

`rsx-types` is the foundation crate and must not pull
`tokio` / `tracing` / `rtrb` into every downstream component.
`rsx-log` is opt-in — only producers that emit structured
log records depend on it.

## Architectural Decisions

**Runtime: none on the hot path; tokio-adjacent on the
drain thread.** Producers push records from whatever thread
they happen to run on (tile, monoio reactor, tokio task).
The drain side is a `std::thread::spawn` that periodically
walks the registry and calls `tracing::event!`. There is no
async runtime requirement on either side.

This is the tile pattern in miniature, applied to telemetry:
producer is single-writer to a `rtrb` ring, consumer is
single-reader, and the bounded ring gives backpressure for
free (drop on full, surfaced as a counter). See
[`../notes/tiles.md`](../notes/tiles.md) for the broader
pattern.

## Dependencies

- `rtrb` — SPSC ring buffer
- `tracing` — drain output
