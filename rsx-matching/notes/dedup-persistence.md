# Dedup is WAL-persisted, not memory-only

**Domain term.** *Deduplication* rejects a re-sent order so it is not
executed twice. Clients re-send an order when they don't get an ack in
time (`specs/2/49-webproto.md`), so the same `(user_id, order_id)` can
legitimately arrive more than once. The invariant at stake is
**exactly-one completion** per order (`../CLAUDE.md` Correctness
Invariant #2): an order must not fill twice.

## Problem

A memory-only dedup set works fine until the ME restarts. Two facts make
a naive rebuild insufficient:

- The dedup **window is 1 hour** (`DEDUP_WINDOW`), but the book
  **snapshot cadence is ~10 s**. So the snapshot, and the post-snapshot
  WAL replay that rebuilds the book, only see the last ~10 s of orders.
- A client whose order was accepted 20 minutes ago, then times out and
  re-sends after an ME crash, would find its `(user, oid)` *not* in a
  10-second-deep set — and the order would **double-execute**, violating
  exactly-one-completion.

Durability of the fill is not enough; the *acceptance record* has to be
authoritative for dedup too.

## Fix

Dedup detection is **WAL-persisted**: every accepted order writes a
`RECORD_ORDER_ACCEPTED` before it matches, and on recovery the entire
1-hour window is rebuilt from those records — independent of the
snapshot.

```
recovery:
  rebuild_dedup_window(wal):                 // scans retained WAL (≤4 h)
    for each RECORD_ORDER_ACCEPTED in seq order:
      age = now - rec.ts_ns
      dedup.seed(user, oid, inserted_ago = age)   // skips if age ≥ 1 h
```

`rebuild_dedup_window` (`wal.rs`) scans from the earliest retained WAL
file, seeds each key with its *remaining* TTL keyed off the record's
ME-stamped `ts_ns`, and — because it walks in seq (ascending time) order
— feeds `dedup.seed` oldest-first so the pruning queue stays ordered
(`dedup.rs::seed`). It covers pre- *and* post-snapshot records, is
idempotent (a set), and is a one-time cold-path scan bounded by WAL
retention (4 h ≫ the 1 h window). The forward book-replay pass
deliberately does **not** also seed dedup — one owner for the whole
window keeps the pruning queue ordered.

On the hot path, dedup stays cheap: the tracker reads a **cached clock**
the loop refreshes every 1024 spins rather than sampling `Instant::now()`
per order — the hour-scale window tolerates the coarse tick
(`notes/zero-heap.md`).

## Cost it removes

Double-execution of a legitimately re-sent order after a restart — i.e.
a violation of exactly-one-completion that a memory-only or
snapshot-only dedup set would allow for any order older than one snapshot
interval.

## Cite

- `wal.rs::rebuild_dedup_window`; `dedup.rs` (`DEDUP_WINDOW`, `seed`,
  `refresh_clock`); `main.rs` (recovery call sites);
  `ARCHITECTURE.md` § "Deduplication"; bugs.md
  `ME-SNAPSHOT-NO-INDEX-DEDUP-REBUILD`.
- `../CLAUDE.md` Correctness Invariant #2; `specs/2/49-webproto.md`
  (client resend on no-ack-timeout).
