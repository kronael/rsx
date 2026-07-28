# rsx-book Architecture

The deterministic, in-memory matching core and the data structures behind it.
This is "how it is"; the "why" lives in
[`notes/`](notes/) and the formal spec `specs/2/21-orderbook.md`.

## Module layout (`rsx-book/src/`)

| File | Purpose |
|------|---------|
| `book.rs` | `Orderbook` struct; `insert_resting`, `unlink_order`, `cancel_order`, `modify_*`, best-bid/ask scan, occupancy maintenance, `price_asc` build. |
| `matching.rs` | `process_new_order`, `match_at_level`, `can_fill_fully` — GTC / IOC / FOK / post-only / reduce-only, event emission. |
| `slab.rs` | Generic `Slab<T>` arena: bump + free-list, O(1) alloc/free. |
| `compression.rs` | `CompressionMap` — 5-zone sawtooth price→index bisection. |
| `occupancy.rs` | `Occupancy` — hierarchical set-bit bitmap; `set`/`clear`/`find_next`/`find_prev` over a fixed summary hierarchy. |
| `level.rs` | `PriceLevel` — 32-byte level head (head/tail slab handle, total_qty, order_count). |
| `order.rs` | `OrderSlot` — 128-byte `#[repr(C, align(64))]` order, hot fields in cache line 0. |
| `event.rs` | `Event` enum + typed reason codes; `MAX_EVENTS = 65_536`. |
| `user.rs` | `UserState` — per-user net position + active order count, tracked inside the book. |
| `migration.rs` | Lazy incremental recentering when mid drifts. |
| `snapshot.rs` | Owned logical snapshot capture/restore plus legacy RXSN loading. |

## How the pieces compose

```
                        Orderbook
   ┌──────────────┬──────────────┬───────────────────────────┐
   │ active_levels│  orders       │  bid_occ / ask_occ         │
   │ Vec<PriceLevel> Slab<OrderSlot>  Occupancy (per side)     │
   │  (dense,     │  (arena,      │  (bitmap over the same     │
   │  compressed) │  128B slots)  │   compression slots)       │
   └──────┬───────┴──────┬────────┴────────────┬──────────────┘
          │ index by     │ head/tail handle    │ bit set =
          │ compression  │ into slab; orders    │ level non-empty
          │ price→slot   │ doubly-linked (FIFO) │ (next-best find)
          ▼              ▼                      ▼
   CompressionMap    OrderSlot chain        find_next / find_prev
   (price → u32)     head → … → tail        (O(depth) skip-empty)
```

- A price maps to a **slot index** via `CompressionMap::price_to_index`
  (bisection, ~2 ns).
- `active_levels[slot]` is a `PriceLevel`: head/tail handles into the
  slab, plus `total_qty` and `order_count`.
- Orders at a level are a doubly-linked list threaded through the
  slab (`next`/`prev` are slab handles), head→tail = time priority.
- `bid_occ` / `ask_occ` mark which slots are non-empty, so
  next-best-level is a bitmap find, never a scan.

## Slab arena (`slab.rs`)

`Slab<OrderSlot>` is a preallocated `Vec<OrderSlot>` with a
free-list head and a bump cursor.

- **alloc**: pop `free_head` (reuse) or bump `bump_next` (fresh).
  O(1). Asserts on exhaustion.
- **free**: push the slot onto `free_head`. O(1). The free-list
  chains through each slot's `next` field.
- **no-leak invariant**: `live = bump_next − |freelist|`. Every
  `alloc` pairs with at most one `free`. `free_count()` walks the
  list for test/introspection cross-checks (not the hot path).

`OrderSlot` is 128 bytes, `#[repr(C, align(64))]` (compile-time
asserted): cache line 0 is hot (price, remaining_qty, side, flags,
tif, next/prev/tick_index), cache line 1 is cold (user_id, sequence,
original_qty, timestamp_ns, order ids). Cancel and match touch only
line 0.

## Compression map (`compression.rs`)

A price→slot quantization centered on `mid_price`, so the whole
tradeable range fits a bounded, dense level array instead of one
slot per tick. Five zones by absolute distance from mid:

| zone | distance from mid | ticks per slot |
|---|---|---|
| 0 | 0–5%   | 1  (1:1 near mid) |
| 1 | 5–15%  | 10 |
| 2 | 15–30% | 100 |
| 3 | 30–50% | 1000 |
| 4 | 50%+   | catch-all, 1 slot per side |

`price_to_index` is a 2–3 comparison bisection over the four raw-price
thresholds, then an in-zone offset. Each zone is a symmetric ± band
around mid laid out at ascending index, so the index is a
**sawtooth**: it is not globally price-monotonic across zone
boundaries. Two consequences enforced everywhere:

- **BBA is tracked by raw price, never by index.** `best_bid_px` /
  `best_ask_px` are compared as prices; the tick index is only a
  slot address.
- **Next-best walks `price_asc`, not the raw bitmap.**
  `build_price_asc` precomputes the ≤10 zone-half index sub-ranges
  ordered by price band (within each, ascending index == ascending
  price). Recomputed only on construction / recenter.

Orders sharing a slot (zones 1–4) store their exact price; the
matcher checks the real price per order.

## Occupancy bitmap (`occupancy.rs`)

Per-side hierarchical set-bit index over the compression slots: bit
set = "this level holds ≥1 resting order of that side". `levels[0]`
is one bit per slot; each higher level is one bit per word of the
level below. ~120k slots ⇒ 3 levels (1929 + 31 + 1 words), ~15 KB.

- **`set` / `clear`** — O(depth); climb up only while a word flips
  empty↔non-empty, so a deep cancel is a couple of word writes.
- **`find_next` / `find_prev`** — O(depth); climb summary words to
  find a candidate, then descend via `trailing_zeros` /
  `leading_zeros`.
- **`find_first_in` / `find_last_in`** — bounded to a `price_asc`
  sub-range.

Maintained in lockstep with `PriceLevel::order_count` at every site
that crosses the 0 boundary: `insert_resting` (0→1 sets),
`unlink_order` (→0 clears — covers cancel and maker-fill),
`migrate_single_level`, `trigger_recenter` (reset), and snapshot
load (`rebuild_occupancy`). A stale bit would be a phantom or
skipped level, i.e. a matching bug; `scan_reference_test.rs`
cross-checks the bitmap path against a brute-force scan. Deep why:
[`notes/occupancy.md`](notes/occupancy.md).

## Best-bid / best-ask

Cached as `(best_bid_tick, best_bid_px)` / `(best_ask_tick,
best_ask_px)`. Updated on the fly during insert (compare raw price)
and match. When a level empties, `scan_next_bid` / `scan_next_ask`
walk `price_asc` in price order and take the extreme set bit per
sub-range — the common near-BBO case returns after touching a few
zone-0 summary words.

## Matching algorithm (`matching.rs`)

`process_new_order(book, order)` — one aggressor in, events out.
Event buffer is reset at entry, so `book.events()` afterward is
exactly this order's events.

```
process_new_order(book, order):
    event_buf.clear()
    validate tick/lot           -> OrderFailed(VALIDATION) on fail
    reduce-only:                 clamp qty to net position,
                                 or OrderFailed(REDUCE_ONLY)
    post-only:                   OrderCancelled(POST_ONLY) if would cross
    FOK:                         can_fill_fully()? else OrderFailed(FOK)
                                 (pre-check — no partial, no rollback)

    cross loop (Buy vs asks / Sell vs bids):
        while remaining > 0 and touch level exists:
            match_at_level(book, touch, order)
            if touch emptied: best = scan_next_*()
            if no progress or book side empty: break

    residual:
        remaining > 0 and IOC   -> OrderDone(CANCELLED, filled/remaining)
        remaining > 0 otherwise -> insert_resting + OrderInserted
        remaining == 0          -> OrderDone(FILLED)

    if best bid/ask tick changed: emit BBO
```

`match_at_level(book, tick, aggressor)` walks the level from `head`
(FIFO), and per maker:

```
    skip if maker price doesn't cross aggressor limit
    fill_qty = min(aggressor.remaining, maker.remaining)
    decrement both; decrement level.total_qty
    emit Fill                       <-- fill precedes any OrderDone
    update_positions_on_fill(taker, maker)
    if maker fully filled:
        unlink_order (clears occupancy if level empties)
        emit OrderDone(FILLED) for the maker
        slab.free(maker)
```

Event ordering upholds invariant #1 (fills precede ORDER_DONE): the
maker's `Fill` is emitted before its `OrderDone`, and both before
the taker's terminal event at the end of `process_new_order`.
Invariant #2 (exactly-one completion): every return path emits
exactly one terminal event for the aggressor.

The `reduce_only` flag arrives on the Risk-authorized order. The book does not
receive a separate position snapshot: `UserRegistry` derives per-symbol
`net_qty` from the same in-order fills it applies, persists it in the book
checkpoint, and reconstructs post-checkpoint changes by deterministic order
WAL replay. This narrow duplicate is needed at the exact match-sequence point;
portfolio margin, collateral, and currency conversion remain in Risk and the
gateway.

An explicit cancel is handled by the ME command path: it calls
`cancel_order_checked`, then emits `OrderCancelled(User)` followed by
`OrderDone(Cancelled)`. `OrderCancelled` therefore describes both policy
cancellation (such as post-only crossing) and a user cancel command; its
typed reason distinguishes them.

`can_fill_fully` (FOK) walks only the *crossing* levels in price
order via `price_asc` + occupancy, summing each level's maintained
`total_qty` and early-exiting the instant the running total reaches
the order size — O(levels crossed), never a per-order or full-slot
scan. A whole level shares one price, so `total_qty` counts it
exactly.

## Incremental recentering (`migration.rs`)

When mid drifts past 50% of zone-0 half-width, the book rebuilds its
compression map without a stop-the-world pause:

- `trigger_recenter(new_mid)` — swap in a fresh empty active array
  and new `CompressionMap`; keep the old array as `old_levels`;
  reset occupancy + `price_asc`; enter `Migrating`.
- `resolve_level(price)` — lazy: migrate the old level covering
  `price` on first access past the frontier.
- `migrate_batch(n)` — proactive: migrate `n` levels per idle call,
  advancing bid/ask frontiers until both cover the old range, then
  `complete_migration` back to `Normal`.

During migration two arrays are live; `migrate_single_level`
re-links each order into the new array and re-sets occupancy / BBA.

## Event buffer

`Orderbook` reserves space for 65,536 events once at construction and clears
the length per `process_new_order`; the reserved storage is reused without
growing. `emit` asserts on overflow (invariant: ME never drops events). A sweep
emits two events per fully consumed maker
(`Fill` + maker `OrderDone`), then taker `OrderDone` and at most one
final `BBO`, so one cascade safely covers 32,767 fills. The caller
drains `book.events()` after each order; that fan-out lives in the ME
process, not in this crate.

The fixed cap keeps this implementation short enough to study; it is not the
intended production boundary. The direct replacement is a bounded resumable
match step that checks output capacity before the next state mutation, returns
a continuation, lets the caller drain, and resumes from the same maker and
remaining taker quantity. That continuation is not implemented here.

`Event` is `Copy`, so `emit` performs one fixed-buffer store.
`book.events()` returns `&[Event]`; iteration borrows each event and does not
copy it again. A callback or lazy producer would save that one store at the
cost of running WAL, transport, and backpressure code inside the book
transition. Materializing the bounded result keeps the state change complete
before any external side effect and preserves one inspectable event order for
all consumers.

## Snapshot and recovery (`snapshot.rs`)

The book snapshot is a recovery checkpoint, distinct from Marketdata's L2
client snapshot.

`snapshot::capture` rejects a migrating book, then owns:

- book sequence, config, and compression center;
- active orders in exact per-level FIFO order;
- slab handles and the exact free-list consumption order; and
- user state and its allocation order.

The DTO owns values only. It contains no references to the live slab or level
arrays. `snapshot::restore` validates handles and free lists, restores slab and
user allocation order, rebuilds linked levels from the captured FIFO order,
then rebuilds occupancy and BBO.

`rsx-matching` owns the WAL boundary and Avro disk adapter. The ME captures on
its single thread, resumes matching, and sends the owned DTO to a named worker.
That worker writes and fsyncs `snapshot.bin.tmp`, atomically renames it, and
fsyncs the directory. Recovery also accepts the legacy checkpoint envelope
with RXSN v2 or v3 state. Snapshot capture/restore p50 and p99 are not measured.

## How it plugs into the ME process

`rsx-book` is the deterministic matching-core library: book state, crossing,
order lifecycle, events, and checkpoints, but no runtime, threads, transport,
or WAL ownership. The matching-engine process (`rsx-matching`) owns one
`Orderbook` per symbol and drives it from a pinned loop:
decode an incoming order off the transport, call
`process_new_order`, drain `book.events()`, and publish. Because
each book is single-owner state on one thread, it needs no locking;
scale-out is one book (one core) per symbol. `rsx-marketdata` runs
the same book as a shadow (fed inserts/cancels/fills) to serve L2 /
BBO. The book makes no thread-safety claims because no caller shares
it across threads.

**Trust boundary.** The book is internal, single-owner, and never
touches the network. Authentication, margin, and risk limits are
enforced upstream (gateway JWT, risk tile) before an order reaches
it — the book trusts that boundary and does not re-check caller
identity or solvency on the hot path. It validates only structural
well-formedness (tick / lot multiples, reduce-only / post-only
semantics), the checks it needs to keep its own state consistent.

## Memory layout (config-driven)

| Component | Sizing | Memory |
|---|---|---|
| Order slab | `capacity` × 128 B (constructor arg) | e.g. 78M slots ≈ 10 GB **virtual**; pages fault in on use |
| Price levels (×2: active + staging) | ~120k slots × 32 B × 2 | ~8 MB (grows with range/tick) |
| Occupancy (×2 sides) | ~15 KB each | negligible |
| Event buffer | reserved `Vec<Event>` capacity of 65,536 | ~8 MB |

Slab capacity is a caller choice; level-array and occupancy sizes
follow from mid/tick via the compression map.

## Operation complexity

| operation | cost |
|---|---|
| Insert (rest) | O(1) — bisect + slab alloc + tail link + bit set |
| Cancel by handle | O(1) — slab unlink + bit clear; if the touch empties, a bounded summary-word lookup finds the next level |
| Match, touch survives | O(fills) |
| Match, touch clears | O(fills) plus one bounded next-level lookup |
| Deep sweep of K levels | O(K) in levels swept |
| Best bid/ask read | O(1) — cached |
| Recenter | amortized via lazy + batched migration; no global pause |

## Architectural decisions

**Runtime: none — deterministic matching core.** `rsx-book` is a library,
not a process: no async runtime, no threading primitives, no transport I/O.
The caller owns the loop and the threading model. Consumers today
are `rsx-matching` (the ME process) and `rsx-marketdata` (shadow book),
both single-owner on whatever thread drives them.
