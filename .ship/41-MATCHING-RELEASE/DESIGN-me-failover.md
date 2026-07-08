# DESIGN — Matching Engine failover / warm standby

Status: design, pre-implementation. Reviewed before any code lands.

Closes the #1 phase-2 gap for "ME working well": today the matching
engine has **no lease and no standby** (confirmed — no advisory-lock,
lease, or `AdvisoryLease` reference anywhere in `rsx-matching/src/`).
Two ME processes started for the same symbol would **both** bind their
UDP order socket, both write their own WAL, and both fan out fills =
split-brain. Invariant #10 ("at most one active ME per symbol") is
currently unenforced for ME.

The fix mirrors the risk warm-standby protocol almost exactly. Risk
already does this: `rsx-risk/src/failover.rs::run_warm_catchup`
(lines 179-326) + `rsx-risk/src/lease.rs` + the re-enterable
`run_main` demote loop in `rsx-risk/src/main.rs` (lines 187-226,
341-367, 925-935). ME reuses the same shapes; the only ME-specific
pieces are the book/dedup rebuild (already exists for local crash
recovery) and one Postgres keyspace subtlety (section 1).

Spec anchor: `specs/2/21-orderbook.md` §"Replica Takeover (Same
Mechanism as Risk)" lines 258-264. CaughtUp/tip semantics:
`specs/2/10-replication.md` lines 276-288, 339.

---

## 1. Invariant + threat model

**Invariant #10:** at most one *active* ME per symbol. "Active" =
accepts orders on UDP, appends to its WAL, and fans out fills to
risk + marketdata.

**Catastrophe:** two active MEs for symbol S. Each has its own
independent `WalWriter` seq counter and its own book. They would
produce **divergent, both-"authoritative"** fill streams for the same
symbol — different trades, different `seq` for the same logical event,
double-executed orders. Risk would apply fills from whichever packets
arrive; positions become unrecoverable. Unlike a dropped order (which
the client re-sends and WAL dedup makes exactly-once,
`rsx-matching/src/main.rs:768-797`), split-brain has **no recovery** —
both WALs are internally consistent and mutually contradictory. This
is the single worst failure in the system.

**The advisory lock is the sole fence.** Catch-up does not gate
safety; it only gates *when* the lock is attempted. Postgres grants a
given advisory-lock key to exactly one session cluster-wide, so at
most one ME can hold symbol S's key and therefore be active. This is
verbatim the risk model — see the `AdvisoryLease` doc comment,
`rsx-risk/src/lease.rs:10-20`.

### What the lock keys on — the ME-specific subtlety (MUST get right)

Risk keys its lock on `shard_id` via the **single-bigint** form:
`SELECT pg_try_advisory_lock($1)` with `$1 = shard_id as i64`
(`rsx-risk/src/lease.rs:35-37`).

**ME must NOT naively reuse that keyed on `symbol_id`.** Postgres has
a single advisory-lock keyspace for the one-argument (bigint) form.
Risk shard 3 already holds `pg_advisory_lock(3)`. If ME for symbol 3
also called `pg_try_advisory_lock(3)`, ME symbol 3 would **collide
with risk shard 3** — one would starve the other and, worse, an ME
could believe it lost/won based on a *risk* shard's liveness. Risk
and ME are independent scale-out axes (per repo CLAUDE.md); their
locks must live in disjoint keyspaces.

Two options, both correct:

- **(Recommended) Two-int32 form for ME:**
  `SELECT pg_try_advisory_lock($1::int, $2::int)` with
  `$1 = ME_LOCK_CLASS` (a fixed namespace constant, e.g. `1`) and
  `$2 = symbol_id`. Postgres keeps the two-key lock space **disjoint
  from the single-key space** — so ME's `(1, 3)` never conflicts with
  risk's `3`, and ME symbols are separated from each other by
  `symbol_id`. No coordination with risk's key allocation needed.
- **Offset in the single-bigint space:**
  `key = ((ME_LOCK_CLASS as i64) << 32) | symbol_id`. Also disjoint
  provided risk keys stay `< 2^32` (they are — `shard_id: u32`). Less
  self-documenting than the two-key form.

> Verify against the deployed Postgres version that the two-key and
> one-key advisory spaces are disjoint (documented Postgres behavior,
> but confirm on the target cluster — see Open Questions).

**Where in ME startup the lock is acquired:** *after* warm catch-up,
*before* the ME binds its order socket / WAL writer / fan-out senders
— exactly as risk calls `try_acquire` only inside
`run_warm_catchup` once caught up (`rsx-risk/src/failover.rs:288`).
See section 3 for the exact ordering.

**Prerequisite:** ME's Postgres connection is currently *optional*
(only for config polling — `rsx-matching/src/main.rs:230`,
`RSX_ME_DATABASE_URL`). With failover enabled the DB connection
becomes **required** (it holds the lease). Fail-fast at startup if
the URL is missing and failover is enabled.

---

## 2. Standby architecture

A standby ME is the same binary in `NodeState::WarmCatchup`. Mirror
risk's two-state machine (`rsx-risk/src/main.rs:78-89`):
`WarmCatchup` → `Live`.

**What the standby runs (and only this):**

1. Loads its local book base state if present — `load_snapshot` +
   `load_wal_seq` (`rsx-matching/src/wal.rs:482-540`) — exactly like
   the active's cold-start recovery
   (`rsx-matching/src/main.rs:305-331`). Cold first-boot: empty book,
   replay from seq 1.
2. Opens **one** `ReplicationConsumer` (`rsx-cast`, TCP) against the
   active ME's replication server (`RSX_ME_REPLICATION_ADDR` → the
   active's `RSX_ME_REPLICATION_BIND_ADDR`, `9700+sid`). This is the
   recorder pattern: `ReplicationConsumer::new(stream_id=symbol_id,
   [addr], tip_file, tls)` then `run_once`/`run`
   (`rsx-recorder/src/main.rs:241-261`;
   `rsx-cast/src/replication_client.rs:55,96,142`). Resume from the
   consumer's persisted `tip` (`replication_client.rs:34,67`), seeded
   from the snapshot's `wal_seq.txt` on first boot so we don't
   re-request records the snapshot already contains.
3. Applies each streamed record into the in-memory book + dedup +
   order_index via the **shared per-record apply path** (section 5),
   the identical logic `replay_wal_after_snapshot` uses per record
   (`rsx-matching/src/wal.rs:544-618`): `RECORD_ORDER_ACCEPTED` →
   `process_new_order` (rebuilds book + fills deterministically),
   `RECORD_ORDER_CANCELLED` → `cancel_order` via index. Fill /
   OrderInserted / OrderDone / OrderFailed / BBO are **skipped**
   (side effects re-derived by `process_new_order`,
   `wal.rs:609-611`). Dedup is seeded per `RECORD_ORDER_ACCEPTED` as
   it streams (see note below).
4. Tracks `applied_seq` (highest seq applied) and watches for
   `RECORD_CAUGHT_UP { live_seq }` (`rsx-cast/src/records.rs:11,62`)
   to know it has drained the active's current WAL.
5. Periodically `save_snapshot` of its replicated in-memory book
   (`rsx-matching/src/wal.rs:507`) so its **own** restart is warm
   (bounded replay), not a full-history replay.

**What the standby does NOT do while standby (this is the fence's
other half — no writes escape):**

- Does **not** bind the order UDP socket (`RSX_ME_CAST_ADDR`
  `CastReceiver`, `main.rs:397`). It never accepts orders.
- Does **not** construct the fan-out `CastSender`s to risk /
  marketdata (`main.rs:401,411`). No fills/BBO leave a standby.
- Does **not** run its own `WalWriter` **as an active writer** — it
  assigns no new seqs, appends no live records. (Book rebuild is
  in-memory; the snapshot in step 5 persists book state, not a WAL.)
- Does **not** serve `RSX_ME_REPLICATION_BIND_ADDR` (the active holds
  that port; the standby is a client of it, `main.rs:424-456`).
- Does **not** poll/apply config writes or emit `CONFIG_APPLIED`
  (those are active-writer side effects); it *receives*
  `RECORD_CONFIG_APPLIED` on the stream and applies the config to its
  book so its replica stays faithful.

It is strictly read-only until promoted. This mirrors risk, which
binds only `mark_receiver` during warm catch-up and defers
`gw_receiver` / `me_senders` / `gw_sender` until after promotion
(`rsx-risk/src/main.rs:337` vs 410-451).

**Dedup note:** the one-shot recovery path deliberately does *not*
re-seed dedup inside the forward replay (`wal.rs:573-576`) —
`rebuild_dedup_window` owns it in a single ascending pass at startup.
A *continuously-running* standby instead seeds dedup incrementally
per streamed `RECORD_ORDER_ACCEPTED` (via `dedup.seed(user, hi, lo,
age)` using the record's `ts_ns`, same call `rebuild_dedup_window`
makes, `wal.rs:656`). As a belt-and-suspenders step, the standby may
run `rebuild_dedup_window` once just before flipping to active
(section 3) to guarantee the full 300 s window is present regardless
of how long it has been streaming.

---

## 3. Promotion — the exact fence ordering

Mirror `run_warm_catchup` (`rsx-risk/src/failover.rs:179-326`). The
**lock acquisition is the fence**; every write-capable resource is
constructed strictly *after* the lock is held.

```
loop {                                    // standby catch-up loop
  run_once(stream): apply records, track applied_seq,
                    stop on RECORD_CAUGHT_UP{live_seq}     // §2
  caught_up = (applied_seq >= live_seq)
  if !caught_up { continue }              // keep streaming, no lock

  acquired = pg_try_advisory_lock(ME_LOCK_CLASS, symbol_id)  // THE FENCE
  if !acquired { sleep(poll_ms); continue } // another ME is active; stay warm

  // ---- past this line we are the SOLE lock holder (invariant #10) ----
  FINAL DRAIN: run_once once more, apply everything up to the
               current WAL tip (records the old active wrote between
               the last CAUGHT_UP and our lock win). applied_seq
               now == active's last durable seq.
  (optional) rebuild_dedup_window(...)     // full 300s dedup guarantee
  break                                    // -> flip to Live
}

// ---- flip to Active (only now do write-capable resources exist) ----
wal_writer = WalWriter::new(...);
wal_writer.set_next_seq(applied_seq + 1); // invariant #5: never regress
                                          // (mirrors main.rs:336,348)
cast_sender  = CastSender(risk);          // fan-out to risk
mkt_sender   = CastSender(marketdata);    // fan-out to marketdata
cast_receiver = CastReceiver(RSX_ME_CAST_ADDR); // START accepting orders
start ReplicationService on RSX_ME_REPLICATION_BIND_ADDR; // serve downstream
spawn lease-renewal thread (renews the lock; on loss -> demote);
enter the existing live hot loop (main.rs:528-870) unchanged.
```

**Why no window exists where two MEs both write:**

1. The old active holds the lock for the whole time it is writing.
   Postgres will not grant the key to the standby's `try_acquire`
   until the old active's session **ends** (process exit / crash /
   TCP close of its PG connection releases the advisory lock
   automatically) or it explicitly `release`s it.
2. The standby constructs **no** write-capable resource
   (`WalWriter` seq assignment, `CastReceiver` for orders, fan-out
   `CastSender`s, replication server bind) until *after*
   `try_acquire` returns `true`. Before that it is pure read-only.
3. Therefore the first instant a second ME *could* write is strictly
   after it holds the exclusive lock, which is strictly after the
   first ME stopped being a session that holds it. The write-capable
   windows cannot overlap. This is exactly risk's argument
   (`rsx-risk/src/failover.rs:169-173, 285-293`).

**Order-socket handoff (deployment):** the promoted ME binds
`RSX_ME_CAST_ADDR`, which the dead active has released. Two models:
- **Co-located warm pair (single host):** only the active binds the
  UDP port; the standby binds it after promotion (port now free).
  Works today with no VIP — identical to risk's co-located model
  (risk binds `gw_receiver`/`me_receiver` only post-promotion,
  `main.rs:410,427`).
- **Cross-host:** the ME order/replication addresses are a floating
  address (VIP) claimed by the lock winner; risk's config
  (`RSX_ME_CAST_ADDR`, `RSX_ME_REPLICATION_ADDR`) is unchanged across
  failover. This is an ops concern, identical for risk (Open
  Questions).

**Downstream reconnect:** risk / recorder / marketdata consume the
old active's replication server; when it dies their
`ReplicationConsumer` sees TCP EOF and reconnects with backoff
(`rsx-cast/src/replication_client.rs:96-137`) to the same
(floating/co-located) address, now served by the new active from
`applied_seq+1`. Their live UDP `CastReceiver` FAULTs on the seq gap
and drives `handle_replay` against the new active
(`rsx-risk/src/main.rs:632-659`). No consumer change needed.

**Demote:** if the lease is later lost, ME re-enters `WarmCatchup`,
identical to risk's `run_main` returning `Ok(())` and the outer loop
re-entering catch-up (`rsx-risk/src/main.rs:192-200, 925-935`). Reuse
`spawn_lease_thread`/`stop_lease_thread`
(`rsx-risk/src/lease.rs:112,182`).

---

## 4. Loss window + correctness

**What is lost on failover:** orders the old active accepted but had
not yet **flushed** to its WAL when it crashed. The standby consumes
the active's *WAL replication stream*, which only carries flushed
records (`WalWriter` flush cadence = 10 ms,
`rsx-matching/src/wal.rs:470-477`; `specs/2/10-replication.md:200`).
Records accepted in the last `<10 ms` before the crash were never
durable on the active either — so the failover loss window is
**identical to the active's own crash-recovery loss window**: ≤10 ms
of unflushed accepted orders, plus at most one `tip_persist_interval`
(10 ms, `replication_client.rs:87`) of records the standby received
but had not persisted its tip for (those get re-streamed on
promotion's final drain — no loss, just idempotent re-apply).

Net: **loss window ≈ 10 ms of in-flight orders**, bounded by WAL
flush + tip persistence, well within WAL retention (4 h,
`main.rs:283`). Cold-history availability is bounded by retention with
ARCHIVE fallback (section 7 / `specs/2/10-replication.md:186-188`).

**Recovery of the lost in-flight orders** is the *existing* R-N1
mechanism, not new: a dropped pre-ack order has no `ORDER_ACCEPTED`
in any WAL, so the client re-sends on ack-timeout (`49-webproto`),
the promoted ME dedups on `(user_id, order_id)` via its rebuilt
dedup window, and executes exactly once. This is the same reasoning
the ME's own FAULTED handler already documents
(`rsx-matching/src/main.rs:775-793`).

**Invariant ties:**

- **#5 tips monotonic:** the promoted ME sets
  `wal_writer.set_next_seq(applied_seq + 1)` before any live append,
  so the new active's seqs continue strictly above the last
  replicated seq — never reused, never regressed. Same guard as
  local recovery (`main.rs:336, 348`, `wal.rs:159`).
- **#1 fills precede ORDER_DONE / #2 exactly-one-completion:** the
  standby rebuilds the book by re-running `process_new_order` on the
  same `ORDER_ACCEPTED` sequence the active ran — matching is
  deterministic, so the standby's book is byte-identical to the
  active's at `applied_seq`. Post-promotion it emits the standard
  Fill…OrderDone event sequence through the unchanged
  `publish_events` path (`rsx-matching/src/wal.rs:241`). An
  in-flight order that never reached a durable `ORDER_ACCEPTED`
  yields no partial completion (it simply didn't happen from the
  book's view); its re-send completes exactly once.
- **#6 no crossed book:** deterministic replay reproduces the exact
  resting book; no crossed state can appear that live matching
  wouldn't have produced.

---

## 5. Reuse map (reuse vs ME-specific)

| Piece | Reuse as-is | ME-specific / new |
|---|---|---|
| Advisory lease struct | `rsx-risk/src/lease.rs` (`AdvisoryLease`, `try_acquire` 34, `acquire` 47, `release` 57, `renew` 78, `spawn_lease_thread` 112, `stop_lease_thread` 182) — logic is domain-agnostic | **Lock key**: ME needs the two-int32 form `(ME_LOCK_CLASS, symbol_id)`, not the single-bigint `shard_id`. `AdvisoryLease` as written hard-codes the single-key SQL (`lease.rs:35-37,50,63,85-88`), so it can't be reused *verbatim*. Either add a namespaced variant or house a shared `rsx-lease` (see below). |
| Warm-catchup state machine | Shape of `run_warm_catchup` (`rsx-risk/src/failover.rs:179-326`): stream→CAUGHT_UP→try_acquire→final-drain→Live | ME's version applies records to a **book** (not a shard); it calls the ME apply fn, not `apply_record`. |
| Re-enterable main + demote loop | Shape of `rsx-risk/src/main.rs:187-226` (restart/backoff), `925-935` (lease-loss demote) | ME `main` is today a single flat `fn main` with no run-loop wrapper (`rsx-matching/src/main.rs:195`). Needs refactor into a re-enterable `run_active` + outer catch-up/restart loop. |
| Replication consumer (TCP tail) | `rsx-cast::ReplicationConsumer` **entirely** — `new`/`run`/`run_once`/`tip`/tip-file (`replication_client.rs:55,96,142,34`). FROZEN, no change. | Consumer callback is ME-specific (applies to book+dedup). |
| Replication server (serve WAL) | Already wired in ME: `ReplicationService::new`/`serve` on `RSX_ME_REPLICATION_BIND_ADDR` (`main.rs:424-456`; `rsx-cast/src/replication_server.rs:41,56`). No change. | Move its start to *post-promotion* (only the active serves). |
| Book + dedup rebuild | `load_snapshot`/`save_snapshot`/`load_wal_seq` (`wal.rs:482-540`), `rebuild_dedup_window` (`wal.rs:638`), `rebuild_order_index_from_book` (`main.rs:98`) | **Extract per-record apply** from `replay_wal_after_snapshot` (`wal.rs:568-611`) into a shared `apply_me_record(book, index, dedup, type, payload)` so both the file-replay path and the new stream path call it. This is the ME analog of risk's shared `apply_record` (`rsx-risk/src/replay.rs:248`). |
| WAL seq continuity | `WalWriter::set_next_seq` / `last_seq` (`rsx-cast/src/wal.rs:159,283`). No change. | Call it with `applied_seq+1` on promotion. |
| Health states | `LoadGauges` / `DaemonState::{WarmCatchup,Live}` (used in `rsx-risk/src/main.rs:347,365`) | ME currently hard-sets `state_idx=4` "running" (`main.rs:477`); switch to WarmCatchup while standby, Live when active, and set `ready` only when Live (mirror risk `main.rs:348,366`). |

**Shared-lease decision (needs a founder call, section 7):** cleanest
is a tiny new `rsx-lease` crate (move `AdvisoryLease` there,
parameterize the key: single-bigint for risk, two-int for ME) that
both risk and ME depend on. Fastest-to-ship is duplicating ~100 lines
into `rsx-matching/src/lease.rs` with the two-int SQL. A direct
`rsx-matching → rsx-risk` dependency is rejected (couples the two
independent scale-out axes). **rsx-cast is NOT the home** — it takes
no runtime dep and `AdvisoryLease` uses `tokio_postgres`
(`rsx-cast/CLAUDE.md`, frozen + no-runtime rule).

---

## 6. Phased implementation plan

Each increment is independently shippable, testable, and leaves the
tree green (`make test` + `make lint`). Order is by safety-criticality:
the fence first, standby second, auto-promotion third.

### Increment 1 — single-writer fence at ME startup (safety core)

Smallest change that eliminates split-brain. **No standby, no
catch-up yet** — just refuse to become active without the lock.

- ME requires `RSX_ME_DATABASE_URL`; connect at startup (reuse the
  existing `rt` + `tokio_postgres::connect`, `main.rs:231-247`).
- Before binding the order `CastReceiver` / fan-out `CastSender`s /
  replication server (`main.rs:397-456`), acquire the lock keyed on
  `(ME_LOCK_CLASS, symbol_id)` — **blocking** `acquire` is fine for
  increment 1 (a second ME simply parks). Spawn the renewal thread
  (`spawn_lease_thread`); on lease loss, exit (crash-restart) — no
  demote path yet.
- Everything downstream of the bind is unchanged.

Result: starting a second ME for symbol S blocks (or exits) instead
of writing. Invariant #10 enforced.

**Test (split-brain):** integration test starts ME #1 (acquires
lock, becomes active), then ME #2 for the same `symbol_id` against
the same Postgres. Assert ME #2 never binds `RSX_ME_CAST_ADDR` /
never appends to its WAL / never emits a fill while #1 holds the
lock. Assert an ME with a *different* `symbol_id` proceeds (no false
contention), **and** an ME with `symbol_id == N` does **not** contend
with a *risk* shard `shard_id == N` (the keyspace-namespacing
regression test — this is the subtle one).

### Increment 2 — standby replication-consumer + warm catch-up

Adds the read-only follower and the catch-up-then-lock protocol, but
promotion is still effectively "whoever wins the free lock first"
(same as a fresh start). No automatic *takeover* of a live symbol
yet — that's increment 3's kill/promote test.

- Refactor `fn main` into `run_active(...)` (the current hot loop) +
  an outer loop that runs `WarmCatchup` then calls `run_active`,
  re-enterable on demote (mirror `rsx-risk/src/main.rs:187-226`).
- Extract `apply_me_record(...)` from `replay_wal_after_snapshot`
  (section 5) and add the ME `run_warm_catchup` (stream via
  `ReplicationConsumer`, apply to book+dedup+index, detect
  CAUGHT_UP, `try_acquire`, final drain) modeled on
  `rsx-risk/src/failover.rs:179-326`.
- Switch the lock to **non-blocking** `try_acquire` inside the
  catch-up loop; keep blocking `acquire` out of the hot path.
- Standby periodic `save_snapshot` of the replicated book so its own
  restart is warm.
- Health: WarmCatchup while following, Live + ready on promotion.

**Test (warm follow):** ME #1 active + a driver submitting orders;
ME #2 as standby consuming #1's replication stream. Assert #2's book
`sequence` and BBO track #1's within the tip lag, and #2 emits
nothing. Assert #2 rejects nothing / accepts nothing (read-only).

### Increment 3 — automatic promotion on active failure

Turns the standby into a hot-takeover replica.

- Add the demote path: lease-loss → tear down active resources →
  re-enter `WarmCatchup` (mirror `rsx-risk/src/main.rs:925-935`).
- On the active's death, the standby's next `try_acquire` succeeds
  (old session's lock auto-released on PG disconnect); final-drain to
  the last replicated tip; `set_next_seq(applied_seq+1)`; bind order
  socket + fan-out + replication server; enter `run_active`.
- Verify downstream (risk/recorder/marketdata) reconnect + FAULTED
  replay against the promoted ME with no seq gap and no double-fill.

**Test (kill-active → promote-standby):** ME #1 active + ME #2
standby, driver submitting a steady order flow. `SIGKILL` ME #1
(hard crash, not SIGTERM, so no clean release — proves the
PG-session-death release path). Assert: (a) ME #2 promotes within
`lease_poll_interval + backoff`; (b) the promoted ME's first live seq
== old `applied_seq + 1` (monotonic, invariant #5); (c) no order is
double-executed (dedup); (d) a risk consumer's positions are
consistent with the union of pre- and post-failover fills (invariant
#4). Also: a symbol with a resting order older than the driver run
must still be on the promoted book (rebuild completeness).

---

## 7. Open questions / risks for founder review

1. **Postgres keyspace namespacing (blocking for increment 1).**
   The design keys ME on the two-int32 advisory form to stay disjoint
   from risk's single-bigint `shard_id` locks. Confirm on the target
   Postgres that the two-key and one-key advisory spaces are disjoint
   (documented Postgres behavior, but verify on the deployed version),
   and ratify `ME_LOCK_CLASS`'s value + where it's defined.

2. **Shared lease housing.** New `rsx-lease` crate vs duplicate
   ~100 lines into `rsx-matching/src/lease.rs`? (Not rsx-cast —
   frozen + no runtime dep; not a matching→risk dep — couples the
   axes.) Recommendation: `rsx-lease` for one source of truth, but
   duplication is acceptable to ship increment 1 fast.

3. **Does the standby need the snapshot, or is the stream enough?**
   The ME replication stream is sufficient to rebuild the *full* book
   **only if it can be replayed from seq 1** (empty-book origin).
   Hot WAL retention is 4 h (`main.rs:283`), so a cold standby
   replaying from seq 1 hits `RECORD_REPLICATION_NOT_AVAILABLE` for
   pre-retention seqs and must **federate to ARCHIVE**
   (`ReplicationConsumer` multi-endpoint, `specs/2/10-replication.md:186-188,
   307-318`) to get the tail that holds long-resting orders. Confirm
   the standby's endpoint list includes the ME archive, and that a
   resting order older than 4 h is present in archive. Mitigation in
   the design: the standby snapshots its own replicated book, so only
   the *first* cold boot needs full-history replay. **Decision
   needed:** is archive-federation in scope for the standby's
   endpoint list now, or do we assume snapshot-shipping / co-located
   shared filesystem for the base?

4. **Tip readability across instances.** The active persists
   `wal_seq.txt` (`wal.rs:521-526`) locally; the standby persists its
   own `ReplicationConsumer` tip file. On the same host these are
   distinct paths — confirm the standby's `RSX_ME_WAL_DIR` /
   tip-file path does not collide with the active's snapshot dir
   (`{wal_dir}/{symbol_id}/`). Cross-host: each has its own disk;
   fine.

5. **Post-promotion replay of pre-`applied_seq` history.** The
   promoted ME's WAL starts at `applied_seq+1`; it cannot serve
   replay for seqs `≤ applied_seq` it never wrote locally (unless we
   also mirror replicated records into its local WAL — a heavier
   variant). A downstream needing older seqs falls to ARCHIVE. This
   is *identical* to a promoted risk shard and is believed
   acceptable, but confirm no consumer strictly requires the new
   active to serve pre-promotion history. If unacceptable, add
   "standby mirrors the WAL to disk" as an increment-2.5 (the
   recorder already shows raw-record-to-file, `rsx-recorder/src/main.rs:64-79`).

6. **Address handoff (co-located vs VIP).** Same open question risk
   has: single-host co-located pair works with only-active-binds; the
   cross-host / floating-IP story is an ops concern not yet specified.
   Which deployment model do we commit to for the demo?

7. **Lease cadence.** Reuse risk's `lease_poll_interval_ms` /
   `lease_renew_interval_ms` (`rsx-risk/src/main.rs:270-272`)? A
   faster poll shortens takeover time but adds Postgres load. Pick ME
   defaults (risk uses these from `replication_config`); the renewal
   thread already handles 3-consecutive-error → demote
   (`rsx-risk/src/lease.rs:167-175`).

8. **Determinism guarantee under config changes.** Book rebuild
   re-runs `process_new_order`; if `CONFIG_APPLIED` (tick/lot) landed
   mid-stream, the standby must apply the same config at the same seq
   point the active did so matching stays byte-identical. The stream
   carries `RECORD_CONFIG_APPLIED` in order, so applying it inline
   during catch-up (section 2, step "receives CONFIG_APPLIED")
   preserves this — confirm the apply happens *before* subsequent
   `ORDER_ACCEPTED` records are matched, matching the active's
   ordering.
