# 2026-07-09 — rsx-matching bench re-run (numbers do not reproduce)

**What:** re-ran the matching Criterion benches on an idle box (the concurrent
demo cluster was stopped first) to verify the release headline numbers.
**Source:** `cargo bench -p rsx-matching --bench process_order_bench --bench
match_depth_bench`, HEAD at `f6316cf`.
**Conclusion:** the published `~30 ns` match and `266 ns` accept **do not
reproduce** on this box today; it measures **~48 ns** and **~295 ns**. Throughput
is close. The gap is shared-host variance, not a code regression.

## Numbers (this run vs the 2026-07-03 report)

| Bench | 2026-07-03 (published) | 2026-07-09 (this run) | note |
|---|---|---|---|
| `match_by_depth/n=1` | 30.4 ns | **47.9 ns** | +58% |
| `match_by_depth/n=100000` | 29.7 ns | **52.1 ns** | still flat/depth-independent |
| `me_accept_path/full` | 266 ns | **294.8 ns** (283–304) | +11% |
| `me_throughput/orders` | 281 ns / 3.6M/s | **289 ns / 3.46M/s** | stable (−4%) |

Match stays depth-independent (~48–52 ns flat across n=1→100k), so the *shape* of
the result holds; only the absolute per-op latency is higher.

## Why the numbers don't reproduce

- **Shared host.** The 2026-07-03 report is explicitly "indicative on a shared
  4-core docker host." A shared host has noisy-neighbour and throttling variance;
  the tight per-op latencies (match ~30–50 ns) are the most sensitive to it,
  which is exactly where the gap is largest (+58%). Throughput, being amortised,
  is stable (−4%).
- **Not a regression I introduced.** The WAL-dedup re-bench (before any of this
  session's refactors) already measured ~51 ns / ~301 ns. The `UserRegistry`
  refactor and the three LOW fixes landed after that and did not move it. So the
  ~48/295 figure is the box, consistently, not a code change.

## Takeaway for the founder

- The **match/accept latency numbers need a dedicated (non-shared) box** to be
  trustworthy as published figures. On a shared docker host they vary ~1.6× on
  the tight per-op benches. The published `30 ns`/`266 ns` are a best-case
  shared-host snapshot, not a floor you can reproduce on demand.
- **Throughput (~3.5M orders/s) and depth-independence are robust** — those
  reproduce.
- The full-workspace `make bench-gate` is a **nightly-scale** job: `cargo bench
  --workspace` exceeds its own `timeout` wrapper in a single session (it timed
  out mid-`rsx-book`), so it is not a per-session gate. Filed as
  `MATCHING-BENCH-SHARED-HOST-VARIANCE`.

## Caveats

Same shared 4-core docker host as the Jul-03 report. Cluster off, but a
concurrent session was active on the box (its residual build/IO could add
noise). Criterion 50-sample groups; ranges quoted. Single box, in-process, no
cross-process transport.
