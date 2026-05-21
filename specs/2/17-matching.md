---
status: shipped
---

# Matching Engine Service

Matching is per-symbol, single-threaded, and stateless with
respect to user balances. It consumes validated orders from
Risk and emits fills and order lifecycle events.

## Responsibilities

- Maintain orderbook (ORDERBOOK.md)
- Execute matches deterministically
- Emit fills, order inserted/cancelled/done events
- Append events to WAL (DXS.md/WAL.md)

## Inputs / Outputs

Inputs:
- CMP/UDP from Risk: validated orders (`RECORD_ORDER_REQUEST`)
  and cancels (`RECORD_CANCEL_REQUEST`)

Outputs:
- CMP/UDP to Risk: fills and order lifecycle events
- CMP/UDP to Marketdata: Fill, OrderInserted, OrderCancelled, BBO
  (no OrderDone — see MARKETDATA.md)
- WAL records for replay/marketdata

## Threading

One ME process per symbol; a single pinned thread runs the main
loop. `RSX_ME_CORE_ID` pins to a core (a degenerate one-tile
"tile architecture"). No locking inside the loop.

## Order Acceptance Flow

For every CMP/UDP order frame:

1. **Dedup** — `DedupTracker::check_and_insert(user_id, oid_hi,
   oid_lo)`. On duplicate, write `OrderFailedRecord{reason=
   REASON_DUPLICATE}` to WAL and CMP to Risk; skip the rest.
2. **Accept** — write `OrderAcceptedRecord` to WAL. WAL append
   failures panic with a named invariant (see consistency
   invariant 7).
3. **Process** — call `process_new_order` on the book; events
   land in the book's fixed event buffer.
4. **Persist events** — `write_events_to_wal` appends every
   emitted event (Fill, OrderInserted, OrderCancelled,
   OrderDone, BBO). WAL is authoritative; failure panics.
5. **Maintain cancel index** — walk `book.events()` and update
   the O(1) cancel index (see below).
6. **Fan out** — best-effort CMP send of each event to Risk
   and to Marketdata (with the MD filter above). Drops are
   recovered by NAK / DXS-TCP replay; CMP send errors log
   and continue.

Cancel frames follow steps 3–6 with `cancel_order` (no
dedup/accept records).

## O(1) Cancel Index

The ME holds `FxHashMap<(user_id, oid_hi, oid_lo), slab_handle:
u32>`. It is rebuilt incrementally from `book.events()` after
every match cycle: `OrderInserted` inserts, `OrderDone` removes
(covers both filled and cancelled terminal transitions). Cancel
processing looks up the slab handle in O(1) instead of an O(n)
slab scan, then verifies the slab slot still matches (defensive
against drift) before unlinking.

## TIF and Flag Semantics

- **GTC** — match against the opposite side; remainder rests
  via `insert_resting` and emits `OrderInserted`.
- **IOC** — match the same way; remainder is dropped without
  resting and emits `OrderDone{reason=REASON_CANCELLED}` with
  the filled/remaining split.
- **FOK** — pre-check `available_liquidity` against the limit
  price; if not fully fillable, emit `OrderFailed{reason=
  FAIL_FOK}` and do nothing else.
- **POST_ONLY** — before matching, if the order would cross
  the best opposite tick, emit `OrderCancelled{reason=
  CANCEL_POST_ONLY}` and return; never matches, never rests.
- **REDUCE_ONLY** — requires an existing position on the
  opposite side; clamps `remaining_qty` to `|net_qty|`. If no
  position or wrong side, emit `OrderFailed{reason=
  FAIL_REDUCE_ONLY}`.

## BBO Derivation

`process_new_order` saves `(best_bid_tick, best_ask_tick)` on
entry and re-checks at the end of every match cycle. If either
changed, it emits an `Event::BBO{bid_px, bid_qty, ask_px,
ask_qty}` derived from the head order at each best level. The
cancel path does the same comparison after `cancel_order`.

## ORDER_DONE Exactly Once (Invariant #2)

Every order reaches exactly one terminal event:

- Full fill — `OrderDone{REASON_FILLED}` emitted inside
  `match_at_level` after the last fill consumes the rest.
- IOC remainder — `OrderDone{REASON_CANCELLED}` emitted after
  matching, no insert.
- User cancel — `OrderDone{REASON_CANCELLED}` emitted in the
  cancel path after `OrderCancelled`.
- POST_ONLY cross — `OrderCancelled{CANCEL_POST_ONLY}` only;
  the order never existed in the book.
- FOK / validation / reduce-only reject — `OrderFailed`; no
  `OrderDone` (the order was never accepted into the book).

## Determinism

- Fixed-point arithmetic only
- Single-threaded per symbol
- No external I/O in the core loop

## Config

- Env-only: symbol_id, tick/lot, decimals (base config).
- Optional Postgres config polling (every 10 minutes) to
  apply scheduled symbol config updates when DATABASE_URL
  is set.

## Notes

This spec describes behavior; tile composition lives in
PROCESS.md. Implementation details live in ORDERBOOK.md and
WAL/DXS docs.
