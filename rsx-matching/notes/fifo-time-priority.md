# FIFO time-priority (the fairness invariant)

**Domain term.** Within one price level, orders rest in a queue. *Time
priority* means the order that arrived first fills first — first-in,
first-out. This is the fairness promise of a continuous limit-order book:
at a given price, you are not overtaken by someone who arrived later.

## Problem

If matching picked an arbitrary resting order at the best price — or a
"cheapest to touch" one — a latecomer could jump the queue ahead of an
earlier order at the same price. That is unfair and, worse,
non-deterministic: two replays of the same order stream could produce
different fills, breaking crash recovery (the book is rebuilt by
replaying the WAL; see `notes/dedup-persistence.md`).

## Fix

The queue discipline itself is a **rsx-book** decision: `insert_resting`
links each new order at `level.tail`; `match_at_level` walks from
`level.head` (`specs/2/6-consistency.md` invariant #3 cross-reference;
`specs/2/21-orderbook.md` § "FIFO Within Price Level"). rsx-matching's
own job is to **preserve that order end-to-end**, so the fairness the
book computes survives into the WAL and back out of replay:

```
match order ─▶ book event buffer ─▶ emit_events walks buffer in order
             (FIFO fills)            └▶ WAL append (append_framed)
                                     └▶ same Framed to risk + mkt
```

`emit_events` (`wal.rs`) iterates `book.events()` in buffer order and
calls `append_framed` before fanning the same bytes to the cast
destinations — so on-disk sequence, on-wire sequence, and match order are
one and the same. This is what makes "Fills precede ORDER_DONE"
(`specs/2/6-consistency.md` invariant #1 / §2 ordering rule) hold on
every stream. Replay re-runs `process_new_order` on the accepted-order
records in seq order (`replay_wal_after_snapshot`), regenerating the
identical fill sequence.

## Cost it removes

Queue-jumping and replay non-determinism. Because match order is never
re-ordered on its way to disk/wire, and replay re-derives it from the
same input, one order stream always yields one fill sequence.

## Cite

- Mechanism (owned elsewhere): `rsx-book` `insert_resting` /
  `match_at_level`; `specs/2/21-orderbook.md` § "FIFO Within Price
  Level (Invariant #3)".
- Preservation here: `wal.rs::emit_events` / `publish_events`;
  `specs/2/6-consistency.md` invariants #1, #3, #5.
