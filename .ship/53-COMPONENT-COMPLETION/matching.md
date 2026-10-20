# 53.2 — rsx-matching completion plan

Read-only audit of the crate at `21049460` (split finished: `main.rs` →
`app.rs` → `cast_loop.rs`, handlers `order.rs` / `cancel.rs`, lib modules
`checkpoint`, `config`, `dedup`, `wal`, `wire`). Every status below was
checked against the code, not BUGS.md or the crate docs. Line numbers are
per file at that commit.

Legend: **T** implemented + tested · **U** implemented, untested ·
**M** missing (or contradicted by the code) · **S** spec stale or
self-contradicting (doc fix, no code) · **N/A** not an ME concern.

"Tested" counts a test anywhere in the workspace that pins the behaviour;
when the only test is in `rsx-book`, the row says so. Nothing in
`rsx-matching` tests the ME tile itself: `order.rs`, `cancel.rs`,
`cast_loop.rs` and the recovery sequence in `app.rs` are binary-private and
have no unit test. Six of the twelve `rsx-matching/tests/*.rs` files
(`lifecycle`, `invariant`, `event`, `smooshed`, `position`,
`order_processing`) drive `rsx_book` directly and test no ME code.

## 1. Spec vs code

### 17-matching.md (the ME service spec)

| # | Requirement | St | Evidence |
|---|---|---|---|
| 1 | One pinned thread, `RSX_ME_CORE_ID`, no locks | U | `app.rs:81-89`, `cast_loop.rs:62-254`; structural |
| 2 | Inputs `RECORD_ORDER_REQUEST` + `RECORD_CANCEL_REQUEST` | U | `cast_loop.rs:127-158`; no test drives the dispatch |
| 3 | Dedup → `OrderFailed{Duplicate}` to WAL + both casts | U | `order.rs:49-100`; `dedup_test.rs` tests the tracker only |
| 4 | `OrderAccepted` to WAL before matching, WAL failure panics | U | `order.rs:101-137` |
| 5 | `process_new_order` on the book | T | rsx-book `matching_test.rs`, `rsx-matching/tests/lifecycle_test.rs` |
| 6 | Publish: one frame per record to WAL + risk + md; **cast sends warn only** | M | `wal.rs:333-339` `FanoutSink::emit` propagates `send_framed` errors with `?`; `publish_events` then hits `.expect` (`order.rs:165-177`, `cancel.rs:118-121`) — a UDP `EAGAIN`/`ENOBUFS` on the risk socket kills the ME and skips the md send. Contradicts 17 step 4, `notes/authoritative-wal.md:34-35` and the crate's keeper section |
| 7 | Every record on the single WAL seq to both streams | U | `wal.rs:282-340`; `src/wal_test.rs` uses a capture sink, not `FanoutSink` |
| 8 | O(1) cancel index, drift check before unlink | U | `cancel.rs:21-76` (lookup :33, checked cancel :63); no cancel test in the crate |
| 9 | TIF / post-only / reduce-only semantics | T | rsx-book `fok_liquidity_test`, `post_only_test`, `matching_test`; `tests/position_test.rs` |
| 10 | BBO after best change, cancel path included | U | book path T (rsx-book `bbo_emitted_*`); cancel path `cancel.rs:102-114` untested |
| 11 | ORDER_DONE exactly once | T | `tests/invariant_test.rs`, `tests/order_processing_test.rs::fok_fail_exactly_one_completion` |
| 12 | "No external I/O in the core loop" | M | `cast_loop.rs:209-223` → `config.rs:215,226` `rt.block_on` Postgres query + upsert on the pinned thread (ME-CONFIG-POLL-BLOCKS-PINNED-LOOP) |
| 13 | Env base config + optional 10-min Postgres poll | U | `config.rs:153-166, 204-243`; SQL tested under `make integration` (`config_poll_test.rs`), the apply path never |
| — | "Marketdata gets no OrderDone" vs step 4 "every record to both" | S | 17:26-27 contradicts 17:54-63; code sends all |
| — | `wal_integration.rs`, `cancel_order`, BBO "from the head order" | S | now `wal.rs`, `cancel_order_checked` (`cancel.rs:63`), `current_bbo()` (`cancel.rs:107`) |

### 21-orderbook.md (ME-owned sections: §1, §2, §2.7–2.9, §6, §6.5, §7)

| # | Requirement | St | Evidence |
|---|---|---|---|
| 1 | Fixed-point i64 price/qty | T | rsx-types; all book tests |
| 2 | Tick/lot alignment at entry | T | `tests/order_processing_test.rs` (invalid tick/lot/zero/negative) |
| 3 | §2.9 poll every 10 min, apply by `effective_at_ms` + version, emit CONFIG_APPLIED | U | `config.rs:204-243`; `config_poll_test.rs` covers the SELECT; no test of `apply_scheduled_configs` |
| 4 | §2.9 "Matching validates tick/lot sizes **and status**" | M | `symbol_config_schedule.status` exists (`rsx-risk/migrations/005_symbol_config.sql:27`); `poll_scheduled_configs` never selects it (`config.rs:33-39`) and `write_applied_config` always writes `'active'` (`config.rs:78`) |
| 5 | §2.7 incremental recentering, `migrate_batch(100)` on idle, lazy `resolve_level` | M | ME calls `recenter_now` (`order.rs:229-237`), which migrates eagerly in one shot (`rsx-book/src/migration.rs:234-237`). Deliberate (BUGS RECENTER-EAGER-TAIL-SPIKE). The idle `migrate_batch` branch (`cast_loop.rs:193-195`) is dead: the ME is never `Migrating` |
| 6 | §2.8 WAL per symbol, every order/cancel/fill | T | `tests/wal_test.rs::wal_records_written_for_all_event_types` |
| 7 | §2.8 embedded `ReplicationService` | U | `app.rs:268-301`; service tested in rsx-cast only |
| 8 | §2.8 online snapshot: capture on loop, refuse while migrating, Avro worker, tmp + fsync + rename + dir fsync | T | `cast_loop.rs:229-244`, `checkpoint.rs:55-186`; `replay_after_snapshot_test.rs::{checkpoint_is_self_describing_avro_v1, incomplete_next_checkpoint_does_not_replace_last_good_checkpoint}`, rsx-book `snapshot_during_migration_fails` |
| 9 | Shutdown: final capture, flush worker, join | T | `cast_loop.rs:98-118`; `checkpoint_worker_persists_final_capture_before_join` (worker only) |
| 10 | Recovery: checkpoint (legacy RXSN v2/v3 readable) + replay from `wal_seq+1` | T | `replay_restores_orders_appended_after_snapshot`, `legacy_checkpoint_with_rxsn_v3_still_loads`, `replay_with_no_snapshot_replays_from_seq_1`; the `app.rs:164-214` sequencing (next-seq bump, error arms) untested |
| 11 | §2.8 replica takeover: ReplicationConsumer tip + Postgres advisory lock | M | no replica mode, no lease in the crate |
| 12 | §2.8 tail efficiency (≤1–3 µs per order during migration) | M | eager recenter; spike is O(slot count) on one order |
| 13 | §6 fixed 65 536-event buffer, panic before overflow | T | `tests/event_test.rs`, rsx-book |
| 14 | §6.5 per-user net_qty, 60 s deferred reclaim, no reclaim during replay | T | rsx-book `src/user_test.rs`; ME calls `maintain_users` only on idle live spins (`cast_loop.rs:196-201`), replay never does |
| 15 | §7 zero heap on the hot path | U | no allocation-counting test anywhere |

### 6-consistency.md

| # | Requirement | St | Evidence |
|---|---|---|---|
| 1 | Every record to both direct consumers on one seq | U | `wal.rs:333-339` |
| 2 | Recorder as replication consumer | U | ME side is the `ReplicationService` sidecar (`app.rs:268-301`) |
| 3 | Mirrored stream to a hot-spare ME | M | none |
| 4 | Total order within a symbol (monotonic seq) | T | `tests/replay_fifo_test.rs`, `invariant_test.rs::event_seq_monotonic_within_symbol` |
| 5 | Fills precede ORDER_DONE | T | `invariant_test.rs`, `event_test.rs` |
| 6 | Crash → snapshot + WAL replay | T | `replay_after_snapshot_test.rs` |
| 7 | Inv 5: ORDER_DONE is the commit boundary | U | event order is the book's; no test of a multi-fill WAL sequence ending in DONE |
| 8 | Inv 7: book persisted via snapshot + WAL | T | as 6 |
| — | §3 / inv 4 "ring full = ME stalls", Verification "matching stalls on md ring" | S | the ME has no rings; fan-out is UDP |
| — | §5 "SIGTERM = crash, no flush, no drain" | S | code and 21 §2.8 flush the WAL and write a final checkpoint (`cast_loop.rs:98-118`) |
| — | §1 diagram shows ME → Gateway; cross-refs cite `wal_integration.rs` | S | gateway is fed by risk (6:59-62) |

### 41-testing-matching.md (M1–M31)

| St | Rows |
|---|---|
| T (19) | M3 tick/lot, M4 reduce-only, M5 dedup (window differs, below), M8, M9, M10 (rsx-book `match_fill_price_is_maker_price`), M11 (book path), M12, M13, M16, M17 (rsx-book `user_test`), M18, M19, M20 (`smooshed_test`), M22, M24, M26, M29 (rsx-book), M30 |
| U (9) | M1, M6 fan-out, M7 FAULTED skip (`cast_loop.rs:159-188`), M14 replication service, M15 config poll (the 5 "tests" below are tautologies), M23 zero heap, M25 per-consumer links, M27 commit boundary, M28 fills final |
| M (1) | M21 incremental CoW recentering (eager by design) |
| S (1) | M2 "GTC only in v1" — IOC/FOK/post-only ship |
| N/A (1) | M31 gateway ingress cap |

E2E paragraph gaps: "zero heap during matching", "seq continuity" across
restart, "rotation under load", "fan-out under load (ring drain, per-ring
backpressure)" have no test; the ring items describe a design the ME does not
have (S). "Replica takeover" and "hot spare ME" (Integration Points) are M.

Tautological tests counted as no coverage: `tests/config_test.rs`
`config_version_increments`, `config_effective_at_respected`,
`config_updates_tick_lot_sizes`, `config_poll_interval_10min`;
`tests/event_test.rs::event_buffer_max_capacity` asserts a constant.
`event_test.rs::{fills_precede_order_done_always, exactly_one_completion_per_order}`
are weaker copies of the same-named tests in `invariant_test.rs`.

### 47-validation-edge-cases.md (ME rows)

| # | Requirement | St | Evidence |
|---|---|---|---|
| 1 | §3.1/3.2 tick/lot reject | T | `order_processing_test.rs` |
| 2 | Config change: resting orders grandfathered, new orders on new tick | T | rsx-book `config_test.rs::config_tick_size_change_validates_new_orders`; ME apply path U |
| 3 | §3.3 dedup: exact dup, dup after cancel, accept after window | T | `dedup_test.rs`, `order_processing_test.rs::{new_order_duplicate_id_rejected, new_order_after_dedup_window_accepted}` |
| 4 | §1.4 / §1.8 unknown `tif` / `side` → `ORDER_FAILED` | M | `wire.rs:14-18` and the replay copy `wal.rs:395-400` map unknown tif to GTC and any side ≠ 0 to Sell |
| 5 | §1.5 reduce-only matrix, §1.6 post-only matrix | T | `position_test.rs`, rsx-book `matching_test`, `post_only_test` |
| 6 | §3.4 valid cancel → CANCELLED | U | `cancel.rs` |
| 7 | §3.4 not found / already cancelled / wrong user → REJECT | M | `cancel.rs:33-43` logs a warn and emits nothing; client sees no reply from the ME |
| 8 | §3.5 IOC table | T | rsx-book `matching_test` |
| 9 | §3.6 self-trade allowed | U | no test |
| 10 | §5 ME order: dedup → validate → reduce-only → post-only → FOK | U | `order.rs` + book; no ME-level test of dedup-before-validate |
| 11 | §7 rejection-rate-by-reason metrics | M | ME `/metrics` exports `orders_processed`, `publishes`, `dedup_map` only (`app.rs:326-345`); not even `gauges.drops` (bumped at `cast_loop.rs:178`) |
| — | §3.3 "5 min", "dedup map cleared on restart" | S | `dedup.rs:6` 1 h; window rebuilt from WAL (`app.rs:211-214`, tested `dedup_window_rebuilt_from_wal_for_pre_snapshot_order`) |
| — | §3.5 FOK table "ORDER_DONE(CANCELLED)" vs §5 "OrderFailed(FAIL_FOK)" | S | code: `OrderFailed(FillOrKill)` |
| — | Layers: ME validates "symbol existence" | S | `OrderMessage` has no `symbol_id` (`rsx-messages/src/lib.rs:400-420`); one process per symbol |

### 18-messages.md (ME rows)

| # | Requirement | St | Evidence |
|---|---|---|---|
| 1 | Cancel: lookup by `(user, oid)`, remove, OrderDone(CANCELLED) | U | `cancel.rs` |
| 2 | Cancel miss → `ORDER_FAILED(ORDER_NOT_FOUND / ALREADY_CANCELLED)` | M | as 47 #7; `OrderFailureReason` has no such value (`rsx-book/src/event.rs:17-22`) |
| 3 | `OrderCancelledRecord.reason` uses `CANCEL_REASON_*` (post-only = 4) | M | `wal.rs:174` writes the book's `OrderCancelReason as u8` (PostOnly = 3, which the wire table calls SYSTEM). Latent: gateway and md ignore the field (`rsx-gateway/src/route.rs:155-183`), `rsx-cli` prints it |
| 4 | Completion exactly once, dedup key `(user_id, hi, lo)` | T | as above |
| — | `risk_in_ns` "always 0, OrderMessage exactly 64 bytes" | S | `OrderMessage` is 128 bytes and carries `risk_in_ns` (`rsx-messages/src/lib.rs:418-423`); ME stamps it (`wal.rs:111`), tested `src/wal_test.rs::matching_records_retain_risk_ingress` |

### Other specs that name ME behaviour

Audited by a read-only sub-agent against the code; the load-bearing rows
(19:67-69, 48:83-98, 48:136-138, 10:502, the report p99 gap) were re-checked
by hand.

| Spec | Requirement | St | Evidence |
|---|---|---|---|
| 48-wal | Flush every 10 ms | T | `wal.rs:344-354`, `cast_loop.rs:225`; `tests/wal_test.rs::flush_timer_fires_at_10ms` |
| 48-wal | Also flush at 1000 records (48:87) | M | `WalWriter::should_flush` (`rsx-cast/src/wal.rs:279`) is never called by the ME |
| 48-wal | Rotate 64 MB, retain 4 h | U | `app.rs:150`; rotation + `prune_old_segments` live in rsx-cast and run only on rotate, so a quiet symbol keeps segments past 4 h (rsx-cast frozen) |
| 48-wal | Buffer full → producer **stalls** (48:83, :136) | M | `append_framed` returns `WouldBlock` past 2×64 MB (`rsx-cast/src/wal.rs:184-190`); the ME `.expect`s it (`order.rs:123`) and exits. Contradicted |
| 48-wal | Flush lag > 10 ms → stall | M | fsync is inline so a slow disk does stall, but a flush *error* only warns (`cast_loop.rs:225-227`) and buffering continues |
| 48-wal | Replica lag > 100 ms → stall (48:138) | M | no consumer-lag signal; needs rsx-cast feedback |
| 48-wal | Offload worker (48:100) | M | not in the ME; the recorder archives instead |
| 48-wal / 10 | "SIGTERM = crash" | S | `cast_loop.rs:98-118` flushes + final checkpoint (matches 21 §2.8) |
| 10-replication | ORDER_ACCEPTED before processing, dedup rebuilt from WAL | T | `order.rs:101-137`, `wal.rs:451-479`; `dedup_window_rebuilt_from_wal_for_pre_snapshot_order` |
| 10-replication | Embedded `ReplicationService` | U | `app.rs:268-301` (TLS required) |
| 10-replication | ME skips unknown record types **and logs a warning** (10:502) | M | live loop (`cast_loop.rs:127-157`) and replay (`wal.rs:423`) skip silently |
| 10-replication | `CancelReason` table (10:127-139) | M | same as 18 #3 |
| 19-metadata | Poll 10 min, apply `effective_at <= now && version > current`, emit CONFIG_APPLIED | U | `config.rs:25-64, 168-243`; only the SQL is tested (`config_poll_test.rs`, `#[ignore]`) |
| 19-metadata | Cold start reloads tick/lot/decimals from `symbol_config_applied` (19:65-69) | M | `app.rs:115-120` keeps only `cfg.config_version`; the book is built from env (`app.rs:142`) or the checkpoint (`app.rs:166`), and `app.rs:304-313` then announces a version the book may not run |
| 19-metadata | `status` (active/paused/halted), `min_notional`, `max_order_qty` (19:39-49) | M | never selected; no `OrderFailureReason` for a halt |
| 57-config-server | Replay rebuilds config from the WAL | M | `ConfigAppliedRecord` has no tick/lot; replay skips it (`wal.rs:423`). 57 is `status: not implemented` |
| 59-latency | `me_in_ns`, `match_done_ns` on fills and the taker's OrderDone | U | `order.rs:39,151`, `wal.rs:110-113, 193-215`; `src/wal_test.rs` only checks pass-through of the arguments |
| 45-tiles | ME = one pinned loop, no rings | U | `cast_loop.rs`; 45's thread inventory omits the checkpoint thread, the replication runtime and the current-thread runtime used by the poll |
| 22-perf | ME latency benches with p50 **and p99** | M | none of the six `rsx-matching/benches` reports p99; `reports/20260716_book-matching-publication.md` (the crate README's evidence link) has no p99; `bench-baseline.json` has no ME entries, so `make perf-gate` never fails them. The book has p50/p99 (`reports/20260727_book-deep-match-tail.md`) |
| 5 / 9 / 20 / 30 / 44 / 45 / 57 | stale `rsx-matching/src/main.rs:NNN` citations, "book starts empty after restart" (44:123), "checkpointing deferred" (44:426), missing env vars `RSX_RISK_ME_RECV_ADDR` / `RSX_RISK_ME_SEND_ADDR` (9:118-134), fictional `RSX_ME_BTC_ADDR` (20:417), "OrderCancelled **or** OrderDone" (5:37, code emits both) | S | doc step |

### Counts

| Spec | T | U | M | S (doc) |
|---|---|---|---|---|
| 17-matching | 3 | 8 | 2 | 2 |
| 21-orderbook | 8 | 3 | 4 | — |
| 6-consistency | 4 | 3 | 1 | 3 |
| 41-testing-matching (M1–M31, +1 N/A) | 19 | 9 | 1 | 1 |
| 47-validation (ME rows) | 5 | 3 | 3 | 3 |
| 18-messages (ME rows) | 1 | 1 | 2 | 1 |
| 48 / 10 / 19 / 57 / 59 / 45 / 22 | 2 | 5 | 11 | 2 |
| **Total rows** | **42** | **32** | **24** | **12** |

M rows overlap across specs (cancel reject in 47 and 18, cancel-reason codes
in 18 and 10, eager recentering in 21 twice and 41, replica in 21/6/41).
Distinct missing items: 18 — 11 fix-now (the spec decides), 7 proposals.

## 2. Open matching bugs (BUGS.md)

| Entry | Verdict | Why |
|---|---|---|
| ME-PG-DRIVER-INLINE-SPAWN | **fix now** | real, `app.rs:105`; repo rule decides; `rsx-risk/src/db.rs` has the shape |
| ME-CONFIG-POLL-BLOCKS-PINNED-LOOP | **fix now** (reclassify from proposal) | real, `cast_loop.rs:209-223`, `config.rs:215,226`. 17 §Determinism already decides the behaviour ("no external I/O in the core loop") and `rsx-risk` already ships the shape (sidecar thread + 64-slot config ring drained by the loop, `rsx-risk/src/app.rs:286-289`). The "correct the docs instead" alternative contradicts the spec |
| ME-REPLAY-ERR-SKIPS-SET-NEXT-SEQ-SEQ-REUSE | **fix now** | still real, now `app.rs:196-202` (BUGS cites `main.rs:263`). 6 §5 makes restart the one recovery path; abort boot |
| ME-REPLAY-SILENT-SKIP-UNDECODABLE-RECORD | **fix now** | still real, now `wal.rs:384-386, 413-415` (and `rebuild_dedup_window` `wal.rs:465-467`); BUGS cites `:719` |
| ME-SNAPSHOT-SYNCHRONOUS-HOT-LOOP | **stale — prune** | fixed by `36f014fc`: loop only captures (`cast_loop.rs:229-244`); Avro encode + fsync + rename run on `CheckpointWriter`'s thread (`checkpoint.rs:55-186`). Residue: no capture-pause p50/p99 bench → step 10 |
| ME-MAKER-ORPHANED-QUOTES-WEDGE-BOOK | **stale as an ME bug — re-home** | the "locked book" was `/api/book`, which reads the playground's marketdata cache (`rsx-playground/server.py:8505-8510`, DEMO-MD-DISPLAY-STARVE); the ME cannot rest a crossing order (`process_new_order` matches before `insert_resting`). Cancel misses are the maker / GW-PENDING-TIMEOUT-STRANDS-RESTING-ORDERS. Move under maker/gateway |
| ME-ENGINE-LATENCY-NOT-REPORTED | **ME part done — rewrite** | ME stamps `me_in_ns`/`match_done_ns` on fills and the taker's OrderDone (`order.rs:39,151`, `wal.rs:110-113, 193-215`). Remaining work is gateway/terminal |
| ME-SNAPSHOT-NO-INDEX-DEDUP-REBUILD, ME-WAL-WRITER-DUPLICATION, ME-ACCEPT-BENCH-NO-BBO, MATCHING-BENCH-ORDERTYPE-FIXTURE, ME-FAULTED-NO-REPLAY-ADDR, ME-REDUCEONLY-IOC-FILLEDQTY, the "Resolved LOW" line (ME-CANCEL-REIMPL, ME-REASON-DUPLICATE-NAMESPACE, ME-NEXT-SEQ-REGRESSION) | **prune** | each says FIXED/RESOLVED in its own text; history is in git |
| MATCHING-BENCH-SHARED-HOST-VARIANCE | keep, out of this pass | needs a dedicated box, not code |
| ME-GW-DIRECT-SPEC-GAPS | keep, **proposal** | spec rewrite owed before code |
| ME-HOLDS-USER-STATE | keep, **proposal** (deferred design) | |
| SPEC-DEDUP-WINDOW-5MIN-VS-1H | **decision owed** | every spec says 5 min; code says 1 h on purpose (`421971e5`, `notes/dedup-persistence.md:15`). Recommend 1 h in the specs (larger window is the safer side and the rebuild already covers it); the doc step waits on the call |

New entries found in this audit (recorded by step 0):

| New entry | Verdict | Evidence |
|---|---|---|
| ME-FANOUT-CAST-SEND-PANICS (HIGH) | **fix now** | 1 #6 above |
| ME-CANCEL-REASON-WIRE-CODES (LOW) | **fix now** | 18 #3; map User→0, ReduceOnly→1, PostOnly→4 (`rsx-messages/src/lib.rs:37-42`); the book never emits the IOC variant (`rsx-book/src/matching.rs:105` is the only `OrderCancelled` site) |
| ME-WIRE-ENUM-UNVALIDATED (LOW) | **fix now** | 47 #4 |
| ME-METRICS-MISSING-DROPS-AND-REJECTS (LOW) | **fix now** | 47 #11 |
| ME-CANCEL-MISS-SILENT (MED) | **proposal** | 47 #7 / 18 #2: needs a wire reason (`NotFound`), and a decision on what risk and the gateway do with an `ORDER_FAILED` for an order they may already have closed |
| ME-REPLAY-IGNORES-CONFIG-CHANGES (MED) | **proposal** | replay re-executes `OrderAccepted` under the snapshot's tick/lot (`wal.rs:382-411` ignores `RECORD_CONFIG_APPLIED`), and `ConfigAppliedRecord` carries no tick/lot (`rsx-messages/src/lib.rs:251-259`, 16 spare bytes). Orders accepted after a post-snapshot config change can be rejected on replay → book diverges. Needs a wire/replay design |
| ME-CRASH-BEFORE-FLUSH-SEQ-REUSE (MED) | **proposal** | records are cast before the 10 ms WAL flush (`order.rs:131-136`, `wal.rs:335-338`); a crash in that window loses them from disk, and boot sets `next_seq` from disk, so the next records reuse seqs risk and md already hold and their `seq <= tip` gates drop them. GUARANTEES §1.1 assumes replicas the ME does not have |
| ME-CONFIG-STATUS-IGNORED (LOW) | **proposal** | 21 #4: the spec names "status" but not which values halt or what the ME emits for orders on a halted symbol |
| ME-REPLICA-TAKEOVER (design) | **proposal** | 21 #11, 6 #3 |
| ME-WAL-BACKPRESSURE-EXITS (MED) | **fix now** | 48:83-98, :136-137 say stall; the ME exits on `WouldBlock` (`order.rs:123`), never calls `should_flush`, and warns past flush errors (`cast_loop.rs:225-227`) |
| ME-BOOT-IGNORES-APPLIED-CONFIG (MED) | **fix now** | 19:65-69 says the ME reloads tick/lot/decimals from `symbol_config_applied` on cold start; `app.rs:115-120` keeps only the version |
| ME-BENCH-NO-P99 (LOW) | **fix now** | repo rule + 22: no ME bench or report carries p99; no ME entry in `bench-baseline.json` |
| ME-REPLICA-LAG-BACKPRESSURE, ME-WAL-RETENTION-GC-ON-ROTATE-ONLY | **proposal** (rsx-cast frozen) | 48:138 needs consumer feedback; `prune_old_segments` runs only inside `WalWriter::rotate` |
| ME-WAL-OFFLOAD-WORKER | **proposal / spec decision** | 48:100 offload worker vs the recorder that archives today |

Split: **14 fix-now items in 11 entries** (11 spec gaps from section 1 plus ME-PG-DRIVER-INLINE-SPAWN,
ME-REPLAY-ERR-SKIPS-SET-NEXT-SEQ-SEQ-REUSE, ME-REPLAY-SILENT-SKIP-UNDECODABLE-RECORD),
**10 proposals** (ME-CANCEL-MISS-SILENT, ME-REPLAY-IGNORES-CONFIG-CHANGES,
ME-CRASH-BEFORE-FLUSH-SEQ-REUSE, ME-CONFIG-STATUS-IGNORED, ME-REPLICA-TAKEOVER,
ME-REPLICA-LAG-BACKPRESSURE, ME-WAL-RETENTION-GC-ON-ROTATE-ONLY,
ME-WAL-OFFLOAD-WORKER, plus the kept ME-GW-DIRECT-SPEC-GAPS and
ME-HOLDS-USER-STATE), **1 decision** (dedup window), **12 stale/fixed entries
to prune, re-home or rewrite**.

## 3. LOC — what to cut in `rsx-matching/src` (2,348 lines at `21049460`, 147 of them `src/*_test.rs`)

| Cut | Evidence | Approx. |
|---|---|---|
| Dead idle-migration branch + the comment that describes it | `cast_loop.rs:189-195`; `order.rs:210-214` says "Migration then proceeds lazily (resolve_level per touched price) and on idle cycles (migrate_batch below)", false since `recenter_now` is eager | −8 |
| Four copies of "prepare → append → send risk → send md" | `order.rs:77-100` (dup reject), `order.rs:119-137` (accepted), `config.rs:186-196` (`emit_config_applied`), `wal.rs:333-339` (`FanoutSink`). One `fan_out(record, what)` with warn-on-send replaces all four and fixes ME-FANOUT-CAST-SEND-PANICS | −35 |
| Replay rebuilds `IncomingOrder` by hand | `wal.rs:391-407` duplicates `wire::to_incoming` (`wire.rs:9-27`); one side/tif decoder serves both and carries the enum check | −15 |
| Row → `ScheduledConfig` mapping twice | `config.rs:50-62` and `config.rs:125-136` | −12 |
| Env socket-address boilerplate ×4 + 14 `// SAFETY: fail-fast at startup` comments | `app.rs:222-256` (one `env_addr(key, default)` helper; `get_env` in `config.rs:138-151` is the precedent); the SAFETY markers sit on ordinary `.expect`s (rsx-risk dropped the same markers) | −35 |
| `log_effective_matching_config`: a 22-line fn called once | `app.rs:41-62` → one `info!` at the call site | −12 |
| Loop timers built in `app.rs`, passed through `LoopState`, destructured | `app.rs:216-219`, `cast_loop.rs:54-57, 77-80` → `cast_loop::run` creates them | −12 |
| PG connect: nested `match` with inline spawn | `app.rs:100-140` → named `connect_db` + `drive_pg_connection` | −10 |
| `process_cancel` takes three loose ids and leaves the index update to the caller | `cancel.rs:21-31`, `cast_loop.rs:143-155` → take `&CancelRequest`, update the index inside like `process_order` does | −6 |
| Fictional `dedup_map` cap 65 536 in `/metrics` | `app.rs:330-334`; the set has no cap | 0 (truth fix) |

About −145 lines in `src`; the config sidecar (step 5) adds ~60 back.
Tests (outside `src`, cut in step 8): the five tautologies and two duplicate
tests listed under 41 (≈ −110), replaced by tests of the ME tile.

Not cut: the legacy RXSN checkpoint reader (`checkpoint.rs:208-233`) — 21
§2.8 Recovery step 1 requires it; dropping it is a founder call.
`save_checkpoint` (`checkpoint.rs:249-257`) is test-only but 9 lines and the
tests' simplest entry; leave.

## 4. Playground

Surveyed by a read-only sub-agent; the gateway/risk-only counter fetch
(`server.py:3992-3996`), the local-only recent-orders cancel
(`server.py:6270-6280`) and the `/api/book` source were re-checked by hand.
Paths are under `rsx-playground/`.

**Shown today**
- ME process up/down, pid, restarts: `scan_processes` (`server.py:1666`),
  `/x/processes`, topology node `_topo_matching` (`server.py:2597`); ladder
  marked "[STALE - ME down]" (`_me_live`, `server.py:4402`).
- Ladder/BBO — from marketdata (`_book_snap`), falling back to the ME WAL BBO
  or `snapshot.bin`, then a synthetic maker book (`server.py:8505-8527`).
- Order entry with GTC/IOC and reduce-only controls (`pages.py:1396-1418`,
  `3235-3265`); outcome classes filled/resting/cancelled/rejected incl.
  duplicate (3) and reduce-only (7) reasons (`server.py:5258-5260`).
- Real cancel by CID on the trade page only (`server.py:5740`).
- ME WAL: per-stream files, timeline, `rsx-cli` dump (`server.py:3170`,
  `4067`, `7075`); decoder for FILL/BBO/ACCEPTED/INSERTED/CANCELLED/DONE.
- WAL-derived invariant checks (`server.py:5815+`); ME kill/restart buttons
  (`server.py:4693`, `4901`).
- Config page: scheduled rows only (`server.py:3040-3079`).

**Missing (what step 12–13 add)**
- Any ME `/health`, `/ready`, `/metrics` read: `_fetch_daemon_counter` is
  wired for `gw-` and `risk-` only; `me_to_mkt` is a WAL record count. No
  `drops` (FAULTED skips), no dedup size, no reject-by-reason.
- FOK and post-only controls (the server already maps both,
  `server.py:5366`, `5455`; `play_orders.spec.ts:8-9, 215` note the gap) and
  their outcomes (FAIL_FOK, post-only cancel).
- ME dedup demo: the only "duplicate" path is the playground's own
  `x-idempotency-key` (`server.py:5581`), which never reaches the ME.
- Recent-orders "Cancel" flips local state only (`server.py:6270-6280`);
  `/api/orders/batch`, `/random`, `/invalid` never reach the ME.
- WAL: no ORDER_FAILED / CONFIG_APPLIED decode or filter, no per-ME WAL tip.
- Applied vs scheduled config (`symbol_config_applied` never read).
- Recovery: kill/restart shown, but nothing shows the book came back
  (checkpoint seq, restored order count).

**Tests touching the ME today:** `play_guarantees.spec.ts:253` (kill ME,
cross two orders, check exactly-one + tips), `play_faults.spec.ts:120`,
`play_health_truthful.spec.ts:103`, `play_book.spec.ts`, `play_wal.spec.ts:64,
161`, `play_orders.spec.ts:145-213`; API tests assert HTTP 200 or recorded
flags only (`api_processes_test.py:441-453`, `api_orders_test.py:175-188`).
`acceptance_test.py` is the one real fill/cancel/restart test and no make
target runs it.

Run: `make gate` (startup → partials → API list in `_gate-api` → full
Playwright). A new `tests/api_matching_test.py` must be added to the
`_gate-api` list; a new `tests/play_matching.spec.ts` goes in the
`process-control` project's `testMatch`. Latency stays out of play specs.

## 5. Steps

One `sonnet` agent and one commit per step, sequential on the shared tree;
the parent reads each diff before starting the next. rsx-cast is not edited
in any step: every fix below lives in the caller, using `WalWriter` /
`CastSender` APIs that already exist. "Counts" = `cargo test -p rsx-matching`
pass/ignored totals, recorded before the step.

| # | Step (commit) | Done when | Verify |
|---|---|---|---|
| 0 | `docs(bugs)`: triage per section 2 — prune the 9 fixed/resolved ME entries, prune ME-SNAPSHOT-SYNCHRONOUS-HOT-LOOP, move ME-MAKER-ORPHANED-QUOTES-WEDGE-BOOK under maker/gateway, rewrite ME-ENGINE-LATENCY-NOT-REPORTED to its gateway remainder, re-cite the live entries to current lines, reclassify ME-CONFIG-POLL-BLOCKS-PINNED-LOOP as fix-now, add the new entries | every ME-* entry cites a file:line that exists at HEAD | `grep -n 'rsx-matching/src/main.rs:' BUGS.md` → nothing; `git diff --stat` touches BUGS.md only |
| 1 | `fix(matching)`: one `fan_out` path for every WAL-framed record — `FanoutSink`, the dup-reject and accepted blocks in `order.rs`, `emit_config_applied` — WAL append panics with its invariant message, each cast send warns and the other send still runs (ME-FANOUT-CAST-SEND-PANICS; LOC −35) | a failing risk send leaves the record in the WAL, still reaches md, returns `Ok` | new `src/wal_test.rs` case with a sender whose `send_to` fails (e.g. IPv6 dest on an IPv4 socket); `cargo test -p rsx-matching`; `make lint` |
| 2 | `fix(matching)`: WAL backpressure per 48 — on `WouldBlock` flush and retry (stall, not exit); flush when `should_flush()`; a flush error keeps the loop stalled and retrying rather than buffering on | appends past the 2× cap succeed after a stall; 1000 appends trigger a flush | tests with a small `max_file_size` writer; `cargo test -p rsx-matching` |
| 3 | `fix(matching)`: fail-loud recovery — move the `app.rs:142-214` sequence into a lib fn (`wal::recover` or `recovery.rs`) returning the book, index, dedup and next seq; replay `Err` aborts boot; a handled type with an undecodable payload is an `Err`; unknown types warn once per type (ME-REPLAY-ERR-SKIPS-SET-NEXT-SEQ-SEQ-REUSE, ME-REPLAY-SILENT-SKIP-UNDECODABLE-RECORD, 10:502) | truncated ACCEPTED payload → `Err`; snapshot-only boot → next seq = `wal_seq + 1` | new cases in `tests/replay_after_snapshot_test.rs`; `cargo test -p rsx-matching` |
| 4 | `fix(matching)`: boot config from Postgres — named `connect_db` + `drive_pg_connection` (ME-PG-DRIVER-INLINE-SPAWN); when a `symbol_config_applied` row exists its tick/lot/decimals are the book config (fresh book, and `update_config` after checkpoint restore, before replay), per 19:65-69; env stays the no-DB fallback | a restarted ME with applied v2 runs v2's tick even with env v1 | new `#[ignore]` testcontainers case in `tests/config_poll_test.rs` on the extracted boot fn; `cargo test -p rsx-matching -- --ignored --test-threads=1`; `make integration` |
| 5 | `fix(matching)`: config poll off the pinned loop — a named sidecar thread owns the runtime + client, polls every 600 s (and once at start, flagged as the one behaviour choice the spec leaves open), hands due `ScheduledConfig`s over a 64-slot `rtrb` ring; the loop drains it each spin → `update_config` + CONFIG_APPLIED; the applied upsert goes back over a return ring (rsx-risk shape, `rsx-risk/src/app.rs:286-289`). Replace the five tautological `config_test.rs` cases with tests of the drain fn (ME-CONFIG-POLL-BLOCKS-PINNED-LOOP) | no `block_on` reachable from `cast_loop.rs` | `grep -n block_on rsx-matching/src/cast_loop.rs rsx-matching/src/config.rs` → only the sidecar; `cargo test -p rsx-matching`; `make integration` |
| 6 | `fix(matching)`: wire hygiene — one side/tif decoder shared by `to_incoming` and replay (LOC −15); unknown side/tif → `OrderFailed(Validation)` through the dup-reject path, before any ACCEPTED record (47 §1.4); cancel reasons written as `CANCEL_REASON_*` (User 0, ReduceOnly 1, PostOnly 4) | post-only cancel record carries 4; tif=7 order yields one ORDER_FAILED and no ACCEPTED | `tests/wire_test.rs`, `tests/wal_test.rs` cases; `cargo test -p rsx-matching -p rsx-cli` |
| 7 | `refactor(matching)`: LOC cuts from section 3 not already taken — dead `migrate_batch` branch + its comment, `env_addr` helper + the 14 SAFETY markers, inline `log_effective_matching_config`, loop timers into `cast_loop::run`, one row→`ScheduledConfig` mapper, `process_cancel(&CancelRequest)` updating the index itself. No behaviour change | `wc -l rsx-matching/src/*.rs` below the step-6 total; counts unchanged | `cargo test -p rsx-matching` (same counts); `make lint`; `make fmt-check` |
| 8 | `test(matching)`: tests of the ME tile — `src/order_test.rs`, `src/cancel_test.rs` in the binary: dup reject on both streams, ACCEPTED precedes events, `me_in_ns <= match_done_ns` on fills (59), cancel → CANCELLED + DONE + BBO, cancel miss emits nothing (pins today's behaviour until ME-CANCEL-MISS-SILENT is decided), self-trade fills (47 §3.6); a counting-allocator test that `process_new_order` + `publish_events` on a warmed book and WAL buffer allocate nothing (21 §7, 41 M23; the dedup and index maps may grow by design, `notes/zero-heap.md`, so they stay outside the measured span — if the WAL buffer itself allocates, record it in BUGS.md instead of loosening the test); delete the duplicate `event_test.rs` cases and `event_buffer_max_capacity` | 41 M6, M23, M25, M27, 47 #6/#9/#10, 17 #2–4, #7, #8, #10 move U → T | `cargo test -p rsx-matching`; `make test` |
| 9 | `feat(matching)`: `/metrics` truth — export `drops`, ORDER_FAILED counts by reason, `wal_last_seq`, last checkpoint `wal_seq` and age, applied `config_version`; drop the fictional 65 536 dedup cap (47 §7) | `curl $RSX_ME_HEALTH_ADDR/metrics` lists them | unit test on the snapshot closure; `cargo test -p rsx-matching` |
| 10 | `perf(matching)`: tail harness for the ME accept path (`process_order` shape: dedup + ACCEPTED + match + publish + index) and checkpoint capture pause, both p50 + p99 from the same run (rsx-book `tail_bench.rs` is the model); add ME entries to `bench-baseline.json`; write `reports/YYYYMMDD_me-accept-tail.md`; point the crate README at it | report has p50 and p99 per row; `make perf-gate` lists the ME benches as gated, not NEW | `cargo bench -p rsx-matching --bench <new>`; `make perf-gate` |
| 11 | `docs(specs)`: reconcile the S rows — 17 (paths, md-gets-everything, `current_bbo`, `cancel_order_checked`, boot config), 21 §2.7/§2.8 (eager recenter citing RECENTER-EAGER-TAIL-SPIKE; replica as proposal), 6 §3/§5, 41 (M2, M21, ring E2E rows), 47 §3.3/§3.5/layers, 18 `risk_in_ns`, 10/48 SIGTERM + stall, 19/57/5/9/20/30/44/45 stale citations and env tables, dedup window per the founder's call; crate README/ARCHITECTURE/`notes/single-pinned-loop.md` after step 5 | no spec cites `wal_integration.rs` or `rsx-matching/src/main.rs:` | `grep -rn 'wal_integration\|rsx-matching/src/main.rs:' specs rsx-matching` → nothing |
| 12 | `feat(playground)`: ME backend — read each `me-*` `/ready` + `/metrics` (extend `_fetch_daemon_counter` beyond `gw-`/`risk-`), per-ME card (orders, publishes, rejects by reason, drops, dedup size, WAL tip, checkpoint seq/age, applied config version); WAL decoder + filters for ORDER_FAILED and CONFIG_APPLIED; config page shows applied vs scheduled; new `tests/api_matching_test.py` added to `_gate-api` | partials render with live numbers | `cd rsx-playground && uv run pytest tests/api_matching_test.py tests/test_htmx_partials.py` |
| 13 | `feat(playground)`: ME features in the UI — FOK and post-only controls with FAIL_FOK / post-only-cancel outcomes; recent-orders Cancel sends a real gateway cancel; "resend same order id" that bypasses the playground idempotency key to show the ME duplicate reject; recovery card (kill `me-<sym>` → resting order count and checkpoint seq before/after); `tests/play_matching.spec.ts` in the `process-control` project (functional only, no latency asserts) | each feature visible and asserted | `cd rsx-playground/tests && bunx playwright test play_matching.spec.ts` |
| 14 | `docs(ship)` + diary: gates and ledger | `make test`, `make integration`, `make lint`, `make fmt-check`, `make gate` green (gate 4 needs a quiet box — record load if it fails, as for risk); BUGS.md pruned; PLAN.md ledger row; diary entry | the gate output itself |

Steps 1–6 can swap order only where files do not overlap (2 depends on 1's
single fan-out site; 5 depends on 4's `connect_db`). Step 11 waits for the
dedup-window decision; everything else is unblocked.

**Founder calls this plan needs (not built):** dedup window 5 min vs 1 h;
ME-CANCEL-MISS-SILENT; ME-REPLAY-IGNORES-CONFIG-CHANGES;
ME-CRASH-BEFORE-FLUSH-SEQ-REUSE; ME-CONFIG-STATUS-IGNORED; replica /
hot-spare ME; replica-lag backpressure and retention GC (both need rsx-cast);
the 48 offload worker vs the recorder; dropping the legacy RXSN checkpoint
reader.
