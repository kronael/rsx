# Failure-Mode Audit — what the integration tier must test

Read-only audit, 2026-08-10. Reads at commit `6e2fd52e`; HEAD at close
`5cbfc156` (+ `b9df8e41` BUGS commit). No code touched.

Grounded in `CRASH-SCENARIOS.md` (S1–S19), `GUARANTEES.md`,
`RECOVERY-RUNBOOK.md`, `specs/2/10-replication.md` §10.1–10.16,
`specs/2/48-wal.md`, `specs/2/6-consistency.md`,
`specs/2/36-testing-replication.md`, and CLAUDE.md invariants #1–#10 — then
extended at the seams. Every verdict is from reading test *bodies*.

Verdicts: **COVERED** (cited test asserts it) / **PARTIAL** (mechanism
unit-tested, seam untested) / **GAP** (searched, absent) / **DEFECT** (code
wrong; recorded in BUGS.md, not fixed).

## 1. The dangerous ones (ranked by blast radius)

Silent wrong-position outranks loud crash. Where today's code can go wrong
*without anyone noticing*:

1. **Silent replay gap after WAL GC + writer restart** (A11,
   `CAST-WAL-RESTART-ACTIVE-FIRST-SEQ-1-SILENT-GAP`, HIGH). Restarted
   producer's rotated segment claims seqs it doesn't contain (`first_seq`
   hardcoded 1, `wal.rs:149`); after GC the replication server *accepts* a
   `from_seq` it can't serve (`replication_server.rs:120-126`) and streams
   from a later seq — no `REPLICATION_NOT_AVAILABLE`, no FAULTED. Consumer
   tip jumps the hole; position ≠ sum(fills) (#4), zero alarms. Defeats the
   archive-fallback mechanism spec §10.5 relies on.
2. **ME replay error → WAL seq reuse → all new fills look like duplicates**
   (D6, `ME-REPLAY-ERR-SKIPS-SET-NEXT-SEQ-SEQ-REUSE`, HIGH).
   `rsx-matching/src/main.rs:263-269`: on replay `Err`, warns and continues
   with `next_seq = 1`; every downstream `seq <= tip` gate
   (`rsx-risk/src/shard.rs:240`) silently drops every new fill. ME trades;
   positions freeze. #4, #5.
3. **Persist write-behind silently dead or silently dropping** (E9/E10,
   `RISK-PERSIST-CIRCUIT-OPEN-SILENT-DEATH` HIGH,
   `RISK-PUSH-PERSIST-RING-FULL-DROPS-EVENT` MED-HIGH). Circuit-open kills
   the persist worker unnoticed (`persist.rs:412-421`; `main.rs:965` checks
   only `lease_held`) — shard stays Main with zero durability, standby can't
   promote. Ring-full `push_persist` drops the event (`shard.rs:166-173`)
   instead of stalling; a dropped Fill + later Tip = permanent PG fills hole
   that replay never heals. Both void the 10 ms/100 ms bounds silently; the
   documented ">100 ms PG lag → stall" rule is simply not implemented.
4. **Risk FAULTED→replication replay path has zero test callers** (C6). The
   load-bearing exactly-once mechanism for the one stream where drops are NOT
   sound (ME→risk fills, GUARANTEES §1.0): `failover.rs::handle_replay` —
   untested; `cast_ingest_test.rs`'s tests `panic!` if Faulted occurs. The two
   worst open bugs already live there
   (`RISK-SINGLE-REPLAY-ENDPOINT-PANICS-TRIO`,
   `RISK-CMP-SEQ-DOMAIN-CONFLATION`), and marketdata's replay apply is a
   recorded no-op (`MD-REPLAY-APPLY-IS-NOOP`).
5. **Live-tail replication re-streams the whole active file every 100 ms**
   (B6, `CAST-REPLICATION-LIVE-TAIL-RESTREAMS-ACTIVE-FILE`, MED-HIGH). No
   within-file seek (`wal.rs:372-390` vs spec §4; unit test
   `reader_skips_to_target_seq_within_file` asserts the *opposite* of its
   name); live-tail loop (`replication_server.rs:177-206`) re-streams the
   active file from byte 0 forever. Dedup consumers are state-correct but
   hammered; the recorder archives every duplicate
   (`rsx-recorder/src/main.rs:66-81, 263-274`, no seq guard) → the ARCHIVE
   tier (S17's backstop) silently accumulates garbage.

## 2. Catalogue

### A. WAL write / rotation / recovery (rsx-cast, frozen)

Unit home: `rsx-cast/src/wal_test.rs` (34 fns; spec 36 cites a nonexistent
`tests/wal_test.rs` — see DOC-1).

| # | Scenario | Invariant | Integration repro | Verdict |
|---|---|---|---|---|
| A1 | Torn record at EOF (§10.2) | fill durability | write, flush, truncate mid-payload, reopen; assert N-1 delivered, writer resumes | **GAP** — `reader_returns_none_at_eof` (:164) uses a clean file |
| A2 | CRC mismatch mid-file (§10.3) | conservative truncation | — | **COVERED** — `reader_crc32_invalid_truncates_stream` (:179) |
| A3 | Crash mid-rotation (§10.1, S16) | `_active.wal` recovery | kill -9 between sync and rename, restart, replay | **GAP** — only happy-rename tested (:331) |
| A4 | Empty active file (§10.7) | reader None, writer reuse | — | **GAP** — closest is missing-dir (:384) |
| A5 | Unknown record type (§10.4) | skip-and-continue | — | **WEAK** — `:199` asserts only `Ok`, not skip-then-next-record |
| A6 | Unknown header version (D20) | fail-fast through reader | — | **PARTIAL** — header primitive only (`header_test.rs:48`) |
| A7 | Buffer backpressure (D8) | producer stalls | — | **COVERED** — `writer_backpressure_stalls` (:121) |
| A8 | Flush lag > 10 ms stall (D27) | 10 ms bound | dm-delay/io-throttled container; assert stall not drop | **GAP** — code only `warn!`s (`wal.rs:229-231`) |
| A9 | GC during replay (§10.8) | reconnect-from-tip | long replay + concurrent prune | **GAP** — prune tested with no live reader (:551) |
| A10 | Rotation boundary continuity (§10.16) | no gap/dup | — | **COVERED** — `write_rotate_read_across_files` (:221), `reader_file_transition_seamless` (:429) |
| A11 | Restart into existing active file after GC | **#4 silently** | GC + restart + rotate + stale-tip consumer must get NOT_AVAILABLE | **DEFECT** — §1.1 |
| A12 | Pre-write rotation mislabels segment range | NAK WAL-fallback misses durable records | NAK a seq from the last pre-rotation flush batch | **DEFECT** — `flush()` rotates before writing (`wal.rs:217-219`); `rotate()` uses buffered `last_seq` (:194, :245-249), new `first_seq = next_seq` (:272); `pick_file_for_seq` (:640-650) scans wrong file → needless FAULTED escalation |
| A13 | Orphaned active files (§10.9) | operator path | — | **GAP** |
| A14 | Concurrent readers (§10.10) | fs safety | — | **GAP** (low value) |

### B. Replication replay / catch-up (TCP)

| # | Scenario | Invariant | Verdict |
|---|---|---|---|
| B1 | Resume from tip+1 after consumer crash; dup window ≤ persist interval (§10.11) | #4, #5 | **PARTIAL** — the one genuine chain: `rsx-cast/tests/seq_gap_replay_test.rs::seq_gap_faults_then_tcp_replay_resumes`. But `replication_client_test.rs`'s dedup/monotonic/backoff tests are **simulations on local variables, not calls into `ReplicationConsumer`** — do not cite them as coverage |
| B2 | from_seq older than retention → NOT_AVAILABLE → next endpoint (§10.5, S17) | fills 0 ms via ARCHIVE | **COVERED** — `replication_federation_test.rs::not_available_falls_through_to_next_endpoint`. But risk wires ONE endpoint and panics on refusal |
| B3 | Replay from future seq (§10.6) | safe no-op + CaughtUp | **GAP** |
| B4 | CaughtUp timing race (§10.12) | no gap/dup at handoff | **GAP** |
| B5 | TCP disconnect mid-live-tail → reconnect → resume (§10.13) | no loss | **GAP** — backoff constants asserted against themselves only |
| B6 | Live tail must stream only NEW records | archive integrity | **DEFECT** — §1.5 |
| B7 | Corrupt tip file → parse 0 → full idempotent re-replay | loud + idempotent | **PARTIAL** — garbage-content path (`replication_client.rs:284-298`) untested |
| B8 | Callback returns false on a data record: tip already advanced (`replication_client.rs:249-253`) → record lost on resume | caller contract | **GAP** |

### C. Cast/UDP hot path

| # | Scenario | Verdict |
|---|---|---|
| C1 | Single datagram loss → NAK from send ring | **PARTIAL** — `cast_test.rs::nak_retransmit_from_wal` (misnamed — in-memory ring, not WAL) |
| C2 | Gap older than ring → WAL disk fallback | **COVERED** — `tests/nak_wal_fallback_test.rs` (subject to A12's window) |
| C3 | NAK exhaustion → sticky FAULTED → TCP replay → reset → live | **COVERED** (transport level) — B1's test |
| C4 | Reorder-ring overflow → sticky Reconnect | **COVERED** — `cast_v4_test.rs::ring_overflow_faults` and siblings |
| C5 | UDP reorder / duplicate datagrams | **GAP** — `oldest_missing_run_naks_sequentially` (:177) is a stub with **zero assertions** |
| C6 | Risk FAULTED → `failover.rs::handle_replay` | **GAP** — zero test callers; §1.4. Repro: netem-drop fill datagrams until FAULTED; assert positions converge |
| C7 | Marketdata FAULTED → gap replay → resync | **GAP** + recorded `MD-REPLAY-APPLY-IS-NOOP` |
| C8 | Heartbeat-driven gap detection on idle | **COVERED** — `heartbeat_triggers_nak_on_idle_gap` |

### D. Matching engine crash/recovery

| # | Scenario | Verdict |
|---|---|---|
| D1 | Snapshot + WAL replay reproduces book (S1) | **COVERED** — `replay_after_snapshot_test.rs`, `replay_fifo_test.rs`; system-level `play_guarantees.spec.ts:253` |
| D2 | Torn `.tmp` checkpoint can't clobber last-good | **COVERED** |
| D3 | Dedup rebuilt from `RECORD_ORDER_ACCEPTED` (#2) | **PARTIAL** — TTL-expiry boundary untested; window 1 h in code vs 5 min in specs |
| D4 | Records ≤ snapshot seq skipped on replay | **COVERED** — `rsx-matching/src/wal.rs:707-713` |
| D5 | Fills-before-DONE *on disk*; exactly-one completion across a crash (#1, #2) | **PARTIAL** — nothing re-reads the WAL or crashes between fill and DONE |
| D6 | Replay `Err` at boot | **DEFECT** — §1.2 |
| D7 | Valid-CRC short-payload record during replay silently skipped | **DEFECT** — `wal.rs:719-721, 748-750` after seq advanced |
| D8 | WAL append backpressure on fill path (`.expect` panic policy) | **GAP** |
| D9 | Crash between WAL append and cast send | **GAP** |
| D11 | CONFIG_APPLIED lost in unflushed window (S18) | **GAP** |
| D12 | Online snapshot consistency | **PARTIAL** — acceptable given single-threaded loop |
| D13 | Slab accounting across a full recovery cycle (#8) | **GAP** — live-process only today |

### E. Risk crash / failover / persistence

| # | Scenario | Verdict |
|---|---|---|
| E1 | Chained cold start: PG load → replay → CaughtUp → live (S3) | **PARTIAL** — each stage separately, never one chained crash-to-live test |
| E2 | Fill dedup `(symbol_id, seq)` idempotency (#4) | **COVERED** — gate `shard.rs:240` |
| E3 | Tips monotonic across restart (#5) | **COVERED** |
| E4 | Two instances, one Main (#10, S10) | **COVERED, now runnable** — `replication_e2e_test.rs::split_brain_prevented_by_advisory_lock`. Note `persist_test.rs::advisory_lock_exclusive` is misnamed — same-session reentrancy only |
| E5 | Main loses lock mid-flight → demote path | **GAP** — `main.rs:961-975` untested |
| E6 | No-caught-up ⇒ no promotion (S10b) | **PARTIAL** — negative case is a copy-pasted boolean test |
| E7 | Promotion staleness window convergence (S10b) | **GAP** (accepted-by-design) |
| E8 | PG lag > 100 ms → stall (48-wal.md hard rule) | **DEFECT (policy absent)** |
| E9 | Persist ring full | **DEFECT** — silent event drop, §1.3 |
| E10 | Persist circuit-open worker death | **DEFECT** — §1.3 |
| E11 | `process_fill` partial apply → replay double-count (#4) | **DEFECT** — `shard.rs:262-306` |
| E12 | Crash mid-funding, resume, zero-sum (#9, S19) | **PARTIAL** — no crash-mid-settlement test |
| E13 | Margin recalc-from-scratch == incremental after recovery (§8.3) | **GAP** |
| E14 | Liquidation/insurance mid-cascade crash | **PARTIAL** |
| E15 | PG killed mid-`flush_batch` | **PARTIAL** — UPSERT idempotency covered; no mid-transaction kill |
| E16 | Orphaned frozen margin on crash | **COVERED** — `orphan_freeze_test.rs` (8 tests; best in crate) |

### F. Gateway / Marketdata / Recorder / Mark

| # | Scenario | Verdict |
|---|---|---|
| F1 | GW crash; client retry same cid → single fill | **PARTIAL** — exactly-once lives in ME dedup |
| F2 | GW-side 5-min dedup window (GUARANTEES §7) | **DOC/IMPL divergence** — not implemented in rsx-gateway/src |
| F3 | Circuit breaker rejects loudly to client | **PARTIAL** — no test asserts an error frame reaches the client |
| F4 | Pending-order timeout on risk restart | **PARTIAL** — client-facing behavior untested |
| F5 | Heartbeat timeout teardown | **PARTIAL** — timer logic only |
| F6 | Stream ordering across reconnect | **GAP** |
| F7 | MD crash → shadow rebuild from ME WAL (S13) | **COVERED** — `replay_e2e_test.rs::dxs_replay_rebuilds_shadow_book` |
| F8 | MD live gap/keep-up overflow | **GAP** |
| F9 | Recorder restart: tip resume, archive contiguity | **GAP** — `test_daily_rotation_naming` never calls rotation logic. Directly hit by B6 |
| F10 | Recorder offline past hot retention | **GAP** |
| F11 | Mark staleness | **COVERED** — `aggregator_test.rs` is thorough |
| F12 | Mark silent (0) → index fallback for liquidation | **PARTIAL** |

### G. Cross-process scenarios only the integration tier can own

Direct CRASH-SCENARIOS rows with no Rust-tier coverage; the playground
playtests (`play_guarantees.spec.ts:253` ME-crash e2e, `:288` all-stop/
all-start) are the only system-level crash coverage today.

G1 (S2, flush-lag crash): GAP. G2 (S4/S14 correlated dual-crash, 100 ms
bound): GAP. G3 (S5 recovery-order independence): GAP. G4 (S6 PG crash
mid-batch): PARTIAL via E15. G5 (S7 disk loss → promotion): GAP. G6 (S8/S15
partitions, netem heal + replay): GAP. G7 (S9 slow consumer → GW rejects
OVERLOADED loudly): GAP. G8 (S10 split-brain injection): E4, now runnable.
G9 (S11 WAL bit-rot end-to-end — loud alarm, not silent continue; interacts
with D6/D7): PARTIAL. G10 (**recovery idempotence — run every recovery
twice**, no double-funding, tips monotonic): GAP. G11 (clock skew across
containers, funding `interval_id`): GAP.

## 3. Defects recorded

`BUGS.md` `## Status — 2026-08-10 — failure-mode audit`, commit `b9df8e41`:

1. `CAST-WAL-RESTART-ACTIVE-FIRST-SEQ-1-SILENT-GAP` (HIGH)
2. `ME-REPLAY-ERR-SKIPS-SET-NEXT-SEQ-SEQ-REUSE` (HIGH)
3. `RISK-PERSIST-CIRCUIT-OPEN-SILENT-DEATH` (HIGH)
4. `RISK-PUSH-PERSIST-RING-FULL-DROPS-EVENT` (MED-HIGH)
5. `CAST-REPLICATION-LIVE-TAIL-RESTREAMS-ACTIVE-FILE` (MED-HIGH)
6. `CAST-WAL-ROTATE-SEQ-RANGE-MISLABEL` (MED)
7. `RISK-PROCESS-FILL-PARTIAL-APPLY-REPLAY-DOUBLE-COUNT` (MED)
8. `ME-REPLAY-SILENT-SKIP-UNDECODABLE-RECORD` (LOW-MED)
9. `GW-DEDUP-WINDOW-DOCUMENTED-NOT-IMPLEMENTED` (LOW-MED, docs)
10. `SPEC-DEDUP-WINDOW-5MIN-VS-1H` (LOW, docs)
11. `TESTSPEC-REPLICATION-EDGE-CASES-OVERCLAIMED` (MED, docs — spec 36's Edge
    Case section claims ~10 tests that do not exist)

The three rsx-cast entries are frozen-crate: bugfix-category but require
founder sign-off before any edit.

## 4. Build order for the testcontainers tier

1. **FAULTED fill-stream recovery e2e** (C6/C7) — the exactly-once mechanism
   with three open HIGH bugs and zero tests.
2. **Chained risk cold-start + correlated dual-crash, each run twice for
   idempotence** (E1/G2/G10).
3. **Silent-gap guard** (A11/B3): after GC + producer restart, stale tip must
   get NOT_AVAILABLE — regression test for the worst defect.
4. **Backpressure honesty** (E8/E9/G7): throttled PG + stalled consumer →
   stall-or-reject, zero silent drops.
5. **Run the now-unblocked advisory-lock suite** (E4/E5/E6) — cheapest win,
   coverage already written.
6. **Kill -9 rotation/torn-write matrix** (A1/A3/D9).
7. **Partition heal + funding crash-resume** (G6/E12).

## 5. Could not determine

- B6/A11/A12 verified by code read, **not reproduced live**.
- Pass/fail of the now-runnable `#[ignore]`d docker tests — not executed, to
  avoid colliding with the concurrent agent.
- Whether circuit-open/pending-timeout rejects reach the WS client as error
  frames (F3/F4).
- rsx-mark's own WalWriter/ReplicationService restart path.
- Line numbers cited were correct at `6e2fd52e`; rsx-risk/rsx-matching *test*
  files shifted afterward via `c3f2b51d` (sources unaffected).
