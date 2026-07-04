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
- `match_by_type/fok_full` is a separate, STILL-open finding (bugs.md
  `FOK-AVAILABLE-LIQUIDITY-ON-SCAN`): FOK's `available_liquidity` pre-check
  is its own O(N-resting) full-book scan, ~296 µs at depth 10k, not touched by
  the occupancy-bitmap fix below.

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
| `match_by_type/fok_full` | in the quarantined 99-224 µs / 1 ms range | 296 µs | **not fixed** — separate bug (below) |

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
