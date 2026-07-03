# rsx-book benchmark run — 2026-07-03 (Phase 1: Measure)

Consolidated, pinned, directly-comparable rsx-book microbenches under a
single shared harness. Phase 1 of the "cast treatment"
(`.ship/31-BOOK-MATCH-CAST-TREATMENT/PLAN.md`) — rsx-book's OWN numbers
only; no competitor baselines yet (that's Phase 2).

All benches route through `rsx-book/benches/harness.rs`: timed thread
pinned to **core 2**, `sample_size(50)` (matches the cast benches),
shared `SymbolConfig` (tick 1, lot 1 => raw units), and a fixed
fat-tailed Student-t book fixture `harness::build(n)` so every bench
measures against identically-constructed books. No bench re-rolls its
own pin/config/fixture — drift is how unfairness creeps in.

## Status — PARTIAL; benches need a fix round (2026-07-03)

Ran cluster-free (RSX tiles stopped, `sample_size 50`, pin core 2, shared
4-core docker host). Outcome: `book_bench` (micro-ops) completed;
**`deep_book_bench` PANICKED** — `assert!("slab exhausted")` at `slab.rs:39` —
so the depth curves, `match_by_depth`, and the deep flatness proof (the
headline) were **NOT captured**. Several `book_bench` micro-ops are also
unreliable. **The review gate caught real bench bugs — these are NOT a baseline
yet.** Bugs logged: `BOOK-BENCH-DEEP-PANIC`, `BOOK-BENCH-MICROOPS-OPTIMIZED`.

### Captured (indicative — trusted subset)
- `slab_free` **7.9 ns** · `compression_new` **12.8 ns** ·
  `price_to_index_bisection` **1.9 ns** · `recenter_lazy_per_access` **2.0 ns**
- `event_buffer_drain_100` **7.3 µs** · `recenter_10k_orders` **291 µs** ·
  `best_bid_scan_after_cancel` **51.7 µs** · `modify_order_price_change` **3.3 µs**

### Quarantined — do NOT cite
- **Optimized away (missing `black_box`):** `modify_order_qty_down` **0 ps**,
  `slab_alloc_bump` 285 ps, `slab_alloc_from_freelist` 735 ps,
  `compression_price_to_index_{near,far}` ~460 ps.
- **Mislabeled:** `match_single_fill` 5.0 µs — sweeps a 1k-ask book (~1k fills),
  NOT the single ~54 ns fill the label claims.
- **Artifact:** `insert_depth` inverted (n=1 → 38 µs, n=100k → 197 ns) —
  insert+cancel pair measurement, not a per-op depth curve.

### NOT captured (deep_book_bench panic)
`cancel_depth` (≥10k), `match_by_depth`, `deep_flat` 100k/1M/10M — including the
depth-independence headline. Blocked on `BOOK-BENCH-DEEP-PANIC`: harness
`build(n)` sizes the slab to `n+1024`, but `cancel_depth` refill churn (and the
1M/10M `deep_flat`) exhaust it. Fix the harness slab sizing + `black_box` the
micro-ops, then re-run. The detail tables below (PENDING) document what each
bench is meant to measure — retain for the fix round.


## What each bench measures

### `book_bench` — micro-ops (depth-free floors)

| Bench | Measures | p50 |
|---|---|---|
| `slab_alloc_bump` | Slab bump-path alloc (fresh arena) | PENDING |
| `slab_alloc_from_freelist` | Slab alloc+free on a warm free list | PENDING |
| `slab_free` | Slab free (return handle to free list) | PENDING |
| `compression_price_to_index_near` | `price_to_index`, near mid | PENDING |
| `compression_price_to_index_far` | `price_to_index`, far zone (sawtooth) | PENDING |
| `compression_new` | `CompressionMap::new` (build cost) | PENDING |
| `price_to_index_bisection` | `price_to_index` swept across a band | PENDING |
| `match_single_fill` | one IOC fill vs 1k-ask book (the '54 ns fill') | PENDING |
| `recenter_10k_orders` | `trigger_recenter` + full `migrate_batch`, 10k book | PENDING |
| `recenter_lazy_per_access` | lazy per-access `resolve_level` during migration | PENDING |
| `event_buffer_drain_100` | sweep ~100 fills + drain the event buffer | PENDING |
| `best_bid_scan_after_cancel` | BBO re-derivation: `scan_next_bid` after touch cancel | PENDING |
| `modify_order_price_change` | `modify_order_price` (cancel+reinsert at new tick) | PENDING |
| `modify_order_qty_down` | `modify_order_qty_down` (in-place shrink) | PENDING |

### `deep_book_bench` — latency vs depth + by order type

Depth curves seed the book to N via `harness::build(n)` (fat-tailed
Student-t), then measure a net-neutral op so depth stays ~N across
criterion iterations. Throughput reported as ops/s (Elements(1)).

**`insert_depth`** — insert+cancel round-trip vs resting depth:

| Depth | p50 | ops/s |
|---|---|---|
| 1 | PENDING | PENDING |
| 100 | PENDING | PENDING |
| 1 000 | PENDING | PENDING |
| 10 000 | PENDING | PENDING |
| 100 000 | PENDING | PENDING |

**`cancel_depth`** — pure `cancel_order` vs depth (timed batches, pool
refilled untimed; pool clamped small vs N so "depth N" holds):

| Depth | p50 | ops/s |
|---|---|---|
| 1 | PENDING | PENDING |
| 100 | PENDING | PENDING |
| 1 000 | PENDING | PENDING |
| 10 000 | PENDING | PENDING |
| 100 000 | PENDING | PENDING |

**`match_depth`** — IOC taker (fat-tailed size) sweep + replenish vs
depth:

| Depth | p50 | matches/s |
|---|---|---|
| 1 | PENDING | PENDING |
| 100 | PENDING | PENDING |
| 1 000 | PENDING | PENDING |
| 10 000 | PENDING | PENDING |
| 100 000 | PENDING | PENDING |

**`match_by_type`** — match latency at fixed depth 10 000, fresh book
per type (all net-neutral):

| Type | Path exercised | p50 |
|---|---|---|
| `gtc_full_cross` | GTC that fully crosses (no resting remainder) | PENDING |
| `ioc_full` | IOC that fully fills | PENDING |
| `fok_full` | FOK (availability check + fill) | PENDING |
| `post_only_reject` | post-only cross guard (rejected, book unchanged) | PENDING |
| `sweep_10_levels` | taker consuming ~10 levels | PENDING |

**`deep_flat_insert` / `deep_flat_match`** — the flat-latency proof:
insert / match at 100k, 1M, 10M resting. Expected FLAT (match is
O(consumed), not O(resting); prior run 2026-05-30 measured match ~52 ns
flat across all three). RAM-bound: `OrderSlot` = 128 B, 10M ~ 1.3 GB.

| Depth | insert p50 | match p50 |
|---|---|---|
| 100 000 | PENDING | PENDING |
| 1 000 000 | PENDING | PENDING |
| 10 000 000 | PENDING | PENDING |

## Prior measured numbers (2026-05-30, NOT re-run here)

Carried from `reports/20260530_component-benches.md` /
`20260530_load-curves.md` for reference — DIFFERENT harness (no shared
pin/config), so not strictly comparable to the tables above once
re-run:

- `match_single_fill` (a.k.a. single fill): **~54 ns** p50.
- deep-book match: **~52 ns FLAT** at 100k / 1M / 10M resting.
- deep-book insert: **~190–215 ns** flat.

## Caveats (honesty)

- **Single box, in-process microbench.** No UDP / WS / kernel — these
  are the isolated data-structure floors, not wire-to-wire latency.
- **Pinned to core 2** via `harness::pin()`; results assume the box is
  otherwise idle. This run was NOT idle (cluster up) → PENDING.
- **`insert_depth` measures insert+cancel** as a pair (net-neutral);
  insert and cancel are inverse ops, so a single-op net-neutral
  measurement of insert necessarily includes the paired cancel.
  `cancel_depth` isolates cancel via untimed refill.
- **`cancel_depth` pool** adds a small number of orders on top of depth
  N (clamped to ≤4096); at N=1 the pool dominates, so the smallest-depth
  cancel point is not a true depth-1 measurement.
- **`sweep_10_levels` replenishes across 10 levels** to avoid collapsing
  the book into one price over iterations; other match types replenish
  at the touch (single level) and may drift the near-touch distribution
  slightly over a long sample.
- Fat-tailed seed is deterministic (fixed xorshift seed per depth) →
  reproducible, but synthetic (Student-t ν=3, not a real book snapshot).
- Every figure cites its bench name; carried-over 05-30 numbers are
  marked "not re-run". Run it yourself with the commands above.
