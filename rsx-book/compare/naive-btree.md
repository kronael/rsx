# rsx-book vs naive BTreeMap baseline

Two separate, independently-built naive-BTreeMap comparisons exist for
rsx-book — noted here so a reader doesn't mistake one for a rerun of the
other.

## 1. `compare_all_bench.rs`'s `naive_btree` contender (this worktree)

`rsx-book/benches/compare_all_bench.rs`, module `naive_btree_impl`:
`BTreeMap<i64, VecDeque<i64>>` per side, one qty entry per level (matching
the abstract single-order-per-level shape every contender in the unified
harness uses). Implements the full `BenchBook` trait including
`match_touch` (a hand-written price-time-priority sweep across levels,
popping from each level's `VecDeque` front). Driven through the exact same
op stream as every other contender — see `compare/README.md` for the table.
This is the "obvious thing" floor inside the unified, automatic harness:
adding a future contender to that file is one `impl BenchBook` + one line
in `all_contenders()`.

## 2. `compare_naive_bench.rs` (built concurrently in a sibling worktree)

Commit `7c1c0e0` — **not part of this worktree's history** (a parallel
effort building the same idea independently; visible via `git show 7c1c0e0`
but not an ancestor of this branch). Its baseline is more detailed than
(1): `BTreeMap<i64, VecDeque<NaiveOrder>>` per side with a proper
`HashMap<order_id, (Side, i64)>` order locator (real per-order id tracking,
not a bare qty), benched through a shared `harness.rs` (core-pinning,
Criterion config, RNG seed) alongside `book_bench`/`deep_book_bench`, at
depths 100/1k/10k, across `match_clear`, `insert_cancel`, and `cancel`.

Its results are now on master in `reports/20260704_book-bench.md` (the
"rsx-book vs. the obvious baseline (BTreeMap)" section): **rsx-book 1.5-2x
faster on match/insert+cancel, 5.5-10x faster on cancel** (widening with
depth — the naive book pays a HashMap lookup + BTreeMap descent + VecDeque
scan per cancel; rsx-book pays a slab unlink). That deeper baseline tracks
real per-order ids and benches deep cancel, which is why it shows rsx-book
*winning* where the unified harness's leaner qty-only `naive_btree` (shallow
insert/cancel churn, no per-order locator) shows the naive tree a few ns
ahead — different naive designs, different op shapes, both honest.

## Why both exist

This worktree's task was to build one **unified, automatic** harness
(`compare_all_bench.rs`) covering rsx-book + hftbacktest + crates.io
crates + a naive baseline all through one trait and one op stream — the
naive contender there is intentionally simple (qty-only, no per-order id)
to keep the shared `BenchBook` abstraction uniform across very different
external crates. `compare_naive_bench.rs` (sibling worktree) is a deeper,
purpose-built naive-vs-rsx-book-only comparison with real per-order
tracking. Neither supersedes the other; a future merge should keep both —
one is the extensible cross-library harness, the other is the
higher-fidelity two-way baseline comparison the CEO audit asked for.
