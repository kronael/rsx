# rsx-book vs hftbacktest (nkaz001/hftbacktest, Rust)

Bench: `rsx-book/benches/compare_all_bench.rs` (contenders `hftbacktest_btree`,
`hftbacktest_hashmap`). Run: `cargo bench -p rsx-book --bench compare_all_bench`.
Box: same machine, same run as every other row in this directory — see
`compare/README.md` for the shared run header (date, HW, commit).

## What hftbacktest's `depth` module is

`hftbacktest::depth` (`BTreeMarketDepth`, `HashMapMarketDepth`,
`ROIVectorMarketDepth`, `FusedHashMapMarketDepth`) is an **L2-aggregated**
price-level book: one `f64` qty per price tick, via
`L2MarketDepth::update_bid_depth` / `update_ask_depth` / `clear_depth`. There
is **no order-id, no FIFO queue, no fill/match generation** — it exists to
reconstruct a shadow depth view from an exchange's L2 diff feed for
backtesting, not to run a matching engine.

This bench uses `BTreeMarketDepth` and `HashMapMarketDepth` (the tree-based
and hash-based impls) as a scoped **dev-dependency only** of `rsx-book`
(`default-features = false, features = ["backtest"]` — skips hftbacktest's
`live` feature entirely: no tokio, no iceoryx2, no chrono). It is never a
dependency of rsx-book's own library code.

## What's fairly compared

Only the **level-touch** op class both sides genuinely share:

| op | rsx-book | hftbacktest |
|---|---|---|
| insert a level | `insert_resting` (one order per level) | `update_bid/ask_depth(px, qty, ts)` |
| reduce a level's qty | `modify_order_qty_down` | `update_bid/ask_depth(px, smaller_qty, ts)` |
| cancel a level | `cancel_order` | `update_bid/ask_depth(px, 0.0, ts)` |
| read best price | `book.best_bid_tick` field | `depth.best_bid_tick` field |

**NOT compared:** rsx-book's `match_*` benches (order-level FIFO walk + fill
generation) have no hftbacktest counterpart — hftbacktest's depth doesn't
match orders, it just replays quantities. `match_touch` in the unified
harness reports `N/A` for both `hftbacktest_btree` and `hftbacktest_hashmap`
by construction (see `compare/README.md`'s capability table).

## Same synthetic stream, same harness

Both sides are driven through the identical deterministic op stream
(`gen_full_cycle_ops` / `gen_insert_cancel_ops` in `compare_all_bench.rs`):
insert a level, reduce it down through `REDUCE_STEPS` steps, cancel it —
one level fully cycled (return-to-empty) before the next, so the same
op-count sequence can be cycled indefinitely inside Criterion without
violating either side's "insert only targets an empty level" assumption.
Level counts: 20 / 100 / 1000 per side.

## Numbers

See `compare/README.md` for the full comparison table (all contenders, one
run). hftbacktest's `update_*_depth` recomputes best-bid/ask via a fresh
`BTreeMap::keys().last()` (tree descent, O(log n)) or a linear
`depth_below`/`depth_above` scan (`HashMapMarketDepth`) on **every** call —
not just when the touch level actually changes. rsx-book's
`insert_resting`/`modify_order_qty_down` are O(1) in the common case and
only pay a scan when the touch level itself empties
(`scan_next_bid`/`scan_next_ask` — a known linear scan, see `BUGS.md`).
This is a genuine structural difference in the two designs, not a
methodology artifact — noted here so the numbers aren't read as
apples-to-apples "same algorithm, different constant factor."

## Fairness caveats

- **L2 vs L3.** hftbacktest never does order-level FIFO matching; the
  `match_touch` op is `N/A` for it by construction, never a faked number.
- **f64 vs i64.** hftbacktest takes `f64` prices/qty; rsx-book is
  fixed-point `i64`. Neither side's tick-conversion cost is excluded or
  charged to the other — hftbacktest converts `f64` price → `i64` tick
  internally on every call (`(price / tick_size).round() as i64`);
  rsx-book's `CompressionMap::price_to_index` is already `i64`-native.
  Both costs are included in the numbers as each library actually runs.
- **Same box, same harness, same op stream** — the strongest fairness bar
  in this directory (contrast with the cross-language cited numbers in
  `compare/cross-language-cited.md`, which are NOT re-run here).
- Excluded impls: `ROIVectorMarketDepth` (needs a fixed ROI/index-range
  config not naturally derived from our level-abstraction) and
  `FusedHashMapMarketDepth` — skipped for scope, not because of any
  fairness problem; `BTreeMarketDepth` + `HashMapMarketDepth` already cover
  hftbacktest's "naive tree" and "optimized hashmap" design points per the
  original cross-match plan (`.ship/34-COMPARE-RESEARCH/PLAN.md`).
