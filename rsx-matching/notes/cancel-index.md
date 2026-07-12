# O(1) cancel index

**Domain term.** Resting orders live in a *slab* — a pre-allocated array
of fixed slots, addressed by a `handle` (the slot index), in `rsx-book`.
A *cancel* arrives keyed by `(user_id, order_id_hi, order_id_lo)` — the
client's identity for the order — not by slab handle. So a cancel must
translate "this user's order id" into "this slab slot".

## Problem

The slab is capacity 65 536. The naive translation scans every slot
looking for the one whose `(user, oid)` matches — `O(65 536)` per cancel,
a full sweep of the arena for a single deletion. Cancels are common
(quote churn), so this scan sits on the hot path and grows with slab
capacity.

## Fix

Keep a side index `FxHashMap<(user_id, oid_hi, oid_lo) → handle>` and
maintain it incrementally from the book's own events — not from a
separate bookkeeping pass:

```
after every match / cancel cycle:
  update_order_index(book.events()):
    OrderInserted{handle, user, oid…} → index.insert((user,oid), handle)
    OrderDone{user, oid…}             → index.remove((user,oid))
```

`OrderDone` fires on *both* full-fill and cancel, so the index never
leaks a stale handle (`wal.rs::update_order_index`, the single
maintainer shared by the live loop, replay, and the bench harness). A
cancel then does `order_index.get(&key)` — `O(1)` — and hands the handle
to `book.cancel_order_checked`, which re-validates capacity + active +
matching `(user, oid)` in rsx-book. The index and slab are kept in
lockstep, so the checked cancel always succeeds; a false return means
index/slab drift — it trips a `debug_assert` and, in release, warns and
bails rather than emitting cancel events for a slot it did not cancel
(`main.rs::process_cancel`).

**On restart**, a snapshot restores the slab but not this map, so it is
rebuilt: `rebuild_order_index_from_book` scans the restored slab once at
startup and re-keys the active slots, then WAL replay layers its
`OrderInserted`/`OrderDone` deltas on top
(`notes/dedup-persistence.md` covers the sibling dedup rebuild).

## Cost it removes

The `O(65 536)` slab scan on every cancel, replaced by one hash lookup.
The maintenance is free-riding on events the book already emits, so it
adds no separate pass on the accept path.

## Cite

- `wal.rs::update_order_index`; `main.rs::process_cancel`,
  `rebuild_order_index_from_book`; `ARCHITECTURE.md` § "O(1) Cancel
  Index"; commit `cdc9360`.
