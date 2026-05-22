# Speed — Off-Hot-Path Tracing

Captured 2026-05-22 after commit `08cb179` (rtrb SPSC ring
+ side-thread drainer). Same cluster, same maker, same
probe.

## Headline

| Leg | Baseline (no sub-probes) | Granular (in-line) | **Off-hot-path** | vs Granular |
|-----|-------------------------:|-------------------:|-----------------:|------------:|
| gateway_in → risk_in | 60 | 58 | **50** | -8 |
| risk_in → me_in | 205 | 191 | **181** | -10 |
| me_in → me_out | 158 | 169 | **159** | -10 |
| me_out → risk_out | 39 | 32 | **33** | +1 |
| risk_out → gateway_out | 666 | 718 | **720** | +2 |
| **GW→ME→GW total** | **1 128** | **1 163** | **1 143** | **-20** |

**Same number of sub-stage probes as the Granular column;
~20 µs net win.** That win is the cost of `tracing::info!`
+ `format!("{:016x}{:016x}", ...)` that used to run on the
critical path, now moved to a side thread.

## What changed

Hot path (before, every sub-stage emit):
```rust
tracing::info!(
    target: "latency",
    stage = "me_in",
    oid = format!("{:016x}{:016x}", hi, lo),
    t_us,
    t0_ns = order_msg.timestamp_ns,
);
```

Hot path (after):
```rust
rsx_types::latency::emit("me_in", hi, lo, t_us, ts_ns);
```

The new `emit`:
1. `PRODUCER.with(...)` — thread_local access (~ns)
2. `prod.push(Sample { ... })` — single atomic store + index
   bump on the per-thread `rtrb::Producer<Sample>`
   (typically 20-30 ns)

Side thread (`latency-drain`):
- Wakes every 100 ms
- Drains every registered consumer half
- Emits the SAME `tracing::info!(target = "latency", ...)`
  line shape so the existing dashboard parser is untouched
- Counts and reports any drops via `tracing::warn!` (ring
  capacity is 8 192 samples per thread → 1.3 s of headroom
  at 10 k orders/s with 6 emits/order)

## Why the win is only ~20 µs, not 100+

I expected larger. The granular round showed each
in-line `tracing::info!` cost ~10-20 µs in debug builds.
With 6 macro probes + 9 sub-probes = 15 probes per order,
naive math suggested 150-300 µs of savings.

Actual: ~20 µs. So either:
- The probes were already cheaper than I estimated, OR
- The bottleneck wasn't tracing — it was something else
  in the same code blocks that the tracing happened to
  surround.

The second is true: the GW→ME→GW path is dominated by
the **gateway CMP poll loop's 100 µs sleep**
(`rsx-gateway/src/main.rs:411`) and possibly a symmetric
sleep on the risk side. Neither were touched in this round.

But the off-hot-path move WAS the right thing to do
regardless, because:

1. **Granular telemetry is now ~free.** We can add 10 more
   sub-probes per order to attribute the remaining 700 µs
   inside risk → gateway → WS without inflating the Rust
   budget by a single µs.
2. **The `format!()` allocation churn is gone.** Each emit
   used to allocate a 32-byte `String` for the hex oid;
   that's now in the drainer.
3. **The drain thread's cadence is tunable.** 100 ms is
   safe; if dashboard freshness matters more, drop to 20 ms.

## Coherent N at this snapshot

1 654 orders with all 6 macro stages on the same `t0_ns`
(post-`taker_ts_ns` wire change, no anchor fallback).

## Where the next ~700 µs is

Per the granular sub-stage attribution that's now free:

- **`risk_cmp_send_done → gateway_cmp_recv = 655 µs`** —
  this is the gateway's CMP poll loop tick latency. The
  `monoio::time::sleep(Duration::from_micros(100)).await`
  at `rsx-gateway/src/main.rs:411` is *not* a real yield;
  it adds 100 µs of fixed delay AND incurs monoio's timer
  resolution jitter (effective wake ~500-700 µs).

  **The fix is structural**: replace the timer-based
  "yield" with a `monoio::select!` between
  `cmp_receiver.socket().readable()` and a 10 ms
  maintenance tick. Expected gain: ~500-600 µs on this
  leg → Rust GW→ME→GW drops to ~500 µs.

- **`me_in → me_out = 159 µs`** — even with tracing off-
  hot-path, this is 3x what the 54 ns Criterion match
  suggests. Need finer sub-probes inside `process_new_order`
  and `write_events_to_wal` to attribute the remaining
  µs. Now feasible because adding probes is free.

## How to reproduce

Same as `SPEED-BASELINE.md` "How to reproduce" section.
Per-stage mining script also unchanged (the drained log
lines look identical to the in-line emissions). The only
difference at the consumer end is a ~100 ms freshness
delay on the dashboard's `/x/latency-stages` endpoint
(samples emitted at T are visible at T + drain_interval).

## What this unblocks

Future per-stage drilldowns no longer need a budget
trade-off discussion. Add probes liberally; the drain
thread eats the cost.
