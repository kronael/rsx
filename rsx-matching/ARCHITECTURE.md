# rsx-matching Architecture

Matching engine process. One instance per symbol. Receives
orders from Risk via casting/UDP, matches against the orderbook,
fans out events to Risk and Marketdata. Authoritative writer
of fills, accepts, cancels, order-failed, and config-applied
records to the WAL.

Specs: `specs/2/17-matching.md`, `specs/2/45-tiles.md` §3.1.

## Module Layout

| File | Purpose |
|------|---------|
| `main.rs` | Binary: casting setup, WAL init, match loop, event routing, cancel index |
| `wire.rs` | `OrderMessage` — `#[repr(C)]` casting wire type for inbound orders |
| `dedup.rs` | `DedupTracker` — 5-minute sliding-window duplicate detection |
| `config.rs` | `poll_scheduled_configs()`, `write_applied_config()` — Postgres config polling |
| `wal_integration.rs` | `publish_events()`, `flush_if_due()`, `write_events_to_wal()` (replay + bench helper) |

## Tile Shape (Degenerate Tile)

Per `specs/2/45-tiles.md` §3.1, matching is a **degenerate
tile**: one core-pinned thread, no SPSC rings. The whole
process is one tile. All work — casting I/O, dedup, matching,
WAL append, casting fan-out — happens on the pinned core. No
intra-process IPC, no cross-thread queues on the hot path.

Pinning: `RSX_ME_CORE_ID` selects the core
(`core_affinity::set_for_current`, `main.rs:195-200`).

## Main Loop

Tight busy-spin on the pinned core (`main.rs:403`):

```
loop {
    1. cast_receiver.try_recv()       // OrderRequest or CancelRequest from risk
    2. dedup.check_and_insert()       // 5-minute sliding window
    3. wal.append_framed(ORDER_ACCEPTED) // authoritative — panic on err
    4. process_new_order(book)        // match against book; events in fixed buffer
    5. publish_events()               // one-CRC fan-out: WAL + cast(risk) + cast(mkt)
    6. update_order_index(events)     // O(1) cancel index maintenance
    7. flush_if_due(wal, 10ms)        // periodic WAL flush
    8. poll_scheduled_configs()       // every 10 minutes (Postgres)
}
```

Cancels share the loop: `process_cancel()` consults
`order_index` for O(1) slab lookup, then runs steps 4-6 via
the same `publish_events` fan-out.

## O(1) Cancel Index

`type OrderKey = (u32, u64, u64); // (user_id, oid_hi, oid_lo)`
`FxHashMap<OrderKey, slab_handle: u32>` rebuilt incrementally
from `book.events()` after every match and cancel cycle
(`update_order_index`, `main.rs:59`).

- `OrderInserted` → insert `(key, handle)`
- `OrderDone` → remove `key` (fires on fully-filled or
  cancelled, so the index never leaks)

Replaces the prior linear slab scan over a 65 536-slot arena.
A defensive check inside `process_cancel` still verifies the
slab slot matches `(user, oid)` after the index hit; mismatch
warns and bails without crashing.

Commit: `cdc9360`.

## WAL Append: Crash on Failure

Matching is the authoritative writer for the fill path. Every
WAL append uses `.expect(...)` with a named-invariant message
(commit `82a9206`):

- `ORDER_ACCEPTED` — "violates 6-consistency.md invariant 7
  (WAL persistence) and breaks dedup on replay"
- Event path (Fill / OrderInserted / OrderDone) — "violates
  6-consistency.md invariant 1 (totally-ordered events) and
  ordering rule 'Fills precede ORDER_DONE'"
- `CANCEL` — "violates invariant 1 and invariant 5
  (ORDER_DONE commit boundary)"
- `ORDER_FAILED` (duplicate-reject) — "violates invariant 7"
- `CONFIG_APPLIED` — "violates invariant 7; CONFIG_APPLIED
  must precede casting fan-out"

Design choice: matching engine is authoritative; silently
losing a fill violates Invariants #1 and #2. Crash, let the
replica take over, replay from WAL tip. casting fan-out sends
remain best-effort (receivers recover via NAK / TCP replay)
and only warn on failure.

## Event Fanout

Fixed array `[Event; MAX_EVENTS]` (MAX_EVENTS = 65_536,
heap-boxed) on the orderbook struct, reset per match cycle.
Two independent `CastSender`s:

- ME → Risk: fills, BBO, order done/failed (all events)
- ME → Marketdata: inserts, cancels, fills

`publish_events` (`wal_integration.rs`) prepares each record once
(single CRC + seq) and fans the resulting `Framed` to WAL + cast
+ (optionally) mkt with `send_framed` / `append_framed` — no
re-CRC per destination. Routing per event type:

| Event | WAL | cast (risk) | mkt |
|---|---|---|---|
| `Fill` / `OrderInserted` / `OrderCancelled` | yes | yes | yes |
| `OrderDone` | yes | yes | no |
| `OrderFailed` | yes | no | no |
| `BBO` | no (derived on replay) | yes | yes |

`BBO` is the one event not framed by the WAL; both senders use
their own seq counter via `CastSender::send` for that record.

## Deduplication

`DedupTracker` keeps `(user_id, oid_hi, oid_lo)` for a
5-minute sliding window. On replay, the dedup set is
rebuilt from `RECORD_ORDER_ACCEPTED` records in the WAL —
duplicate detection is WAL-persisted, not a memory-only
guard.

## Config Hot Reload

`poll_scheduled_configs()` queries `symbol_config_schedule`
every 10 minutes (`main.rs:559`, `600` seconds). When a new
version is due, the matcher writes `CONFIG_APPLIED` to WAL
**before** fanning out to casting, then applies tick/lot
changes live. On startup, the current version emits one
`CONFIG_APPLIED` record too.

## Architectural Decisions

**Runtime: tile (pinned thread) + tokio sidecar.** The
matching loop is the lowest-latency stage of the system —
~340 ns p50 for dedup + WAL accept + match + WAL events.
Network I/O multiplexing does not appear in the inner loop;
the only sockets are one `CastReceiver` (orders in) and two
`CastSender`s (events to risk and marketdata). With nothing
to multiplex, the cheapest reactor is no reactor: one
pinned thread, busy-spin, all work inline on the cache-warm
core. This is the **degenerate tile** in
[`../specs/2/45-tiles.md`](../specs/2/45-tiles.md) §3.1.

A tokio sidecar handles the cold path — `poll_scheduled_configs()`
queries Postgres every 10 minutes for symbol config updates.
That is blocking I/O and explicitly off the hot loop. See
[`../notes/tiles.md`](../notes/tiles.md) for the broader
pattern.

## Cold-start snapshot + WAL replay (wal_integration.rs)

Snapshots are written every 10 s with atomic rename
(`snapshot.bin` + `wal_seq.txt` sidecar). Between snapshots,
SIGKILL would lose every fill — recovery replays the WAL from
`sidecar + 1`:

- `RECORD_ORDER_ACCEPTED` → re-runs `process_new_order` to
  deterministically regenerate fills + side-effect events.
- `RECORD_ORDER_CANCELLED` → `book.cancel_order(handle)`
  against the reconstructed `order_index`.
- Other record types (Fill, OrderInserted, OrderDone, BBO)
  are skipped — they are side effects of accepted-order replay.

`replay_wal_after_snapshot` returns the highest WAL seq applied;
caller seeds `WalWriter::next_seq = ret + 1` so subsequent live
writes never reuse a replayed seq.

## FAULTED → replay recovery (replay.rs)

When `CastReceiver::try_recv` returns `CastRecv::Faulted`, the
matching tile must NOT silently advance past lost orders. The
recovery path:

1. Open a `ReplicationConsumer` against the risk producer's
   replication server (env: `RSX_ME_REPLICATION_ADDR`).
2. Drain Phase 1 records (seq > `last_delivered_seq`) until
   `RECORD_CAUGHT_UP` arrives.
3. Apply each `OrderRequest` / `CancelRequest` to the in-tile
   state via `apply_replayed_record`.
4. Call `cast_receiver.reset_after_replay(new_tip)` to resume
   live UDP delivery from `new_tip + 1`.

Downstream re-emit is intentionally skipped — risk and marketdata
recover their own streams via their own replay paths. The matching
tile is internally consistent after `drain_dxs_replay_into_book`
returns. Live ingestion samples `me_in` / `me_dedup_done` /
`me_wal_*` / `me_match_done`; replay skips these probes (they
target live tail latency).
