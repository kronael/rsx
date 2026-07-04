# rsx-book

A fixed-point limit-order book and matching engine whose match
latency is **constant in book depth** — ~60-65 ns to match whether
the book holds 100 thousand or 10 million resting orders.

One instance is the matching-engine tile for one symbol: a
compression-mapped, slab-arena, i64 fixed-point order book plus the
cross/rest/cancel algorithm over it (`rsx-matching` wraps it in a
process; `rsx-marketdata` runs it as a shadow book). Zero heap on
the hot path, no floats, cache locality as the first design
priority.

## Why this exists

A matching engine's worst enemy is an operation whose cost grows
with the book. The naive shapes all have one:

- A price→level tree (`BTreeMap`) is O(log n) on every insert,
  cancel, and next-best lookup, and allocates a node per new level.
- A flat price array is O(1) to index but O(range) to find the
  next-best level when one empties, and needs a slot per tick over
  the whole price range.

rsx-book removes the depth term from the paths that matter. Three
data structures compose to do it:

- **Slab arena** — orders live in a preallocated array; alloc is a
  bump or free-list pop (556 ps / 1.44 ns), free is an O(1) unlink.
  A cancel is a pure slab unlink by handle: no tree descent, no hash
  probe, no scan.
- **Compression map** — a sawtooth price→slot quantization that
  keeps the full tradeable range in a bounded, dense level array
  (~120k slots for a typical mid/tick, not one slot per tick).
  Near mid it is 1:1; far from mid it coarsens. price→index is a
  3-comparison bisection (~2 ns).
- **Occupancy bitmap** — a hierarchical set-bit index over the
  compression slots, so "next non-empty level" is a
  trailing/leading-zeros find over a handful of hot summary words
  (O(depth=3)), not an O(slots) sweep of the 2.4 MB level array.

The payoff is depth-invariant matching, O(1) cancel-by-handle, and
best-price maintenance that costs a few cache lines. The cost is a
few ns of constant machinery on every level operation — the honest
trade, quantified below.

## How fast

Single core, in-process, Criterion closed-loop, quiet box (the RSX
cluster stopped first — its busy-spin poisons the numbers). Host:
AMD Ryzen 9 5950X, single core. Commit `f45e0a0`. Source:
`cargo bench -p rsx-book`. Full run + method:
[`reports/20260704_book-bench.md`](../reports/20260704_book-bench.md).

**Matching is O(1) in book depth** (`match_depth` / `deep_flat_match`):

| resting orders in book | match latency (median) |
|---|---:|
| 1          | 28.1 ns |
| 100        | 60.0 ns |
| 10,000     | 60.2 ns |
| 100,000    | 64.5 ns |
| 1,000,000  | 66.3 ns |
| **10,000,000** | **65.5 ns** |

Same op across a 10^5× depth range, flat. When a match clears the
touch level (so the book must find the next best) it costs
**145 ns** — `match_ioc_vs_1k_asks`; the occupancy bitmap makes that
next-best find O(depth=3), not O(slots).

Primitives and common ops (per-op, single core):

| op | median | bench |
|---|---:|---|
| slab alloc (bump) | **556 ps** | `book_bench` |
| slab alloc (freelist) | 1.44 ns | `book_bench` |
| slab free | 8.30 ns | `book_bench` |
| compression price→index | 1.91–2.18 ns | `book_bench` |
| best-price read | 1.47 ns | `compare_all_bench::best_read` |
| cancel (depth 1k–10k) | 17.8–17.9 ns | `compare_naive_bench` |
| modify qty-down | 2.12 ns | `book_bench` |
| post-only reject | 5.96 ns | `match_by_type/post_only_reject` |
| FOK feasibility + reject (depth 10k) | ~118 ns | `match_by_type/fok_full` |
| insert + cancel (pair, depth 1k–10k) | 160–171 ns | `compare_naive_bench` |

### Versus other order books

Same box, same Criterion harness (`compare_all_bench`), same op
stream. Full table + per-contender notes:
[`compare/`](compare/).

- **vs `hftbacktest`** — rsx-book wins level ops (15.5–16.2 ns vs
  18–24 ns); hftbacktest is an L2-depth feed reconstructor and does
  not match orders at all (no FIFO).
- **vs `lob`** — rsx-book wins insert+cancel decisively (31–33 ns
  flat vs 98 ns → 1.22 µs, growing 12× with depth) and match_touch
  (30.8 ns vs 81 ns).
- **vs a bare `BTreeMap<price, VecDeque<order>>`** — the naive tree
  is ~1.5× *faster* on pure level churn (10–20 ns) because it skips
  the compression/slab/occupancy machinery. That machinery is what
  buys depth-invariant matching, O(1) cancel, and flat level ops:
  on deep cancel rsx-book is 5.5–10× faster and the gap grows with
  depth. A `BTreeMap` is O(log n), never O(book-size), so the gap
  here is constant-factor, not asymptotic — the honest read.
- **vs tuned C++ flat-array ITCH books** (e.g.
  `itch-order-book` at 61 ns/tick) — same neighborhood, but
  **cited only**, never benched here: different language, ~14-year-
  old hardware, book-maintenance not matching. See
  [`compare/cross-language-cited.md`](compare/cross-language-cited.md).

**Caveats.** These are single-core, in-process, closed-loop
Criterion microbenchmarks — the *algorithm*, not the exchange
round-trip. The cross-process p50 is ~1.1 ms, dominated by
transport and scheduling, not matching (a different story; see the
transport crate). p99 is not published here. Re-run on a quiet box
before quoting an exact ns elsewhere; the relative picture is
stable, individual ns are not.

## What it gives you

- **`Orderbook`** — the full book: compressed bid/ask levels, the
  slab, live best-bid/ask, per-user position tracking, migration
  state, and the per-cycle event buffer. `insert_resting`,
  `cancel_order`, `modify_order_price`, `modify_order_qty_down`.
- **`matching::process_new_order`** — the matching algorithm:
  cross against the opposite side FIFO, then rest / cancel / reject
  the residual per time-in-force. Handles GTC, IOC, FOK, post-only,
  and reduce-only.
- **`Slab<T>`** — generic fixed-capacity arena: O(1) alloc (bump or
  free-list) and O(1) free, zero heap on the hot path.
- **`CompressionMap`** — the 5-zone sawtooth price→slot map.
- **`Occupancy`** — the hierarchical bitmap behind next-best-level
  lookup.
- **`Event`** — `Fill`, `OrderInserted`, `OrderCancelled`,
  `OrderDone`, `OrderFailed`, `BBO`. Written into a fixed per-cycle
  buffer; the caller drains it after each order.
- **`snapshot`** — binary save/load with a magic + version header.

## Quick start

```rust
use rsx_book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_book::Event;
use rsx_types::Side;
use rsx_types::TimeInForce;

// One book per symbol. `capacity` is the slab order-slot count;
// `mid_price` seeds the compression map (raw i64 units).
let mut book = Orderbook::new(config, 1_000_000, mid_price);

// A resting maker (GTC) — inserts, no cross.
let mut maker = IncomingOrder {
    price: 100, qty: 10, remaining_qty: 10,
    side: Side::Sell, tif: TimeInForce::GTC,
    user_id: 1, reduce_only: false, post_only: false,
    timestamp_ns: 0, order_id_hi: 0, order_id_lo: 1,
};
process_new_order(&mut book, &mut maker);

// A marketable taker (GTC) — crosses, fills, done.
let mut taker = IncomingOrder {
    price: 100, qty: 10, remaining_qty: 10,
    side: Side::Buy, tif: TimeInForce::GTC,
    user_id: 2, reduce_only: false, post_only: false,
    timestamp_ns: 0, order_id_hi: 0, order_id_lo: 2,
};
process_new_order(&mut book, &mut taker);

// Drain the events this order produced (Fill, OrderDone, BBO...).
for event in book.events() {
    match event {
        Event::Fill { price, qty, .. } => { /* ... */ }
        Event::OrderInserted { handle, .. } => { /* keep handle to cancel */ }
        _ => {}
    }
}

// Cancel a resting order by its slab handle (O(1) unlink).
book.cancel_order(handle);
```

`process_new_order` resets the event buffer at the start of each
call, so `book.events()` returns exactly the events for the order
just processed. Consumers today: `rsx-matching` (the ME process)
and `rsx-marketdata` (shadow book).

## Guarantees

The book upholds these system-wide correctness invariants (numbers
match `../CLAUDE.md`):

- **Fills precede ORDER_DONE (per order).** `match_at_level` emits
  each `Fill` before the maker's `OrderDone`, and before the taker's
  terminal event.
- **Exactly-one completion per order.** Every path through
  `process_new_order` emits exactly one terminal event:
  `OrderFailed` (validation / FOK / reduce-only), `OrderCancelled`
  (post-only would cross), `OrderDone` (IOC residual or full fill),
  or `OrderInserted` (resting; its terminal event fires later).
- **FIFO within a price level (time priority).** New orders link at
  the level tail; `match_at_level` walks from the head.
- **No crossed book.** The aggressor matches against the touch until
  it stops crossing, then rests the residual — post-loop the book
  cannot be crossed. Best bid/ask are tracked by **raw price**
  (the compression index is a sawtooth, not a price proxy).
- **Slab no-leak.** `live = bump_next − |freelist|`; every `alloc`
  is paired with at most one `free`. Cross-checkable via
  `Slab::free_count`.
- **ME never drops events.** `Orderbook::emit` asserts on overflow
  of the 65,536-slot per-cycle buffer (a runaway cascade is treated
  as an unrecoverable bug, not a silent drop).

## Order types

| type | behavior |
|---|---|
| GTC (limit) | match, rest the remainder |
| IOC | match, cancel the remainder (`OrderDone`, cancelled) |
| FOK | **pre-check** full-fill feasibility; if not fillable, reject before any match (no partial, no rollback) |
| Post-only | reject if it would cross (`OrderCancelled`) |
| Reduce-only | clamp qty to the user's net position; reject if wrong side or no position |

FOK checks `can_fill_fully` (walk crossing levels in price order,
sum maintained `total_qty`, early-exit) *before* matching — so a
rejected FOK never touches the book. There is no fill-then-rollback
path.

## Requirements and assumptions

- **Single-owner, single-threaded.** The book is plain state; it
  makes no thread-safety claims because no caller shares it across
  threads. The caller owns the loop.
- **Inputs are pre-validated.** `process_new_order` runs a tick/lot
  gate (`validate_order`), but the wider system validates margin and
  auth upstream (gateway + risk). The book is the last stage, not
  the security boundary.
- **Fixed-point i64 only, no floats.** Prices and quantities are raw
  i64 units; conversion happens at the API boundary, not here.
- **Memory is preallocated and config-driven.** The slab capacity
  (order-slot count) is a constructor argument; the level array size
  follows from mid/tick via the compression map. A production sizing
  (tens of millions of slots) reserves ~10 GB of **virtual** memory
  for the slab — physical pages fault in on use. The event buffer is
  a heap-boxed `[Event; 65_536]`.

## When NOT to use this

- **You need cross-thread concurrency.** The book is single-owner;
  there is no locking. Shard by symbol (one book per thread) instead.
- **You want an L2/L3 market-data feed decoder.** This is an
  order-level matching book, not a depth-feed reconstructor — use
  `hftbacktest` or similar for ITCH/depth replay.
- **Your price domain is unbounded or sparse and you never match.**
  A plain `BTreeMap` is simpler and ~1.5× faster on pure level
  churn; rsx-book's machinery only pays off when you need
  depth-invariant matching and O(1) cancel.
- **You need floats or a schema-flexible order.** `IncomingOrder`
  is a fixed i64 struct; there is no dynamic field set.
- **Deep full-book sweeps are your hot path.** A market order that
  clears K levels is O(K·depth) — linear in levels swept (each find
  is cache-local, but it is not free). The design optimizes the
  near-BBO IOC path; the rare deep sweep is left simple.

## Install / MSRV

Internal-use crate within the wider rsx exchange project; **not
published on crates.io**. Depends only on `rsx-types` (plus
`rustc-hash` and `tracing`). Rust stable, edition 2021, no nightly
features. Crate version **0.2.0**.

## Lineage / Acknowledgments

- **Slab / arena allocation** — the standard pooled-object pattern
  (`slab`, `bumpalo` crates); rsx-book uses a hand-rolled 128-byte,
  64-byte-aligned slab so order slots are cache-line aligned. See
  [`notes/arena.md`](notes/arena.md).
- **Compression / price-ladder quantization** — the flat price-array
  book of exchange practice (Nasdaq ITCH-style flat books), with a
  sawtooth zone map to bound the array around mid.
- **Hierarchical occupancy bitmap** — the dense-domain radix/bitmap
  index idea (fanout-64 find-next-set via `tzcnt`/`lzcnt`), chosen
  over a BTree because the compression map already quantizes prices
  to dense slots. See [`notes/occupancy.md`](notes/occupancy.md).
- **Cache-oriented layout** — `#[repr(C, align(64))]`, hot/cold
  field splitting (Drepper, Acton data-oriented design). See
  [`notes/align.md`](notes/align.md), [`notes/hotcold.md`](notes/hotcold.md).

## Alternatives

If rsx-book doesn't fit, the peers benched or surveyed in
[`compare/`](compare/):

- [**naive BTreeMap**](compare/naive-btree.md) — the obvious
  textbook book; simpler, faster on level churn, O(log n) not
  depth-invariant.
- [**hftbacktest**](compare/hftbacktest.md) — L2 depth
  reconstruction; no order-level matching.
- [**lob**](compare/lob.md) — a from-scratch Rust order book;
  insert/cancel grows with depth.
- [**itch-order-book / exchange-core / liquibook**](compare/cross-language-cited.md)
  — cited C++/Java baselines (not benched here).

## How to read this crate

- **What** — this README + the formal spec `specs/2/21-orderbook.md`.
- **How** — [ARCHITECTURE.md](ARCHITECTURE.md): the data structures,
  how they compose, the matching algorithm, the ME-tile plug-in.
- **Why + Numbers** — [`notes/`](notes/) for the design rationale
  (slab over malloc, hot/cold split, cache-line alignment, the
  occupancy bitmap's deep why) and
  [`compare/`](compare/) for the cross-book comparison.
  Dated benchmark runs live in `reports/` at the repo root.

## License

Internal-use crate within the wider rsx exchange project.
