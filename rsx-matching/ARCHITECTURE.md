# rsx-matching Architecture

A matching engine is the part of an exchange that pairs orders.
An incoming buy that meets a resting sell (or a sell that meets a
resting buy) becomes a **fill** — a trade; anything that doesn't
cross rests in the book and waits. This process does exactly that
for one symbol: it takes orders from Risk, matches them against
the resting orderbook, decides the fills, records them, and tells
Risk and Marketdata what happened.

The match is **price-agnostic** — it pairs orders on their own
limit prices alone, with no notion of mark or index price (those
live in Risk and Mark). And it is **flat**: the 2026-07-03 bench
measures the match at ~30 ns whether the book holds one resting
order or 100 000; see Measured Performance below. Scale-out is by
symbol: one matching-engine instance per
tradeable symbol, added independently of user-shard (Risk)
scale-out.

Matching is the authoritative writer of fills, accepts, cancels,
order-failed, and config-applied records to the WAL: once it
persists a fill, that fill happened.

Specs: `specs/2/17-matching.md`, `specs/2/45-tiles.md` §3.1.

## Trust Boundary

The matching engine **does not validate user input** — by design.
By the time an order reaches this process it has already passed
two upstream gates: the **gateway** authenticates the client (JWT,
TLS) and checks structural well-formedness, and the **risk tile**
checks margin and position (`rsx-risk`, the pre-trade authority).
ME assumes its inputs are well-formed and in-shard, and does not
re-validate on the hot path. This is the single-owner rule from
the repo trust-boundary policy ("The matching engine doesn't
validate user input" — `CLAUDE.md`; `specs/2/47-validation-edge-cases.md`
owns where each check lives). Adding re-validation here would
duplicate an upstream owner and slow the hot loop for no
correctness gain.

The one thing ME *is* strict about is its own output: every WAL
append on the fill path crashes on failure rather than silently
dropping a record (see WAL Append below), because ME is the
authoritative writer and a lost fill is unrecoverable.

## Measured Performance

In-process microbenchmarks, single box, no UDP/WS — compute
floors, not wire-to-wire latencies. Captured 2026-07-03
([`../reports/20260703_matching-benches.md`](../reports/20260703_matching-benches.md),
`cargo bench -p rsx-matching`, p50 over 50 samples, timed thread
pinned to core 2).

| Point | p50 | What |
|---|---|---|
| `match_by_depth/n=1` | ~30 ns | the match itself |
| `match_by_depth/n=100000` | ~30 ns | **same at 100k resting — depth-independent** |
| `dedup/hit_duplicate` | 3.7 ns | duplicate order rejected |
| `dedup/insert_new` | 147 ns | FxHashMap insert |
| `wal_events/append_1_fill` | 84 ns | serialize 1 fill (no fsync) |
| `me_accept_path/full` | **266 ns** | full accept: dedup + WAL accept + match + WAL events + index, 1 fill |
| `me_throughput/orders` | 281 ns | ≈ **3.6M orders/s** (1 fill each) |
| `wal_replay_30k_records` | 32.8 ms | ≈ 915k records/s cold replay |

The order-type and multi-level-sweep figures in that report are
quarantined (a 10k-level fixture artifact, flagged for a Phase-2
audit) — do not cite them as per-order-type latency. The
single-op and full-accept figures above are the trusted set. These
are indicative (shared 4-core docker host, cluster stopped); re-run
on a quiet box for a citable baseline. The full GW→ME→GW
round-trip is transport-bound (~4 casting hops), not compute-bound.

## Module Layout

| File | Purpose |
|------|---------|
| `main.rs` | Binary: casting setup, WAL init, match loop, event routing, cancel index |
| `wire.rs` | `OrderMessage` — `#[repr(C)]` casting wire type for inbound orders |
| `dedup.rs` | `DedupTracker` — 5-minute sliding-window duplicate detection |
| `config.rs` | `poll_scheduled_configs()`, `write_applied_config()` — Postgres config polling |
| `wal.rs` | `publish_events()`, `flush_if_due()`, `write_events_to_wal()` (replay + bench helper) |

## Execution: a single pinned loop

Matching is **one core-pinned thread running a loop, no SPSC
rings** (the "degenerate tile" of `specs/2/45-tiles.md` §3.1).
All work — casting I/O, dedup, matching, WAL append, casting
fan-out — runs inline on the pinned core; nothing crosses a
thread on the hot path.

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

`publish_events` (`wal.rs`) prepares each record once
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

**Runtime: a single pinned loop + tokio sidecar.** The
matching loop is the lowest-latency stage of the system —
~266 ns p50 for the full accept path (dedup + WAL accept +
match + WAL events + order-index update), per the 2026-07-03
bench (`me_accept_path/full`; see Measured Performance).
Network I/O multiplexing does not appear in the inner loop;
the only sockets are one `CastReceiver` (orders in) and two
`CastSender`s (events to risk and marketdata). With nothing
to multiplex, the cheapest reactor is no reactor: one
pinned thread, busy-spin, all work inline on the cache-warm
core — a loop, not a tile (see "Execution: a single pinned
loop" above).

A tokio sidecar handles the cold path — `poll_scheduled_configs()`
queries Postgres every 10 minutes for symbol config updates.
That is blocking I/O and explicitly off the hot loop. See
[`../docs/concepts/tiles-and-pinning.md`](../docs/concepts/tiles-and-pinning.md) for the broader
pattern.

## Cold-start snapshot + WAL replay (wal.rs)

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

## FAULTED → skip-the-gap

When `CastReceiver::try_recv` returns `CastRecv::Faulted` (a gap
that outran in-band NAK recovery), the matching loop **skips the
gap and resumes live** — it does NOT replay and does NOT panic.
On FAULTED it counts the skipped seqs (`gauges.drops`), warns with
the gap range, then calls `cast_receiver.reset_after_replay(
gap_end_inclusive)` to resume live UDP delivery from
`gap_end_inclusive + 1`.

This is sound because the risk→ME **order** stream is recovered at
the application layer, not the transport:

- A dropped pre-ack order is re-sent by the client (no-ack-within-
  timeout, `specs/2/49-webproto.md`) and deduped on the ME's WAL
  (`RECORD_ORDER_ACCEPTED`) — exactly-once.
- The ME **re-sequences on output** (its own WAL seq), so an
  inbound gap is never an output gap: risk / recorder / marketdata
  still see a contiguous ME stream.

Fill delivery in the other direction (ME→risk) is what genuinely
needs recovery, and it has its own path: the ME runs a WAL
replication **server** (`RSX_ME_REPLICATION_BIND_ADDR`) that risk
pulls from on risk-side FAULTED. ME **cold-start** recovery replays
the ME's own local WAL (snapshot + `replay_wal_after_snapshot`);
neither depends on a remote order-replay consumer, which is why the
old `RSX_ME_REPLICATION_ADDR` pull path was removed.

Live ingestion samples `me_in` / `me_dedup_done` / `me_wal_*` /
`me_match_done` on the hot path.
