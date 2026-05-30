# 20260530 — component microbenches (the floors)

**What:** isolated in-process component latencies (no UDP/WS) — the hard floors each layer adds. Criterion. Sources: `rsx-matching/benches`, `rsx-risk/benches`, `rsx-cast/benches`, `rsx-book/benches`.

## Numbers
- **ME in-process match floor:** ~**210 ns** p50 (`me_process_order_full_path`: dedup + match + WAL-append, no fsync).
- **Risk margin:** pretrade check ~**110 ns**; `apply_fill` ~3.6 ns; BBO→index ~5 ns; exposure lookup ~1.6 ns.
- **Casting loopback RTT** (A→B→A, fill echo, `cmp_rtt_fill_echo`): ~**7.6 µs** → one one-way casting hop ≈ 3.8 µs.
- **Deep-book bench** (`rsx-book/benches/deep_book_bench.rs`, fat-tailed Student-t seed): **match ~52 ns FLAT at 100k / 1m / 10m resting** (depth-independent — O(consumed), not O(resting)); insert ~190–215 ns flat. `OrderSlot` = 128 B, slab u32-indexed → RAM-bound (10M ≈ 1.3 GB).

## Conclusion
The internal compute floors are ns–µs. A full GW→ME→GW round-trip floor ≈ 4 casting hops (~15 µs) + ~330 ns compute — i.e. **transport-bound, not compute-bound**. Matching scales to 10M resting orders at the same ~52 ns. The gap between this floor and the measured ~11 ms e2e (see `20260530_e2e-ws-probe.md`) is gateway egress scheduling, not the engine.

## Caveats
In-process, no kernel/UDP/WS; criterion steady-state (p50). The casting RTT is loopback. These are floors — production adds transport + scheduling.
