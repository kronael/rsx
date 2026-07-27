# 20260530 — crash-consistency audit (docs vs code)

**What:** read-only audit of RSX durability/consistency claims (GUARANTEES.md,
CRASH-SCENARIOS.md, docs/concepts/asymmetric-durability.md, specs/2/10,48,28)
against the actual code. Two deliverables: (1) per-service crash-at-any-instant
loss table + doc/code inconsistencies; (2) recovery-flow sketches per failure
mode. No source or docs edited — findings only.

**Method:** read the WAL writer (`rsx-cast/src/wal.rs`), the ME emit path
(`rsx-matching/src/wal_integration.rs`, `main.rs`), risk recovery
(`rsx-risk/src/main.rs`, `replay.rs`, `persist.rs`, `lease.rs`), recorder
(`rsx-recorder/src/main.rs`), and the replication server
(`rsx-cast/src/replication_server.rs`). The oracle (`codex exec`) produced no
output on this account (matches MEMORY: codex dead here) — every claim below
traces to a file:line, not to the oracle.

---

## The resolved fsync question (the headline)

**A just-emitted fill is NOT fsync'd before it is sent downstream. It can be
lost in the ≤10 ms window.** GUARANTEES.md ("WAL flush every 10ms") is correct;
`docs/concepts/asymmetric-durability.md` ("flushed to WAL and fsync'd before the
fill notification is forwarded downstream") is **wrong** and must be fixed.

Evidence — `rsx-matching/src/wal_integration.rs:458-464` (`fan_out`, the single
emit primitive for every record incl. fills):

```
let framed = writer.prepare(record)?;   // assign seq + CRC, in RAM
writer.append_framed(&framed)?;          // push to in-memory Vec buffer — NO fsync
cmp.send_framed(&framed)?;               // UDP send to risk — happens NOW
if let Some(s) = mkt { s.send_framed(&framed)?; }
```

fsync happens only in `WalWriter::flush()` (`rsx-cast/src/wal.rs:213-245`,
`self.file.sync_all()` at line 229), which the ME main loop calls via
`flush_if_due` **once per loop iteration, time-gated to ≥10 ms**
(`wal_integration.rs:469-480`; main loop call at `rsx-matching/src/main.rs:925`),
or earlier only if 1000 records accumulate (`FLUSH_RECORD_THRESHOLD`,
`wal.rs:44`, `should_flush` at `wal.rs:289`). So `send_framed` to risk precedes
`sync_all` by up to 10 ms. The send and the fsync are fully decoupled.

The unflushed fill exists in exactly two non-durable places at the instant of an
ME crash: (a) the ME's in-RAM `buf` Vec (dies with the process), and (b) an
unacked UDP datagram already delivered to risk's RAM (also volatile, and risk
itself can crash). It is **not** servable to any replication consumer either:
`ReplicationService::serve` reads via `WalReader::open_from_seq` from on-disk WAL
files (`rsx-cast/src/replication_server.rs:16,189,242`), i.e. only fsync'd
records. The CastSender retransmit ring (`rsx-cast/src/cast.rs:129-142`) lives in
the ME's own RAM and dies with the process too — it recovers in-flight UDP loss
while ME is alive, not across an ME crash.

**Net:** fill durability is **0–10 ms loss**, not 0 ms. Risk re-derives positions
from whatever fills *did* get fsync'd; a fill that was matched, sent, and
acted-on by risk but never fsync'd before the ME crash is genuinely gone from the
durable record. In practice the next live record's seq gap and the position-
reconcile invariant would surface it, but the bound is 10 ms, not 0.

---

## DELIVERABLE 1 — per-service crash-consistency table

Loss = data unrecoverable if the named service crashes at an arbitrary instant.

| Service | Crash-at-any-instant loss | Covered by (code path) | Matches GUARANTEES? |
|---|---|---|---|
| **Matching engine** | Fills: **0–10 ms** (unflushed buffer + unacked UDP). In-flight orders: lost. | snapshot.bin + `replay_wal_after_snapshot` (`wal_integration.rs:516,557`); flush 10 ms / 1000 rec (`wal.rs:213,289`) | **N** — GUARANTEES/CRASH-SCENARIOS claim **0 ms fills**; true bound is 10 ms. Their 0 ms rests on an **ME replica WAL that does not exist** (see F2). |
| **Risk** | Positions: 10 ms (PG write-behind). Fills already fsync'd at ME: 0 ms (replayed). | `run_warm_catchup` (`main.rs:1310`) → `load_from_postgres` (`replay.rs:123`) + ME replication replay; `FLUSH_INTERVAL_MS=10` (`persist.rs:376`) | **Y** (10 ms positions), *given* the upstream fill actually reached the ME WAL. |
| **Gateway** | In-flight orders only; no trading state. | stateless; tip-file replay rebuilds caches indirectly (`gateway/src/main.rs:52,85`) | **Y** (0 ms trading state). |
| **Marketdata** | Shadow book (ephemeral); rebuilt from ME WAL via replication + snapshot resend. | `run_replay_bootstrap_blocking` + seq-gap snapshot resend (`marketdata/src/main.rs:21,40-44`) | **Y** (0 ms; derived state). |
| **Mark** | Latest in-flight mark print (≤ one tick); WAL'd but send-before-fsync like ME. | `WalWriter` prepare→append→send then time-gated flush (`mark/src/main.rs:214,242,279`) | Not separately claimed; latest-wins so recoverable. Minor: same send-then-fsync ordering as ME. |
| **Recorder (ARCHIVE)** | Up to **1000 records** buffered before its own fsync; **local-disk only**. | `RecorderState::flush` every 1000 rec, `sync_all` (`recorder/src/main.rs:79,86-94`); local `RSX_RECORDER_ARCHIVE_DIR` (`config.rs:19`) | Not a primary-loss path (it's a consumer of already-durable ME WAL), but **no off-site copy** (see F4). |

### Inconsistencies to fix (each with evidence + which side is wrong)

- **F1 (fsync ordering — DOC WRONG).** `docs/concepts/asymmetric-durability.md:12-14`
  "flushed to WAL and fsync'd before the fill notification is forwarded
  downstream." Code: send precedes fsync by up to 10 ms
  (`wal_integration.rs:458-464` vs `main.rs:925` / `wal.rs:229`). Fix the concept
  doc to match GUARANTEES.md's honest "durable within 10 ms" framing. (The doc's
  own next sentence — "Loss window: bounded by the 10 ms flush interval" — already
  contradicts its first sentence; delete the "before forwarding" clause.)

- **F2 (ME replica — DOCS ASSUME UNIMPLEMENTED REDUNDANCY).** GUARANTEES.md §3.1
  "ME master crash" (line 266 "Risk replays… via replication") is fine, but the
  **0 ms fill** proofs lean on a hot replica: GUARANTEES.md §3.2 "Matching Engine
  Master + Replica Crash" ("both instances flush same WAL", line 340), §3.4 "ME
  disk total failure" ("Replica has identical WAL… replica promoted", lines
  496-504), and CRASH-SCENARIOS.md S1 proof ("ME replica receives same fills via
  SPSC ring and has flushed to its own WAL", lines 149-151) and S7. **Code has no
  ME replica process**: `rsx-matching` ships a single binary (Cargo.toml has only
  the lib + benches, no `src/bin`, no second main), and `RSX_ME_REPLICATION_BIND_ADDR`
  starts a `ReplicationService` that *serves the local WAL for catch-up*
  (`main.rs:468-491`) — it is not a second engine maintaining an independent
  fsync'd copy. So the single-ME crash loss is 0–10 ms (F1), and "ME master +
  replica" / "ME disk failure → promote replica" are **spec-only**. DOCS overclaim;
  either implement the replica or relabel these scenarios "spec-only / future
  work" and drop the 0 ms fill guarantee to "0–10 ms (single ME, no replica)".

- **F3 (S10b describes a different failover than the rest of the docs).**
  `rsx-risk` failover is **eager warm-standby**: every node applies the live
  main's ME replication stream and calls the **non-blocking** `pg_try_advisory_lock`
  only once caught up (`main.rs:1310` `run_warm_catchup`, `lease.rs:37`
  `pg_try_advisory_lock`). CRASH-SCENARIOS.md S10b (lines 826-879) documents this
  correctly. But the earlier S4 (lines 257-305) and GUARANTEES.md §3.1 Risk-crash
  procedure (lines 283-289) describe a **cold** "restart master → load PG →
  request replication → CaughtUp → go live" sequence with a blocking
  `pg_advisory_lock` flavor. Both can't be the live design. Code = warm-standby
  (S10b). The cold-restart procedures in S4 / GUARANTEES §3.1 are the **stale**
  description; reconcile them to the warm-standby + non-blocking-lock model. Note
  `acquire_advisory_lock` (blocking, `replay.rs:396`) and `AdvisoryLease::acquire`
  (blocking, `lease.rs:55`) still exist alongside the non-blocking `try_acquire` —
  worth confirming the blocking one isn't on the promotion path (it is used for
  the migration lock at `main.rs:479`, which is correct).

- **F4 (catastrophic / whole-DC RPO — DOCS IMPLY OFF-SITE; CODE IS LOCAL-ONLY).**
  GUARANTEES.md §3.3 ("restore from offloaded archive (Recorder)", line 449) and
  CRASH-SCENARIOS.md S17 ("ARCHIVE provides cold storage for infinite history")
  imply the recorder is a durable last-resort. **Code:** the recorder writes
  one file per stream per day to a **local-disk** `RSX_RECORDER_ARCHIVE_DIR`
  (`recorder/src/config.rs:19-20`, `main.rs:41-52`), fsync every 1000 records
  (`main.rs:79,86-94`). There is **no S3/scp/rsync/geo/off-site path anywhere** in
  `rsx-recorder` or `rsx-cast` (grep clean), and GUARANTEES.md §13.2 itself admits
  "This repo is single-DC." **True whole-DC RPO if every disk in the DC is lost =
  ∞ (total loss).** All WAL, snapshots, Postgres data, and the "archive" sit on
  disks in the same DC. The honest statement: catastrophic *single-disk* loss is
  survived by the local archive on a different disk; whole-DC disk loss is
  unbounded. DOCS should state RPO = total-loss for whole-DC and stop calling the
  local archive a catastrophe backstop without an off-site replica.

- **F5 (minor — recorder's own loss window undocumented).** The archive flushes
  every **1000 records** (`recorder/src/main.rs:79`), not on a 10 ms timer, so a
  recorder crash can lose up to 1000 records of *archive* coverage (the ME WAL
  still has them for 4 h — `RETENTION_NS`, `wal.rs:50`). Not a system-loss path,
  but S17's "ARCHIVE has complete history" is only true once those 1000-record
  batches are fsync'd. Add the caveat.

- **F6 (minor — retention figure mismatch).** GUARANTEES.md / CRASH-SCENARIOS.md
  repeatedly say ME WAL retention "10 min" (e.g. GUARANTEES.md §1.1 line 39,
  §3.1, S8 says "4h"). Code: `RETENTION_NS = 4 hours` (`wal.rs:50`). CRASH-
  SCENARIOS.md S8/S17 use 4 h (correct); GUARANTEES.md §1.1/§3.x use 10 min
  (stale). Reconcile GUARANTEES.md to 4 h.

---

## DELIVERABLE 2 — recovery-flow sketches

Tied to real code paths. "spec-only" = described in docs, not in code.

### (a) ME crash → replica promote  — **SPEC-ONLY (no replica exists, F2)**

```
docs claim:                          code reality:
  ME-master ──SPSC──> ME-replica       (no ME-replica process)
     │ crash             │ promote      single ME restarts:
     ▼                   ▼                snapshot.bin  ──load──>  Orderbook
  consumers re-point to replica          {wal}/<sym>/snapshot.bin (save_snapshot, 10s)
                                              │ + WAL replay from snapshot.last_seq+1
                                              ▼  replay_wal_after_snapshot (wal_integration.rs:557)
                                          ME live again; serves replication from on-disk WAL
```
Data: orderbook state. FROM `snapshot.bin` + on-disk WAL segments TO in-RAM
`Orderbook`. Phases: load snapshot → replay tail → resume. Feeds: the **local**
ME WAL only. **Loss: 0–10 ms fills (F1), all in-flight orders.** No promote path;
"replica" scenarios (S1 proof, ME+replica, ME-disk) are aspirational.

### (b) Risk crash → reload + warm-catchup  — **IMPLEMENTED**

```
  Postgres ──load_from_postgres──> RiskShard (positions + per-symbol tips)
     │  (replay.rs:123)                 │
     ▼                                  ▼
  ME replication (TCP, RSX_ME_REPLICATION_ADDR)
     │  ReplicationConsumer, resume from shard.tips[sym] (main.rs:1334)
     ▼  apply_record per fill; dedup on tip (invariant #5)
  loop until CAUGHT_UP && applied_seq >= live_seq   (run_warm_catchup, main.rs:1415)
     │
     ▼  pg_try_advisory_lock (NON-BLOCKING, lease.rs:37) ─ win? ─> FINAL DRAIN ─> LIVE
                                                         ─ lose? ─> stay warm, retry
```
Data: positions (from PG) + missed fills (from ME WAL). Phases: PG load →
warm replication catch-up → non-blocking lock → final drain → live. Feeds: PG
snapshot + ME replication stream. **Loss: 10 ms positions (last PG batch);**
re-derived from fills. Promotion is **catch-up-only**: if ME replication is
unreachable and nothing ever catches up, **no node promotes** (S10b accepted
availability tradeoff). NOTE F3: the S4/GUARANTEES "cold restart" wording is
stale vs this warm path.

### (c) ME + Risk both crash  — **IMPLEMENTED (sequential, order-independent)**

```
  ME recovers first (flow a): snapshot + local WAL replay → serves replication
                  │
                  ▼
  Risk recovers (flow b): PG load → ME replication from tips[sym]+1 → CAUGHT_UP → lock → LIVE
```
Data: ME rebuilds book from its WAL; Risk rebuilds positions from PG + ME's
now-served WAL. Phases: ME self-recover → Risk warm-catchup off the recovered ME.
Order-independent (Risk's `run_warm_catchup` retries/backs off until ME's
replication endpoint answers; `main.rs:1405-1411`). **Loss: 0–10 ms fills (F1) +
10 ms positions.** Caveat: if the *same* unflushed fills were lost at the ME (F1),
they're lost for both — the "ME WAL is source of truth" proof assumes the fill was
fsync'd, which is exactly the 0-vs-10 ms gap.

### (d) Replication consumer lag > hot retention → ARCHIVE replay  — **PARTIAL**

```
  consumer (Risk) requests from tips[sym]+1
     │  ME hot WAL retention = 4h (wal.rs:50); requested seq already pruned
     ▼  ReplicationService → RECORD_REPLICATION_NOT_AVAILABLE (NotFound)
  ┌─ code path (main.rs:1388): if tip>0, treat as "ME behind us / GC'd tail",
  │    synthesize caught-up, proceed to lock  ── i.e. assumes PG snapshot covers it
  └─ docs path (S17): operator runs rsx-archive-replay from RSX_RECORDER_ARCHIVE_DIR
        (local disk) → inserts fills into PG → restart Risk → resume
```
Data: historical fills. FROM the **local** recorder archive (one file/stream/day,
local disk, F4) TO Postgres, then normal flow (b). Phases: detect NotFound →
archive replay tool → PG reload → warm-catchup. Feeds: local archive (NOT
off-site). **Loss: 0 ms IF the archive disk survives; ∞ if whole-DC disk loss
(F4).** The in-code branch assumes the PG snapshot is already current and does not
auto-pull from the archive — the S17 `rsx-archive-replay` step is an operator
runbook, and I did not find that binary in the workspace (verify before citing it
as implemented).

---

## Caveats

Static read of code + docs; no crash was actually injected (the chaos tests in
§12 are claimed, not run here). The 0–10 ms fill bound is a worst-case window
(crash in the gap between send and the next 10 ms flush); steady-state most fills
are fsync'd within 10 ms as claimed. Oracle unavailable on this account — no
external second opinion. ME replica absence (F2) inferred from Cargo.toml + no
`src/bin` + the replication-server-only meaning of `RSX_ME_REPLICATION_BIND_ADDR`;
worth a final confirm that no replica is launched by playground/ops scripts.
