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
- `insert_cancel_depth/1`, `/100` and all `match_by_type/*full` + `sweep`
  numbers (99–224 µs, 1 ms) are **QUARANTINED** — the depth-10k `iter_batched`
  fixture's alloc/drop bleeds into the timed region (BUGS.md
  MATCHING-BENCH-ORDERTYPE-FIXTURE). Do NOT cite them as per-order latency; use
  `match_depth`/`deep_flat_match`.
- Criterion closed-loop, quiet box, single run — re-run before quoting elsewhere.
