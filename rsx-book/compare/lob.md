# rsx-book vs lob (rafalpiotrowski/lob-rs, Rust, crates.io `lob` 0.1.0)

Bench: `rsx-book/benches/compare_all_bench.rs`, contender `lob`. Run:
`cargo bench -p rsx-book --bench compare_all_bench`. Same box/run as every
other row in this directory — see `compare/README.md` for the shared header.

## What lob is

A real order-level (L3) limit order book: `Order`/`LimitOrder` with an
`Oid`, `OrderBook::execute(&Order) -> Trade` does price-time-priority
matching + rests the unfilled remainder, `cancel_order(Oid)` removes a
resting order. Unlike hftbacktest's `depth` module, this is a genuine
apples-to-apples comparison class with rsx-book for **insert**, **cancel**,
and **match** — lob actually walks levels and generates fills, it doesn't
just track an aggregate qty.

Added as a scoped `[dev-dependencies]` entry (`lob = "0.1.0"`), used only
from `compare_all_bench.rs`; never a dependency of rsx-book's library code.

## What's NOT fairly compared — real API gaps, not chosen exclusions

lob 0.1.0's public `OrderBook` API has two real gaps versus rsx-book, found
while wiring the `BenchBook` trait impl (`lob_impl` in
`compare_all_bench.rs`):

- **No qty-reduce / amend API.** `OrderBook` exposes `execute` (place) and
  `cancel_order` (remove) — no "modify this resting order's quantity" call.
  `reduce()` returns `false` unconditionally; `supports_reduce() -> false`.
  This excludes `lob` from the `level_touch_full` bench group (which
  requires reduce) — it only appears in `level_insert_cancel`.
- **No public best-price read.** Internally, `Limits::get_best_limit()`
  tracks the best bid/ask per side — but `OrderBook` never exposes it
  (the `bids`/`asks` fields are private, no forwarding method). `best()`
  returns `None` unconditionally; `supports_best() -> false`. Excluded from
  `best_read`.
- **`Trade`'s filled quantity isn't publicly readable.** `execute()` returns
  `Result<Trade, PlaceOrderError>`, but `Trade`'s `filled_volume` and
  `executions` fields are private with no accessor (only `#[derive(Debug)]`).
  `match_touch` here can time the `execute()` call and confirm it succeeded,
  but cannot report back how much was actually filled — the trait method
  always returns `Some(0)` on success (a placeholder, not a claim about fill
  size). This is disclosed, not hidden: don't read anything into the `0`.

None of these are fairness violations to work around — they're genuine gaps
in what lob 0.1.0 exposes publicly. The harness reports `N/A` for the
excluded ops rather than faking a number (see `compare/README.md`'s
capability table).

## What IS fairly compared

`insert` (place a non-crossing limit order) and `cancel` (remove it) — the
`level_insert_cancel` group — and `match_touch` (a marketable order against
a large resting level) — both real order-level operations on both sides.

## Numbers

See `compare/README.md` for the full table.

## Fairness caveats

- Same box, same harness (Criterion), same synthetic op stream as every
  other benched contender in this directory.
- lob's own bundled `benches/lob_benchmark.rs` (in the crate itself) uses a
  different workload (10k orders built once, then two market orders that
  sweep the whole book) — not used here; we drive lob through our shared
  `BenchBook` trait so it sees the identical op stream as rsx-book/
  hftbacktest/naive-btree, not lob's own benchmark shape.
