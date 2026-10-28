# rsx-book vs naive BTreeMap baseline

Two benches compare rsx-book with a plain `BTreeMap` book. They answer
different questions and their results do not combine.

## 1. `compare_all_bench.rs`, contender `naive_btree`

`rsx-book/benches/compare_all_bench.rs`, module `naive_btree_impl`:
`BTreeMap<i64, VecDeque<i64>>` per side, one qty entry per level (matching
the abstract single-order-per-level shape every contender in the unified
harness uses). It implements the full `BenchBook` trait including
`match_touch` (a hand-written price-time-priority sweep across levels,
popping from each level's `VecDeque` front). It runs the same op stream as
every other contender; the stream cycles one level at a time, so it measures
level churn, not resting depth. See `compare/README.md` for the capability
and fairness tables.

This is the "obvious thing" floor inside the unified harness. A new contender
is one `impl BenchBook` plus one line in `all_contenders()`.

## 2. `compare_naive_bench.rs`

`rsx-book/benches/compare_naive_bench.rs` builds a more detailed baseline:
`BTreeMap<i64, VecDeque<NaiveOrder>>` per side with a
`HashMap<order_id, (Side, i64)>` order locator, so it tracks real per-order
ids. It runs through the shared `harness.rs` (core pinning, Criterion
configuration, RNG seed) next to `book_bench` and `deep_book_bench`, at
resting depths 100, 1,000 and 10,000, in the groups `match_clear`,
`insert_cancel` and `cancel`.

Its results are in `reports/20260704_book-bench.md`, section "rsx-book vs.
the obvious baseline (BTreeMap)": rsx-book 1.5-2x faster on match and
insert+cancel, 5.5-10x faster on cancel, widening with depth. The naive book
pays a HashMap lookup, a BTreeMap descent and a VecDeque scan per cancel;
rsx-book pays a slab unlink.

## Why the two disagree

The unified harness's qty-only `naive_btree` has no per-order locator and runs
shallow insert and cancel churn, so a bare tree wins that probe by a small
constant. The deeper baseline tracks per-order ids and benches deep cancel,
where rsx-book wins. They are different naive designs on different op shapes,
and both results stand.
