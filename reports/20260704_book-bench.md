# rsx-book benchmark — 2026-07-04 (clean quiet-box run)

**What:** the full `rsx-book` Criterion suite (`book_bench` + `deep_book_bench`),
the matching/orderbook library in isolation. **Box:** AMD Ryzen 9 5950X (4-vCPU
slice), single core, 1 thread/core. **Method:** the RSX cluster was STOPPED
first (its ME+Risk busy-spin at ~90% CPU otherwise poisons the numbers — a
contaminated earlier run showed +757% noise). Criterion, closed-loop, n=50-100
per case. Commit `f45e0a0`. Source: `cargo bench -p rsx-book`.

## Headline — matching is O(1) in book depth
Matching a marketable order stays **~65 ns whether the book holds 100 thousand
or 10 MILLION resting orders.** Best case **28 ns** at depth 1. Depth-invariant
because the compression map + slab arena make level lookup and order pop
constant-time, not a tree walk.

| resting orders in book | match latency (median) |
|---|---|
| 1               | **28.1 ns** |
| 100             | 60.0 ns |
| 1,000           | 61.0 ns |
| 10,000          | 60.2 ns |
| 100,000         | 64.5 ns |
| 1,000,000       | 66.3 ns |
| **10,000,000**  | **65.5 ns** |

(`match_depth/*` + `deep_flat_match/*` — the same op across depths.)

## Primitives (per-op, single core)
| op | median |
|---|---|
| slab alloc (bump) | **556 ps** |
| slab alloc (freelist) | 1.44 ns |
| slab free | 8.30 ns |
| compression price→index (near) | 1.91 ns |
| compression price→index (far) | 2.18 ns |
| price→index bisection | 1.99 ns |
| compression map build | 12.7 ns |
| lazy recenter (per access) | 1.95 ns |
| modify order qty-down | 2.12 ns |
| post-only reject | 5.96 ns |
| deep flat insert (100k–10M book) | 238–260 ns |

## Bulk / amortized
| op | median | note |
|---|---|---|
| insert+cancel, depth 1k–100k | 160–350 ns | amortized per pair |
| cancel, depth 1k–100k | 15–170 ns | |
| recenter 10k orders | 308 µs | bulk compression-map rebuild |
| best-bid scan after cancel | 50 µs | the known O(n) scan (BUGS.md) |

## Caveats (honest)
- **Lab microbenchmark, not system TPS.** Single core, in-process, no I/O, no
  network — this is the *algorithm*, not the exchange round-trip (that's the
  transport-bound ~1.1 ms cross-process figure, a different story).
- Criterion closed-loop, quiet box, single run — re-run before quoting elsewhere.
- `match_by_type/fok_full` was a separate finding (bugs.md
  `FOK-AVAILABLE-LIQUIDITY-ON-SCAN`): FOK's old pre-check was its own
  O(N-resting) full-book scan, ~296 µs at depth 10k, not touched by the
  occupancy-bitmap fix. **Also fixed 2026-07-04** — see the FOK section below;
  now ~118 ns.

## Post-scan-fix — occupancy bitmap (2026-07-04, commit `da9a2b4`)

**The `MATCHING-BENCH-ORDERTYPE-FIXTURE` finding above was WRONG about root
cause.** The 32-224 µs `match_by_type`/`insert_cancel_depth` numbers were NOT
fixture alloc/drop bleed — `post_only_reject` ran on the exact same depth-10k
fixture and measured 5.96 ns, which is only possible if the fixture itself was
cheap. The real cause: `scan_next_bid`/`scan_next_ask` did an O(compression-
slots) linear scan (~100k slots) whenever a price level actually emptied
(match-that-clears-the-touch, or cancel-that-empties-a-level) — `post_only_
reject` never clears a level, so it never paid the scan; the `match_depth`/
`deep_flat_match` numbers above dodged it too, because their taker is always
replenished onto the SAME touch level before it can go empty. Every op whose
fixture design clears a level (`match_by_type`'s `taker_fill`, cancel-driven
depth sweeps) hit the scan and paid 30-1000x the true match cost.

Fixed by a hierarchical occupancy bitmap (`rsx-book/src/occupancy.rs`): 1
bit/compression-slot + a u64 summary tree, `find_next`/`find_prev` via
trailing/leading-zeros, O(depth=3) instead of O(slots). Re-measured 2026-07-04
on the same quiet box, same commit family:

| bench | before `da9a2b4` | after | speedup |
|---|---:|---:|---:|
| `match_ioc_vs_1k_asks` (clears touch level) | 4.37 µs | **145 ns** | 30x |
| `match_by_type/ioc_full` | ~80 µs | **79.4 ns** | ~1000x |
| `match_by_type/gtc_full_cross` | ~80 µs | **79.7 ns** | ~1000x |
| `match_by_type/sweep_10_levels` | ~1 ms | **700 ns** | ~1400x |
| `match_by_type/post_only_reject` | 5.96 ns (unaffected, never clears) | **6.17 ns** | — |
| `match_by_type/fok_full` | in the quarantined 99-224 µs / 1 ms range | 296 µs → **118 ns** | fixed separately (FOK section below) |

Happy path is unaffected, confirming it was never the fixture:
`match_depth/1000` 61.0 ns → **61.3 ns**, `match_depth/10000` 60.2 ns →
**63.5 ns** — same ~60-65 ns band as the original headline table above,
within run-to-run noise.

**Budget claim, corrected:** the exchange's <500 ns ME-match budget was
previously met only on the path that never clears a resting level. Any real
match that empties the touch (a common case — the whole point of matching is
to consume liquidity) cost 32-224 µs, **200x over budget**. It is now
genuinely met on both paths: 60-65 ns when the touch survives, 145 ns when it
clears.

## rsx-book vs. the obvious baseline (`BTreeMap<price, VecDeque<order>>`)

Per the CEO-audit "so what, vs the obvious thing" ask (`.ship/34-COMPARE-
RESEARCH/PLAN.md`): a textbook order book — `BTreeMap<i64, VecDeque<Order>>`
per side, `HashMap<order_id, (side, price)>` to locate an order for cancel
(linear scan within its level's VecDeque — no slab, no compression map, no
occupancy bitmap). Same Criterion harness (`rsx-book/benches/harness.rs`),
same box, same RNG seed per depth so both books hold statistically-identical
content. Source: `rsx-book/benches/compare_naive_bench.rs`, `cargo bench -p
rsx-book --bench compare_naive_bench`.

| op | depth | rsx-book | naive BTreeMap | speedup |
|---|---:|---:|---:|---:|
| match, clears touch level | 100 | 72.1 ns | 106.5 ns | 1.5x |
| match, clears touch level | 1,000 | 71.7 ns | 110.2 ns | 1.5x |
| match, clears touch level | 10,000 | 71.6 ns | 117.8 ns | 1.6x |
| insert + cancel (pair) | 100 | 160.0 ns | 241.7 ns | 1.5x |
| insert + cancel (pair) | 1,000 | 162.2 ns | 286.8 ns | 1.8x |
| insert + cancel (pair) | 10,000 | 171.1 ns | 349.1 ns | 2.0x |
| cancel | 100 | 18.4 ns | 101.0 ns | 5.5x |
| cancel | 1,000 | 17.8 ns | 146.4 ns | 8.2x |
| cancel | 10,000 | 17.9 ns | 178.4 ns | 10.0x |

**Honest reading:** BTreeMap was never O(book-size) for this — tree removal
and next-best lookup are both O(log n), so it never had rsx-book's pre-fix
O(slots) bug; the gap here is constant-factor (slab handle vs. hash lookup +
tree traversal + heap alloc/dealloc per level), not asymptotic. The gap is
narrowest on match (1.5-1.6x, both O(1)-ish at these depths) and widest on
cancel (5.5x→10x, growing with depth) — rsx-book's cancel is a pure slab
unlink (O(1), no tree, no hash lookup), while the naive cancel pays a
HashMap lookup plus a BTreeMap tree descent plus a VecDeque scan, and that
tree descent cost grows with depth. `insert+cancel` sits between the two
(1.5x→2.0x, growing) since it's dominated by the same insert-side BTreeMap
entry-or-default cost at both ends.

## FOK fill-or-kill — no map, just "try to match it" (2026-07-04)

FOK (fill-or-kill) must fill the whole order immediately or reject it. The
old check (`available_liquidity`) answered "is there enough crossable
liquidity?" with a SEPARATE O(N) pass: it iterated all ~100k active levels
AND every resting order on each, summing crossable qty, *before* matching.
At depth 10k that pre-check was the entire cost — `fok_full` sat at ~296 µs
while every other order type was 60-145 ns.

The fix is not a new structure (no histogram, no per-side liquidity index).
FOK is just "try to match it, take it or don't", so `can_fill_fully` walks
only the *crossing* levels in price order — the same traversal a real match
performs, via the book's existing best-level index (`price_asc` +
occupancy) — and sums each level's already-maintained `total_qty`, stopping
the instant the running total reaches the order size. A whole price level
shares one price, so it either crosses or it doesn't; `total_qty` counts it
exactly with no per-order walk. Complexity: O(levels crossed, early-exit)
instead of O(slots + orders).

| bench | before | after | speedup |
|---|---:|---:|---:|
| `match_by_type/fok_full` (depth 10k) | 296 µs | **~118 ns** | ~2500x (−99.95%) |

Correctness is pinned by `rsx-book/tests/fok_liquidity_test.rs`: 3000 FOK
probes over multi-zone random flow, each compared to an independent
brute-force sum over every resting order — the fast walk must fail (no
fills) exactly when brute-force liquidity < order size, and fully fill
otherwise. Caveat: the ~118 ns figure is from a lightly-contended box (a
parallel bench was running); the −99.95% magnitude is unambiguous, but
re-run on a fully quiet box before quoting the exact ns elsewhere.

## Distribution robustness (2026-07-04, `distribution_bench.rs`)

Does the occupancy bitmap hold O(depth) regardless of how orders are laid out?
Next-best / match-that-clears / cancel measured under dense (packed zone 0),
sparse (gaps across zones), and concentrated (single wall) shapes, at depth 1k
and 10k. Quiet box, single core, medians (ns):

| op | dense 1k / 10k | sparse 1k / 10k | concentrated 1k / 10k |
|---|---|---|---|
| next_best (pure scan) | 21.7 / 21.8 | 29.0 / 26.6 | 25.3 / 25.0 |
| match_clears (clear touch + scan) | 73.8 / 71.9 | 81.2 / 77.8 | 77.9 / 77.8 |
| cancel_touch (cancel best → scan) | 41.8 / 43.3 | 46.0 / 48.7 | 42.9 / 43.2 |
| cancel_deep (far level, no scan) | 26.6 / 26.8 | 26.3 / 25.8 | 26.2 / 25.9 |

**Verdict: O(depth), not O(slots), in every shape.** Every op is flat 1k→10k
(10× the orders, ~same latency). Sparse adds only a small *additive* constant to
the scan-bearing ops (+5-7 ns) from skipping empty summary words — additive, not
proportional to the gap, the O(depth) signature. `cancel_deep` is a dead-flat
~26 ns baseline (pure slab unlink + O(1) bit clear), so the scan is the only
distribution-sensitive part and it stays bounded. No shape degrades.

## Tail latency (2026-07-04, `tail_bench.rs`)

Closes the last CEO-eval gap: every figure above is a Criterion median
(p50). This section measures p50/p99/p99.9 on the hot ops, with special
attention to `match_clears` — the level-clearing path the occupancy
bitmap fixed (see the "Post-scan-fix" section above). Quiet box (RSX
cluster stopped), single core (pinned, same `harness::pin()` as every
other bench in this suite), dense-shape book (contiguous levels behind
a 1-order touch), depths 1k and 10k. Source: `rsx-book/benches/
tail_bench.rs`. Run: `cargo bench -p rsx-book --bench tail_bench`.

### Methodology — and why a naive per-op timer would lie here

These ops run in 25-100 ns. `Instant::now()` on this box (vDSO
`clock_gettime(CLOCK_MONOTONIC)`) costs ~20-30 ns per call — a large
fraction of the op — so timing every single op individually mostly
measures the timer, not the op. Measured directly (back-to-back
`Instant::now()` calls, nothing timed in between, n=100,000):

| | mean | p50 | p99 | p99.9 | max |
|---|---:|---:|---:|---:|---:|
| **timer floor** | 24.0 ns | 20.0 ns | 31.0 ns | 31.0 ns | 114,485 ns |

The floor's own max (114 µs) is OS scheduling jitter on an otherwise
idle box — a reminder that any single huge `max` reading below could be
noise, not the algorithm, and is called out per-op rather than
overclaimed.

Given that floor, this harness reports two numbers per op, both printed
by the harness, only one of them quoted as authoritative:

1. **Batch-amortized (the number in the table below).** Batches of
   `BATCH=64` consecutive ops are timed as one span and divided by 64,
   so timer overhead contributes <1 ns to the per-op figure. 2,000,000
   ops per op/depth combo -> 31,250 batch samples, so p99.9 has ~31
   points above it (a real quantile, not a handful of outliers).
2. **Single-op raw timer (context only, NOT quoted as fact).** Every
   op timed individually, n=100,000. Included in the raw harness
   output so the reader can see it sits close to the timer floor (p50
   50-90 ns vs. the 20-31 ns floor) — i.e. its distribution is
   partially timer noise, and its p99/p99.9 should not be read as the
   op's true tail.

Both runs are preceded by 20,000 discarded warmup iterations (cache /
branch-predictor warmup) and every op result is passed through
`std::hint::black_box` so the compiler cannot elide it. Single run,
quiet box, commit `f45e0a0`+`tail_bench.rs` — re-run before quoting
elsewhere, per the standing caveat on every table in this report.

### Results (batch-amortized, ns/op)

| op | depth | mean | p50 | p99 | p99.9 | max |
|---|---:|---:|---:|---:|---:|---:|
| match (partial fill, touch survives) | 1,000 | 29.4 | 28.3 | 35.7 | 154.8 | 1,949.9 |
| match (partial fill, touch survives) | 10,000 | 29.0 | 28.2 | 41.3 | 118.5 | 266.9 |
| **match_clears** (empties touch → occupancy scan) | 1,000 | 71.1 | 70.5 | 76.5 | 175.2 | 1,289.4 |
| **match_clears** (empties touch → occupancy scan) | 10,000 | 70.2 | 68.2 | 109.4 | 216.5 | 542.3 |
| cancel_touch (cancel best → scan) | 1,000 | 46.4 | 43.2 | 90.0 | 175.6 | 1,406.1 |
| cancel_touch (cancel best → scan) | 10,000 | 40.8 | 39.6 | 74.4 | 173.3 | 583.1 |
| cancel_deep (far level, no scan) | 1,000 | 26.4 | 26.1 | 28.0 | 89.2 | 232.9 |
| cancel_deep (far level, no scan) | 10,000 | 28.0 | 26.8 | 40.5 | 97.8 | 303.2 |

### Reading it

**`match_clears` has a tight tail, not a fat one.** p99/p50 is
1.08-1.60× (76.5/70.5 at depth 1k, 109.4/68.2 at depth 10k) — nowhere
near the 30-1000× blowups the pre-fix O(slots) scan produced (see the
"Post-scan-fix" section: 4.37 µs, ~80 µs, ~1 ms before). The
occupancy-bitmap find (O(depth=3) trailing/leading-zeros) does not
have a data-dependent slow path at these depths; the p99.9 bump
(~175-217 ns, ~2.5-3× p50) is consistent with occasional branch
mispredicts / cache misses on the summary-word walk, not an
asymptotic blowup, and stays flat 1k→10k (matching the O(depth)
verdict from the distribution-robustness section above).

**`match` (happy path, touch survives) is the tightest of the four** —
p99/p50 ~1.1-1.5×, as expected: no scan, no bitmap walk, just slab pop
+ level update.

**`cancel_touch` has the widest p99/p99.9 spread of the four**
(p99/p50 up to 2.1×, p99.9/p50 up to 4.4×) — it does two things per
op (insert + cancel-that-empties-and-scans), so it accumulates two
sources of jitter instead of one; `cancel_deep`'s single-op, no-scan
baseline is correspondingly the tightest apart from `match`.

**Caveats.** Single run, quiet box, dense shape only (the
distribution-robustness section above already covers sparse/
concentrated — this section is about the tail, not the shape). The
single-op numbers exist in the raw harness output for context but are
NOT reported here as fact — they sit too close to the ~20-30 ns timer
floor to trust their tail. `max` columns include rare multi-hundred-ns
to ~2 µs batch averages, i.e. one op inside that batch of 64 likely
stalled for tens of µs (OS scheduling, not algorithm) — consistent
with the timer floor's own 114 µs max on an idle box. Re-run before
quoting an exact ns elsewhere.
