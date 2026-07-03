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

## >>> NUMBERS PENDING — cluster was up during this run <<<

The RSX cluster was running when this bench pass was attempted, so
**no p50s were recorded** (recording contended numbers would be
dishonest). The operator should re-run with the cluster stopped and
fill the tables below.

Evidence (2026-07-03, `ps` + `/proc/loadavg`, 4-core box):

```
pid       %cpu  comm
1874120   18.9  rsx-matching
1861580    8.4  rsx-recorder
1861312    3.7  rsx-mark
1861565    2.3  rsx-marketdata
1861448    0.9  rsx-gateway
3534114    0.0  rsx-risk        (busy-spin tile; sampled low this instant)
loadavg: 2.83 4.56 5.51  (4 cores)
```

`rsx-risk` and `rsx-matching` are busy-spin tiles; on a 4-core box they
contend cores 2/3 — exactly the cores the harness pins to. Load average
2.83/4 (and 4.56 / 5.51 over 5 / 15 min) confirms sustained load.

### Re-run (cluster-free)

```bash
./rsx-playground/playground stop-all      # or kill the rsx-* PIDs
pgrep -af 'rsx-risk|rsx-matching|market_maker'   # confirm empty
cargo bench -p rsx-book --bench book_bench
cargo bench -p rsx-book --bench deep_book_bench   # 10M depth ~1.3 GB RAM
```

Then transcribe the criterion p50 (median) into the tables below and
drop this PENDING banner.

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
