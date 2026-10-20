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
live in Risk and Mark). The fixed-work sweep observes no depth-related
increase while preloaded resting depth changes. Scale-out is by symbol: one
matching-engine instance per
tradeable symbol, added independently of user-shard (Risk)
scale-out.

Matching is the authoritative writer of fills, accepts, cancels,
order-failed, and config-applied records to the WAL: once it
persists a fill, that fill happened.

Specs: `specs/2/17-matching.md`, `specs/2/45-tiles.md` §3.1.

## Trust Boundary

The matching engine **does not revalidate authentication, authorization,
margin, or position limits** — by design.
By the time an order reaches this process it has already passed
two upstream gates: the **gateway** authenticates the client (JWT,
TLS) and checks structural well-formedness, and the **risk tile**
checks margin and position (`rsx-risk`, the pre-trade authority).
ME assumes the caller is authenticated and risk-approved. `rsx-book` still
enforces structural price/quantity sign, tick, and lot invariants needed to
protect its own state. ME does not duplicate the upstream policy checks. This
is the single-owner rule from
the repo trust-boundary policy ("The matching engine doesn't
validate user input" — `CLAUDE.md`; `specs/2/47-validation-edge-cases.md`
owns where each check lives). Adding re-validation here would
duplicate an upstream owner and slow the hot loop for no
correctness gain.

The one thing ME *is* strict about is its own output: every WAL
append on the fill path crashes on failure rather than silently
dropping a record (see WAL Append below), because ME is the
authoritative writer and a lost fill is unrecoverable.

## Performance evidence

The benchmark harnesses live in `benches/`. Dated runs live in
`../reports/`. The 2026-07-16 matching report records Criterion medians but
not p99, so its exact latency table is not repeated here. Published latency
tables require p50 and p99 from the same run and workload.

## Module Layout

| File | Purpose |
|------|---------|
| `main.rs` | Binary entry: panic handler, tracing, `app::run()` |
| `app.rs` | Composition root: env + Postgres config, core pin, `recover` call, cast sockets, replication and config sidecars, health server, hand-off to `cast_loop::run` |
| `cast_loop.rs` | `LoopState` and `run`, the pinned busy-spin loop: recv dispatch, FAULTED skip, dedup cleanup, config apply, WAL flush, checkpoint cadence, heartbeats |
| `order.rs` | `process_order` — the order-request handler: dedup gate, accept record, match, fan-out, cancel-index update, recenter |
| `cancel.rs` | `process_cancel` — the cancel-request handler: index lookup, checked cancel, event fan-out |
| `wire.rs` | `OrderMessage` — `#[repr(C)]` casting wire type for inbound orders |
| `dedup.rs` | `DedupTracker` — 1-hour sliding-window duplicate detection |
| `config.rs` | Env config load; the config sidecar (`run_config_sidecar`: `poll_scheduled_configs`, `write_applied_config`) and the rings it shares with the loop (`config_link`); the loop-side `apply_due_configs` and `emit_config_applied` |
| `checkpoint.rs` | `MatchingCheckpointV1`, `CheckpointWriter` worker thread, `load_checkpoint` / `save_checkpoint` — Avro `snapshot.bin` persistence |
| `recovery.rs` | `recover` — checkpoint restore, WAL replay, dedup-window rebuild, next WAL seq; `RecoveryError` |
| `wal.rs` | WAL fan-out (`publish_events`), flush, replay, dedup-window rebuild, cancel-index maintenance |

## Execution: a single pinned loop

Matching is **one core-pinned thread running a loop, no SPSC
rings** (the "degenerate tile" of `specs/2/45-tiles.md` §3.1).
All work — casting I/O, dedup, matching, WAL append, casting
fan-out — runs inline on the pinned core; nothing crosses a
thread on the hot path.

Pinning: `RSX_ME_CORE_ID` selects the core. `app::run` calls
`setup_hot_thread` last, after the replication, checkpoint and config
sidecar threads exist: a Linux thread inherits its creator's affinity, so a
sidecar spawned after the pin would share the loop's core.

## Main Loop

Tight busy-spin on the pinned core (`cast_loop::run`):

```
loop {
    1. cast_receiver.try_recv()       // OrderRequest or CancelRequest from risk
    2. dedup.check_and_insert()       // 1-hour sliding window
    3. wal.append_framed(ORDER_ACCEPTED) // authoritative — panic on err
    4. process_new_order(book)        // match against book; events in fixed buffer
    5. publish_events()               // one-CRC fan-out: WAL + cast(risk) + cast(mkt)
    6. update_order_index(events)     // O(1) cancel index maintenance
    7. flush_if_due(wal, 10ms)        // WAL flush every 10 ms or 1000 records
    8. apply_due_configs()            // configs the sidecar polled from Postgres (ring pop)
}
```

Cancels share the loop: `process_cancel()` consults
`order_index` for O(1) slab lookup, then runs steps 4-6 via
the same `publish_events` fan-out.

## O(1) Cancel Index

`type OrderKey = (u32, u64, u64); // (user_id, oid_hi, oid_lo)`
`FxHashMap<OrderKey, slab_handle: u32>` rebuilt incrementally
from `book.events()` after every match and cancel cycle
(`update_order_index`, `wal.rs`).

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

A full WAL buffer is not a failure: `WouldBlock` from the append
flushes and retries, so the producer stalls instead of exiting
(`specs/2/48-wal.md` § Hard Backpressure Rules). A flush that errors
is retried every millisecond until it succeeds, with one warning
per episode; the loop does not buffer on past a failed flush.

## Event Fanout

The orderbook reserves `MAX_EVENTS` event slots once
(`MAX_EVENTS = 65_536`) and reuses the storage per match cycle.
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
1-hour sliding window. On replay, the dedup set is
rebuilt from `RECORD_ORDER_ACCEPTED` records in the WAL —
duplicate detection is WAL-persisted, not a memory-only
guard.

## Config Hot Reload

The config sidecar thread (`me-config`, `run_config_sidecar`) owns a
current-thread tokio runtime and the Postgres client. Every 10 minutes
(`CONFIG_POLL_INTERVAL`, 600 seconds, the first poll 600 seconds after
start) it runs `poll_scheduled_configs()` against
`symbol_config_schedule` and pushes each due version, oldest first, onto
a 64-slot `rtrb` ring. A full ring leaves the version scheduled for the
next poll. The pinned loop pops that ring each spin
(`apply_due_configs`): it applies the tick/lot change to the book, hands
the applied config back on a second 64-slot ring, and writes
`CONFIG_APPLIED` to the WAL **before** fanning out to casting. The
sidecar drains the second ring at least once a second and upserts
`symbol_config_applied`; a failed upsert only warns. The loop does ring
pops and WAL appends and no Postgres I/O, so a slow or stalled database
delays only config delivery. On startup, the current version emits one
`CONFIG_APPLIED` record too.

## Architectural Decisions

**Runtime: a single pinned loop + tokio sidecar.** The
matching loop is the lowest-latency stage of the system —
256.25 ns median for the full accept path (dedup + WAL accept +
match + WAL events + order-index update), per the 2026-07-16
bench (`me_accept_path/full`; see Measured Performance). That
scenario fills one lot without moving the BBO; an accept that
changes the best price adds one WAL record.
Network I/O multiplexing does not appear in the inner loop;
the only sockets are one `CastReceiver` (orders in) and two
`CastSender`s (events to risk and marketdata). With nothing
to multiplex, the cheapest reactor is no reactor: one
pinned thread, busy-spin, all work inline on the cache-warm
core — a loop, not a tile (see "Execution: a single pinned
loop" above).

A tokio sidecar thread handles the cold path — it queries Postgres
every 10 minutes for symbol config updates and writes the applied
rows, handing configs to the loop over an SPSC ring (see "Config Hot
Reload"). That I/O is off the hot loop. See
[`../docs/concepts/tiles-and-pinning.md`](../docs/concepts/tiles-and-pinning.md) for the broader
pattern.

## Cold-start snapshot + WAL replay (checkpoint.rs, recovery.rs, wal.rs)

The book is derived state, rebuilt from the WAL. A snapshot is a
periodic **checkpoint** that bounds how much WAL a cold start replays:
without it, recovery would replay from seq 1 (every order the symbol
ever took); with it, recovery loads the book at seq N (the
checkpoint's embedded WAL sequence) and replays only records `> N` — ≤ 10 s
of WAL.
Fills are durable in the WAL either way; the snapshot bounds replay
time, not durability. It is the I-frame; the WAL records between
snapshots are the deltas.

Every 10 seconds the ME briefly captures an owned `BookSnapshotV1` from the
single-threaded book. Capture preserves FIFO and allocator order but owns no
live pointers. Matching resumes as soon as capture returns.

A named background worker wraps that value in
`MatchingCheckpointV1 { schema_version, wal_seq, book }` and writes one Apache
Avro Object Container File. The writer schema is embedded in the file and the
ME supplies the authoritative WAL sequence. The worker fsyncs
`snapshot.bin.tmp`, atomically renames it to `snapshot.bin`, then fsyncs the
directory. A crash before rename leaves the prior generation intact. Shutdown
queues a final capture and joins the worker. Capture is refused mid-recenter.

Recovery detects Avro by its container magic. The legacy checkpoint envelope
and RXSN v2/v3 book snapshots remain readable.

Replay from `embedded WAL sequence + 1`:

- `RECORD_ORDER_ACCEPTED` → re-runs `process_new_order`, regenerating
  fills + side-effect events deterministically.
- `RECORD_ORDER_CANCELLED` → `book.cancel_order(handle)` against the
  reconstructed `order_index`.
- Fill / OrderInserted / OrderDone / OrderFailed / ConfigApplied / BBO →
  skipped (side effects of accepted-order replay).
- Any other record type → skipped with one warning per type.

`recovery::recover` runs the sequence and returns the book, order index,
dedup window and next WAL seq (`replay_wal_after_snapshot`'s highest seq
read, plus one; `checkpoint seq + 1` when nothing newer is on disk). The
caller seeds `WalWriter::next_seq` from it so live writes never reuse a
seq.

Recovery fails loud: a WAL read error, or an `ORDER_ACCEPTED` /
`ORDER_CANCELLED` record whose payload is shorter than its struct, is a
`RecoveryError` and the process exits before it binds a socket. The
dedup-window rebuild scans every retained `ORDER_ACCEPTED`, so an
undecodable one anywhere in the retained WAL stops boot as well. A WAL
that is missing or empty is a clean boot.

Capture is O(active orders). Its p50 and p99 are not measured.

## FAULTED → skip-the-gap

System-level delivery guarantees live in
[`GUARANTEES.md` §1.0](../GUARANTEES.md#10-delivery-guarantees-by-stream).
This section is only the ME-side implementation of the risk→ME
order-stream rule: inbound order gaps are drop-safe; ME→risk fills
are not.

When `CastReceiver::try_recv` returns `CastRecv::Faulted` — a gap
that outran in-band NAK recovery — the matching loop **skips the
gap and resumes live**: it does NOT replay and does NOT panic. On
FAULTED it counts the skipped seqs (`gauges.drops`), warns with the
gap range, then calls `cast_receiver.reset_after_replay(
gap_end_inclusive)` to resume live UDP delivery from
`gap_end_inclusive + 1`.

This is sound for orders because clients retry unacknowledged
orders on no-ack-timeout (`specs/2/49-webproto.md`) and the ME WAL
`RECORD_ORDER_ACCEPTED` dedups acceptance. The ME also re-sequences
on its own output WAL seq, so an inbound order gap is never an
output gap: risk / recorder / marketdata still see a contiguous ME
stream.

Fill delivery in the other direction (ME→risk) is what genuinely
needs recovery, and it has its own path: the ME runs a WAL
replication **server** (`RSX_ME_REPLICATION_BIND_ADDR`) that risk
pulls from on risk-side FAULTED. ME **cold-start** recovery replays
the ME's own local WAL (snapshot + `replay_wal_after_snapshot`);
neither depends on a remote order-replay consumer, which is why the
old `RSX_ME_REPLICATION_ADDR` pull path was removed.

Live ingestion samples `me_in` / `me_dedup_done` / `me_wal_*` /
`me_match_done` on the hot path.
