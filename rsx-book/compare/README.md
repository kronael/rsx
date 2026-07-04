# rsx-book cross-match index

Every comparison rsx-book has against an external order-book implementation
or published number. Two kinds, never blurred:

- **Benched** — real code, same box, same Criterion harness
  (`rsx-book/benches/compare_all_bench.rs`), same synthetic op stream, run
  in this pass. Numbers below are copy-pasted from the actual run.
- **Cited** — someone else's published number, their own harness, their own
  hardware. Never re-run here. Always flagged NOT apples-to-apples, with
  the specific reason.

Source plan: `.ship/34-COMPARE-RESEARCH/PLAN.md`. The general rsx-book
Criterion suite (match latency depth-invariant at ~60-65ns; full
primitive/bulk tables; the naive-BTreeMap two-way comparison the CEO audit
asked for) lives in `reports/20260704_book-bench.md`. See
`compare/naive-btree.md` for how the `naive_btree` contender benched below
relates to that report's deeper `compare_naive_bench` baseline.

## Run header

- **Box:** AMD Ryzen 9 5950X, 4-vCPU slice (matches the HW in
  `reports/20260704_book-bench.md`).
- **Date/commit:** 2026-07-04, re-run on **fixed master** (`efaa620`) — on
  top of the occupancy-bitmap fix (`da9a2b4`) and the FOK fix (`40f6bf8`).
  **This supersedes the first run of this harness**, which was measured in
  a worktree branched *before* `da9a2b4` and therefore benchmarked the old
  O(slots) `scan_next` bug — that run's "rsx-book loses level-touch by ~19x"
  verdict was an artifact of stale code (see the note under Honest verdict).
- **Harness:** `rsx-book/benches/compare_all_bench.rs`, one Criterion
  binary, `cargo bench -p rsx-book --bench compare_all_bench`.
- **Method:** one trait (`BenchBook`) — insert / reduce / cancel a resting
  qty at an abstract price level, read best price, and an optional
  order-level `match_touch`. Every contender below is one `impl BenchBook`.
  Adding a future contender = one impl + one line in `all_contenders()`.
  Contenders that can't do an op (wrong capability, e.g. hftbacktest has no
  matching) are excluded from that op's bench group by construction and
  print an explicit `N/A` line — never a faked number.
- **Op stream:** deterministic, net-neutral (every level starts AND ends
  empty), shared verbatim across every contender — see
  `gen_full_cycle_ops`/`gen_insert_cancel_ops` in the bench file.

## Capability table

| contender | insert/cancel | reduce | best-read | match (real order-level FIFO) |
|---|---|---|---|---|
| rsx-book (real) | yes | yes | yes | yes |
| naive BTreeMap (built here) | yes | yes | yes | yes |
| hftbacktest `BTreeMarketDepth` | yes | yes | yes | **N/A** — L2-aggregated, no FIFO |
| hftbacktest `HashMapMarketDepth` | yes | yes | yes | **N/A** — L2-aggregated, no FIFO |
| lob (rafalpiotrowski/lob-rs) | yes | **N/A** — no amend API | **N/A** — not exposed | yes |
| orderbook (inv2004) | evaluated, **crashes** | — | — | — |
| orderbook-rs (joaquinbejar) | not integrated (scope) | — | — | — |

## Benched numbers (fixed master, `efaa620`)

All medians (Criterion point estimate). **rsx-book is flat across depth
20→1000 on every op** — the occupancy bitmap makes level ops O(depth=3),
not O(slots). For context, the pre-fix column is the same harness run on the
stale worktree (before `da9a2b4`).

### `level_touch_full` — insert, 3×reduce, cancel (mixed avg ns/op)

| levels/side | rsx_book | rsx (pre-fix) | naive_btree | hft_btree | hft_hashmap | lob |
|---|---|---|---|---|---|---|
| 20   | **15.7 ns** | 298.8 ns | 10.2 ns | 18.4 ns | 23.1 ns | N/A |
| 100  | **15.5 ns** | 293.3 ns | 10.8 ns | 18.8 ns | 23.6 ns | N/A |
| 1000 | **16.2 ns** | 261.4 ns | 10.5 ns | 19.8 ns | 23.9 ns | N/A |

### `level_insert_cancel` — insert + cancel only (avg ns/op)

| levels | rsx_book | rsx (pre-fix) | naive_btree | hft_btree | hft_hashmap | lob |
|---|---|---|---|---|---|---|
| 20   | **33.5 ns** | 766.1 ns | 20.0 ns | 24.5 ns | 22.4 ns | 97.9 ns |
| 100  | **33.2 ns** | 744.1 ns | 19.4 ns | 23.4 ns | 23.3 ns | 210.7 ns |
| 1000 | **31.4 ns** | 615.0 ns | 19.9 ns | 23.9 ns | 23.8 ns | **1219 ns** |

### `best_read` — single best-price read (ns)

| rsx_book | naive_btree | hft_btree | hft_hashmap | lob |
|---|---|---|---|---|
| **1.47 ns** | 2.69 ns | 1.77 ns | 1.49 ns | N/A (no public API) |

### `match_touch` — one marketable 1-lot fill against deep resting liquidity (ns)

| rsx_book | naive_btree | hft_btree | hft_hashmap | lob |
|---|---|---|---|---|
| **30.8 ns** | 22.1 ns | N/A (no matching) | N/A (no matching) | 81.0 ns |

## Honest verdict — read this before quoting any number above

**After the occupancy-bitmap fix, rsx-book is competitive-to-winning across
every op, and — the property that matters — flat with book depth.** Level
ops don't grow 20→1000 (15.5-16.2 ns, 31-33 ns), matching is depth-invariant
at ~60-65 ns out to 10M resting orders (`reports/20260704_book-bench.md`),
and cancel is O(1) by slab handle. Specifics:

- **`level_touch_full`:** rsx-book 15.5-16.2 ns — **beats both hftbacktest
  variants** (18-24 ns) and sits ~1.5x behind a bare BTreeMap (10-11 ns).
- **`best_read`:** rsx-book 1.47 ns — **fastest of the field** (tied with
  hftbacktest_hashmap 1.49; ahead of hft_btree 1.77 and naive 2.69).
- **`match_touch`** (the real order-level job an ME does): rsx-book 30.8 ns —
  **beats lob 81 ns**; naive_btree's 22 ns is close but this probe never
  spans levels, so it doesn't exercise the depth-invariance that separates
  them at scale, and hftbacktest can't match at all.
- **`level_insert_cancel`:** rsx-book 31-33 ns, flat — **beats lob
  decisively** (98 ns → 1.22 µs, growing 12x across depth) and trails
  naive/hftbacktest (20-24 ns) by a small constant.

**Where a bare BTreeMap is faster, and why that's the honest trade, not a
loss.** naive_btree wins pure level churn (10-20 ns vs rsx's 15-33 ns)
because it skips machinery rsx-book carries on every op: compression-map
price→slot lookup (~2 ns), slab alloc/free, occupancy bitmap set/clear, and
live best-bid/ask maintenance. That machinery is exactly what buys
depth-invariant matching to 10M orders, O(1) cancel-by-handle, and flat
level ops — none of which the naive tree provides (its cancel pays a
HashMap lookup + tree descent + VecDeque scan that *grows* with depth; see
the deeper `compare_naive_bench` in `reports/20260704_book-bench.md`, where
rsx-book is 5.5-10x faster on deep cancel). A few ns of constant overhead on
level churn is the price of the properties an ME actually needs. The
`match_touch` and depth-invariant match numbers are rsx-book's job; the
level-touch microbench is an L2-depth-shaped op class where a dumb tree's
lack of overhead shows, and rsx-book is now within a small constant of it.

> **Superseded verdict (stale-worktree artifact).** The first run of this
> harness reported "rsx-book loses level-touch by ~19x (261-766 ns), traced
> to a linear `scan_next` scan." That analysis was *correct about the
> cause* but was measured on a worktree branched before `da9a2b4`: the
> linear scan it fingered had already been replaced by the occupancy bitmap
> on master. Re-run on the fixed code, `level_touch_full` dropped
> 298→15.7 ns and `level_insert_cancel` 766→33.5 ns. The lesson: pin a
> comparison harness to the branch that has the fix.

## What this does NOT show

- It does not show rsx-book beats a bare BTreeMap on raw level churn — it
  doesn't, by a small constant, and that's the documented cost of the
  compression/slab/occupancy machinery.
- It does not show hftbacktest or lob are "bad" — they solve a different
  problem (L2 feed reconstruction; a from-scratch order-level book) and
  hftbacktest is competitive on level ops. hftbacktest simply doesn't match
  (by design), and lob's insert/cancel grows with depth.
- It is not a claim about production system latency — everything here is a
  single-core, in-process, closed-loop Criterion microbenchmark, same
  caveat every other bench in this repo carries. This run was on a lightly
  loaded box; re-run before quoting an exact ns externally, but the
  cross-contender *relative* picture is stable.

## Fairness bar per row

| contender | same box | same harness | same op stream | language | verdict |
|---|---|---|---|---|---|
| naive_btree | yes | yes | yes | Rust | benched, fair |
| hftbacktest (btree/hashmap) | yes | yes | yes (level-touch only) | Rust | benched, fair — match N/A by construction |
| lob | yes | yes | yes (no reduce/best) | Rust | benched, fair — partial API |
| orderbook (inv2004) | — | — | — | Rust | not benched — crashes, see `compare/orderbook-inv2004.md` |
| orderbook-rs (joaquinbejar) | — | — | — | Rust | not benched — scope, see `compare/orderbook-rs.md` |
| itch-order-book | no (2012 HW) | no (their own harness) | no | C++ | **cited only**, see `compare/cross-language-cited.md` |
| exchange-core | no (2010 HW, JVM) | no | no | Java | **cited only** |
| liquibook | no (unstated HW) | no | no | C++ | **cited only** |

See per-contender files in this directory for the full detail behind each
row: `hftbacktest.md`, `lob.md`, `naive-btree.md`, `orderbook-inv2004.md`,
`orderbook-rs.md`, `cross-language-cited.md`.
