# orderbook (inv2004/orderbook-rs, crates.io `orderbook` 0.1.9) — NOT benched

Evaluated and implemented (`orderbook_impl` module, since removed from
`compare_all_bench.rs`) but **excluded from `all_contenders()`**: it crashes
under the exact op stream every other contender in this directory is driven
through.

## What it is

A flat-array price-level book shaped around Coinbase's L2 feed format:
`BookRecord { price: f64, size: f64, id: Uuid }`, `open`/`done`/`change` for
insert/cancel/reduce, `bid()`/`ask()` for best price. Single global `Vec` of
`VecDeque<(f64, Uuid)>` indexed by `round(price * 100)`, hard-capped at
`20_000 * 100` slots (price must stay under $20,000).

## The crash

```
thread 'main' panicked at .../orderbook-0.1.9/src/ob.rs:154:28:
index out of bounds: the len is 2000000 but the index is 18446744073709551615
```

`ob.rs`'s `check_ask_bid` walks the best-bid/ask pointer back to the nearest
occupied slot after a cancel:

```rust
if p_idx == self.bid {
    while self.book[self.bid].len() == 0 { self.bid -= 1; }
}
```

`self.bid` is a `usize`. If the side being cancelled from becomes
**completely empty**, this loop has no floor check — it decrements straight
through index 0 (always empty; price 0.00 is never inserted) and
underflows. In a release build (overflow checks off) this wraps to
`usize::MAX`, and the very next `self.book[self.bid]` access panics with
`index out of bounds`.

## Why this isn't "avoiding a fair test"

Every op-stream generator in `compare_all_bench.rs`
(`gen_full_cycle_ops`/`gen_insert_cancel_ops`) fully cycles one level at a
time: insert → (reduce ×N) → cancel, back to empty, before touching the
next level. That means **the very first cancel on either side empties that
side completely** — a thin/new symbol going flat on one side is an entirely
normal, realistic book state, not an edge case invented to break this crate.
`orderbook` 0.1.9 cannot survive it. Making our test avoid "book goes fully
flat" just to dodge this crate's bug would make our own methodology less
realistic for every other contender, to flatter one that can't handle a
real market condition. So: this crate is disqualified by its own crash, not
by our choice of workload.

## Verdict

Not usable for a head-to-head. Documented here per the "if a crate won't
build/integrate cleanly, document why and move on" rule — a real,
reproducible finding, not a fabricated excuse. If this crate is revisited
later, the fix would need to live upstream (a `self.bid > 0` floor check in
`check_ask_bid`), which is out of scope for this repo.
