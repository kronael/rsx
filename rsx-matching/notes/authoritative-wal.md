# Authoritative WAL: crash on fill-path failure, best-effort on send

**Domain term.** The *WAL* (write-ahead log) is the append-only file of
records — accepts, fills, cancels — that is the system's durable truth.
*casting* is the UDP fan-out that pushes those records live to risk and
marketdata. The matching engine is the **authoritative writer**: once it
persists a fill to its WAL, that fill *happened*, and everyone else's
copy is derived from it.

## Problem

Two failures can hit the publish path, and the safe reaction to each is
opposite:

- A **WAL append fails** (disk error). If we log-and-continue, we have
  matched an order but lost the only durable record of it. Recovery
  replays the WAL and this fill vanishes — `seq` gaps, positions that
  don't equal the sum of fills, a book that can't be rebuilt. Silent and
  unrecoverable.
- A **cast send fails** (UDP hiccup, slow consumer). If we crash on this,
  we take down the matching engine — the most latency-critical process —
  over a loss the receiver can already recover on its own.

A uniform policy (crash on both, or warn on both) is wrong for one of
them.

## Fix

Split the policy by who owns recovery.

```
publish_events:
  wal.append_framed(&framed).expect("… violates 6-consistency inv. 1/5/7")   ← CRASH
  cmp.send_framed(&framed)   → on Err: warn!(…), continue                     ← best-effort
  mkt.send_framed(&framed)   → on Err: warn!(…), continue                     ← best-effort
```

- **WAL appends `.expect(...)`** with a message naming the exact
  `specs/2/6-consistency.md` invariant broken — accept: inv. 7 (ME
  persists orderbook); events: inv. 1 (totally-ordered events) + "Fills
  precede ORDER_DONE"; cancel: inv. 1 + inv. 5 (ORDER_DONE commit
  boundary); duplicate/failed: inv. 7. A crash lets the replica take over
  and replay from the WAL tip — the one recovery path the system already
  exercises on every restart (`specs/2/6-consistency.md` §5).
- **Cast sends warn and continue.** Receivers recover missed records
  in-band (NAK) or via TCP WAL replay from the ME's replication server.
  A dropped datagram is not a lost fill — the fill is safe in the WAL.

(Note: the invariant numbers above are `specs/2/6-consistency.md`'s own
list, distinct from the 10 system-wide invariants in `../CLAUDE.md`.)

## Cost it removes

The unrecoverable case (a silently lost fill) *and* the spurious case (a
crash over a transport blip the consumer can recover). Each failure gets
the reaction that matches who owns fixing it.

## Cite

- `main.rs` (accept/duplicate/config `.expect` sites);
  `wal.rs::publish_events` → `FanoutSink` (append then send);
  `ARCHITECTURE.md` § "WAL Append: Crash on Failure".
- `specs/2/6-consistency.md` §5 (crash behavior) + Key Invariants;
  commit `82a9206`.
