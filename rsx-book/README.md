# rsx-book

A fixed-point limit-order book and matching core. It is built for
cancel/reprice-heavy traffic: many resting quotes leave the book without
filling, so cancel is a primary hot path.

The crate boundary separates deterministic state transitions from the ME
daemon. `rsx-book` owns crossing and book state; `rsx-matching` owns transport,
WAL/recovery orchestration, config polling, and downstream fan-out. Keeping
those separate makes the core independently testable and benchmarkable without
pulling I/O into the match path.

A few terms that come up throughout:

- **symbol** — one tradeable market, e.g. BTC-PERP. One book per symbol.
- **tick** — the smallest price increment a symbol allows.
- **depth** — how many orders are currently resting in the book.
- **fixed point** — prices and quantities stored as exact integer units.
- **compression map** — folds the huge tradeable price range into a
  small, bounded, fixed table of levels (not one slot per tick).
- **occupancy bitmap** — finds the next occupied price level in ~3
  steps over summary words, not a linear scan of every level.

The compression layout is symmetric around the current mid price. Slots
become coarser farther from active trading:

```text
raw price distance from mid — widths are schematic

 low ◄─ 50% ─ 30% ─ 15% ─ 5% ─ MID ─ 5% ─ 15% ─ 30% ─ 50% ─► high
        Z4     Z3     Z2    Z1    Z0    Z1     Z2     Z3     Z4
      catch  1000:1  100:1 10:1  1:1  10:1   100:1  1000:1 catch

physical level array — slot index increases ►

 Z0  [-5% ··········· MID ··········· +5%]  one tick per slot
 Z1  [-15% ··· -5%] [+5% ··· +15%]           ten ticks per slot
 Z2  [-30% ·· -15%] [+15% ·· +30%]           100 ticks per slot
 Z3  [-50% ·· -30%] [+30% ·· +50%]           1,000 ticks per slot
 Z4  [above +50%] [below -50%]                one catch-all per side

occupancy over the same slots

 slots       [ ][ ][■][ ] ··· [■][ ][ ]
 L0 bits      0  0  1  0  ···  1  0  0      one bit per slot
 L1 summary  [       1       ][   1   ]      one bit per L0 word
 L2 root     [           1           ]      one bit per L1 word
```

Each zone is price-ordered internally. The physical array is a sawtooth:
zone 0 comes first, then zone 1, and so on. Each occupied level holds FIFO
slab handles.

Cancel is a primary operation, not cleanup. Quotes are withdrawn or
repriced as the market, inventory, and risk change; many resting orders
never fill. A direct handle-to-slab unlink keeps that path bounded.

## Design

- **Slab arena** — preallocated order slots with direct allocate, unlink, and
  reuse. Cancel uses the order handle directly.
- **Compression map** — bounded level slots across the tradeable range.
  Mapping a price to a slot takes 2–3 comparisons.
- **Occupancy bitmap** — a three-level summary for a typical map. Emptying
  the touch finds the next occupied level without scanning the level array.

That bookkeeping costs time in a narrow aggregate-level microbenchmark. A
bare tree wins when the workload is only: insert one price entry, overwrite
its total quantity, then remove it. That probe has no FIFO queue, per-order
identity, matching sweep, or realistic cancel lookup.

## Measurements

The current `match_depth`, `deep_flat_match`, and `deep_match_tail` sweeps
change preloaded depth while keeping one non-draining partial fill constant.
No depth-related increase was observed from 100,000 to 10 million resting
orders in the run below. The measurement describes that workload; it does not
make a general claim about sweeps or compressed levels.

| resting orders in book | p50 | p50 standard error | p99 |
|---:|---:|---:|---:|
| 100,000 | 52.594 ns | 0.024 ns | 87.656 ns |
| 1,000,000 | 50.719 ns | <0.001 ns | 82.969 ns |
| 10,000,000 | 53.688 ns | 0.002 ns | 111.922 ns |

Method:
[`reports/20260727_book-deep-match-tail.md`](../reports/20260727_book-deep-match-tail.md).

The separate touch-clearing benchmark includes the occupancy lookup for the
next price; its dated report owns those measurements.

The [`deep_million`](../reports/20260718_deep-million.md) evidence run
drains every maker in a one-million-order book through bounded FIFO
cascades and verifies empty-book and slab-reuse invariants.

The same-box comparisons, primitive timings, competitor scope, and tail
measurements live in [`compare/`](compare/) and the dated
[`reports/`](../reports/).

## API

- **`Orderbook` and `matching::process_new_order`** — cross FIFO orders,
  rest residuals, cancel by handle, and modify price or quantity.
- **`BookConfig`** — the symbol ID plus raw tick and lot alignment. Display
  precision is deliberately absent; the gateway owns human-unit conversion.
- **`Slab<T>`, `CompressionMap`, and `Occupancy`** — preallocated order
  storage, bounded price slots, and next-level lookup.
- **`Event`** — fills, book changes, terminal outcomes, and BBO updates.
  `emit` appends one value to a reserved buffer; `events()` returns a borrowed
  slice, so draining does not copy `Event` values.
- **`snapshot::capture` / `snapshot::restore`** — owned `BookSnapshotV1`
  values for active orders, FIFO queues, slab reuse order, users, config, and
  compression center. WAL sequence and disk format belong to `rsx-matching`.

The event buffer separates one complete book transition from I/O and
backpressure. A callback or lazy iterator would interleave matching with WAL
and transport side effects. The caller already iterates `&[Event]` and can
encode each borrowed event directly into its output record.

### Snapshot and recovery

The book briefly walks its level queues and returns an owned logical snapshot.
Matching resumes before serialization. `rsx-matching` adds the authoritative
WAL sequence and writes the value as an Avro Object Container File on a named
worker. Recovery restores slab and FIFO order, rebuilds levels, occupancy, and
BBO, then replays later WAL records. Capture is rejected while the compression
map is migrating.

Snapshot save/load p50 and p99 have not been measured.

## Quick start

Run the complete insert, match, event-drain, and cancel example:

```bash
cargo run -p rsx-book --example book_smoke
```

Source: [`examples/book_smoke.rs`](examples/book_smoke.rs).
`book.events()` contains only the events emitted by the latest
`process_new_order` call.

## Guarantees

The book upholds these system-wide invariants:

- **Fills precede ORDER_DONE per order**
- **Exactly one terminal outcome per order**
- **FIFO within each price level**
- **Best bid remains below best ask**
- **Every allocated slab slot is free or active**
- **Events are not silently dropped.** This compact implementation asserts if
  its 65,536-slot per-order buffer overflows. That limit is kept for
  educational brevity, not as the production design: a bounded resumable
  match step can stop before mutation, let the caller drain, then continue.

## Order types

| type | behavior |
|---|---|
| GTC (limit) | match, rest the remainder |
| IOC | match, cancel the remainder (`OrderDone`, cancelled) |
| FOK | **pre-check** full-fill feasibility; if not fillable, reject before any match (no partial, no rollback) |
| Post-only | cancel if it would cross (`OrderCancelled`, post-only reason) |
| Cancel command | remove the referenced resting order (`OrderCancelled`, user-cancel reason, then `OrderDone`) |
| Reduce-only | clamp qty to the user's net position; reject if wrong side or no position |

Reduce-only does not import a balance or USD value into the book. Risk marks an
accepted order `reduce_only`; the book knows only that user's net quantity for
this symbol, maintained synchronously from its own fills. A checkpoint stores
that quantity, and deterministic WAL replay rebuilds fills after the
checkpoint. Risk remains authoritative for portfolio margin and collateral;
the duplicate per-symbol quantity exists only to decide reduce-only at the
exact match-sequence point.

## Requirements and assumptions

- **Single-owner, single-threaded.** The book is plain state; it
  makes no thread-safety claims because no caller shares it across
  threads. The caller owns the loop.
- **Inputs are pre-validated.** `process_new_order` still runs a raw-unit
  tick/lot alignment gate, but the wider system validates margin and
  auth upstream (gateway + risk). The book is the last stage, not
  the security boundary.
- **Fixed-point i64 only, no floats or display metadata.** Prices and
  quantities are raw i64 units. Human precision and conversion happen at
  the gateway boundary, not in the book.
- **Memory is preallocated and config-driven.** The slab's order-slot
  capacity is a constructor argument; the level array size
  follows from mid/tick via the compression map. A production sizing
  with tens of millions of slots reserves ~10 GB of **virtual** memory
  for the slab. Physical pages fault in on use. The event buffer is a
  `Vec<Event>` with capacity reserved once at construction.

## When NOT to use this

- **You need cross-thread concurrency.** The book is single-owner;
  there is no locking. Shard by symbol (one book per thread) instead.
- **You want an L2/L3 market-data feed decoder.** This is an
  order-level matching book, not a depth-feed reconstructor — use
  `hftbacktest` or similar for ITCH/depth replay.
- **Your price domain is unbounded or sparse and you never match.**
  A plain `BTreeMap` is simpler and wins the synthetic aggregate-level cycle
  described above. rsx-book pays extra bookkeeping for FIFO orders, direct
  cancel-by-handle, maintained BBO, and bounded next-level lookup.
- **You need floats or a schema-flexible order.** `IncomingOrder`
  is a fixed i64 struct; there is no dynamic field set.
- **Deep full-book sweeps are your hot path.** A market order that
  clears K levels is linear in the number of levels swept.

## See also

- **What** — this README + the formal
  [order-book spec](../specs/2/21-orderbook.md).
- **How** — [ARCHITECTURE.md](ARCHITECTURE.md): the data structures,
  how they compose, and the matching algorithm.
- **Why and numbers** — [`notes/`](notes/) for the design rationale and
  [`compare/`](compare/) for the cross-book comparison.
  Dated benchmark runs live in `reports/` at the repo root.
- **Related components** — `rsx-matching` owns one book process per
  symbol and drives `rsx_book::matching::process_new_order`;
  `rsx-marketdata` maintains a shadow book.
