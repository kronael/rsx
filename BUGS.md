# Bug queue

The review queue: **OPEN** and **DEFERRED** items, the decisions the owner
still owes, and the rulings that keep a settled finding from coming back.
Resolved bugs live in git and `CHANGELOG.md`, not here. No entry has a failing
test pinned yet (no `known_defect_*` test exists); the fix commit adds its
test. Sections group by crate or surface.

## rsx-cast (frozen)

The crate takes bugfixes that restore documented behaviour; anything else needs
the founder's sign-off before code (`rsx-cast/CLAUDE.md`). Every entry here
waits for that word.

- **CAST-SEND-RING-TOO-SMALL** (MED, correctness) — `SEND_RING_FRAME_BYTES` is
  128 (`rsx-cast/src/cast.rs:76`), but `FillRecord`, `BboRecord`,
  `OrderDoneRecord`, `OrderAcceptedRecord` and `OrderMessage` are 128-byte
  payloads (asserted in `rsx-messages/src/lib.rs`), so the frame is 16 + 128 =
  144 bytes and `send_framed` takes the large-record branch (`cast.rs:212`),
  which never caches the frame. Every NAK for these types misses the in-memory
  ring and reads the disk through `read_record_at_seq`: the ME streams and
  risk's gateway stream (fills and terminals) depend on that WAL read, and
  risk's orders to the ME stay drop-safe only through the gateway's TIMEOUT. The
  comment on the constant ("all <= 64 bytes payload") is false. Not a loss (the
  disk fallback works). needs sign-off.
  **Options:** (a) size the ring frame to at least 144; (b) leave it.
  **Default if nothing is decided:** (b).
- **CAST-CAUGHT-UP-LIVE-SEQ-ECHOES-FROM-SEQ** (LOW, correctness) — when no WAL
  record is at or after the requested `from_seq`, the replication server sends
  nothing and `CaughtUpRecord.live_seq` is `from_seq`
  (`rsx-cast/src/replication_server.rs`, `last_seq = from_seq`), a seq the
  stream does not hold. With records it is the last streamed seq, so the
  marker means two things. `rsx-risk` (warm catch-up) and `rsx-marketdata`
  (bootstrap) treat a round that streamed no record as level and ignore
  `live_seq` in that case; a new consumer must do the same. needs sign-off.
  **Options:** (a) report `from_seq - 1` when nothing was sent; (b) leave the
  consumers' rule. **Default if nothing is decided:** (b).
- **CAST-REPLICATION-CLIENT-TESTS-MODEL-NOT-EXERCISE** (MED, test) —
  `rsx-cast/src/replication_client_test.rs` cases `consumer_dedup_by_seq`,
  `consumer_tip_never_decreases_on_reordered_seq`, `consumer_sends_tip_plus_1`
  and the backoff tests assert logic re-implemented on bare variables, not
  `ReplicationConsumer`, so they pass whatever the consumer does;
  `specs/2/36-testing-replication.md` § Edge Case Tests lists them as planned.
  A test-only change in the frozen crate needs sign-off. **Fix:** drive
  `ReplicationConsumer` over a loopback `ReplicationService`. No test — test.
- **CAST-NAK-WAL-FALLBACK-TEST-LOAD-FLAKE** (LOW, test) —
  `rsx-cast/tests/nak_wal_fallback_test.rs` `nak_wal_fallback_delivers_evicted_seq`
  failed once under a full parallel `make test` on this loaded shared box
  ("WAL-fallback retransmit did not deliver seq=1" after 9.9 s), then passed 3
  of 3 isolated runs in about 0.1 s and a clean full rerun. A timing-window UDP
  retransmit assertion, load-sensitive, not a code regression. needs sign-off
  (frozen crate). **Fix:** widen the retransmit wait window or gate the test's
  deadline on wall-clock load. No test — test.
- **CAST-UNUSED-DEPENDENCIES** (LOW, config) — `webpki-roots`, `toml` and
  `chrono` (`rsx-cast/Cargo.toml:17`, `:22`, `:23`) have no reference in
  `rsx-cast/src`, `tests` or `benches`, so cargo's `unused_dependencies` lint
  prints three warnings on every `cargo check`. needs sign-off (frozen crate).
  **Fix:** delete the three lines. No test — config.
- **CAST-DOC-LINKS-TO-REMOVED-POLL** (LOW, docs) — the doc comments at
  `rsx-cast/src/cast.rs:422` and `:824-825` link `CastReceiver::poll` and say
  "prefer `poll`"; the method is `try_recv_with` (`cast.rs:626`), so the links
  dangle. needs sign-off (frozen crate, doc-only). **Fix:** name
  `try_recv_with`. No test — docs.
- **CAST-WAL-READER-GAP-SEEK-SKIPS-SEGMENTS** (MED, correctness) —
  `pick_start_idx` (`rsx-cast/src/wal.rs:576`), which `WalReader::open_from_seq`
  uses to choose the first file, returns the last file (the active one) when
  `target_seq` falls between two segments' `[first_seq, last_seq]` ranges: the
  loop only returns on a covering match and otherwise leaves `idx` at the last
  entry. A rotated segment whose first seq is above the target is skipped, and
  a replay or live-tail reader opened in a seq gap streams from the active file
  without it. Gaps exist after a restart that does not continue the old seq
  (ME-CRASH-BEFORE-FLUSH-SEQ-REUSE's boot jump would make one). Read from the
  code, not run. needs sign-off (frozen crate; a reader behaviour change beyond
  the restored seek). **Fix:** return the first file whose `last_seq` is at or
  above the target (a gap target starts at the next segment), the active file
  only when none does. **Options:** (a) that fix with a `wal_test.rs` case for a
  gap between two rotated segments; (b) leave it. **Default if nothing is
  decided:** (b).
- **CAST-GC-OWNER-DOCS-CONTRADICT** (LOW, docs) — `rsx-cast/CLAUDE.md` § Rules
  says "moving segments to an archive or deleting them is not the writer's
  job", while `rsx-cast/ARCHITECTURE.md` (§ WalWriter internals, "GC":
  `prune_old_segments` at the end of every rotate; file table, "4h retention
  GC") and the code (`prune_old_segments`, `rsx-cast/src/wal.rs:312`, called
  from `rotate`, `wal.rs:294`, with `RETENTION_NS`, `wal.rs:87`) make the writer
  delete segments older than 4 h. owner decision (frozen crate). **Options:**
  (a) the rule is true: delete the writer's prune and leave retention to the
  recorder archive and an external cleanup; (b) the code is true: reword the
  rule. **Default if nothing is decided:** (b), the code stays. No test — docs.
- **CAST-DOC-STALE-LISTENER-AND-SIZE-CLAIMS** (LOW, docs) —
  `rsx-cast/ARCHITECTURE.md` lines 66 and 164 say the replication live tail
  follows `WalWriter::add_listener` notifications; no such method exists and the
  live tail polls every 100 ms (`rsx-cast/src/replication_server.rs:179`). The
  doc comment at `rsx-cast/src/cast.rs:63` says "Live records are <= 64 bytes";
  the live records include 128-byte payloads (CAST-SEND-RING-TOO-SMALL). needs
  sign-off (frozen crate, doc-only). **Fix:** describe the 100 ms poll and
  state the sizes. No test — docs.
- **TIME-NS-HOTPATH-AUDIT** (LOW, perf) — `time_ns()` (`clock_gettime` through
  the vDSO, about 15-30 ns) is called per record in a few places (replication
  server CaughtUp and record stamping, WAL timestamps). Audit the call
  frequency; if one is a per-record hot path, stamp once per batch or use a
  cached clock. Not correctness. deferred — needs a profile first. No test —
  perf.
- **REPLAY-RESCAN-PER-CONNECTION** (MED, perf) — `oldest_and_highest_seq`
  (`rsx-cast/src/wal.rs:648`) full-scans the active WAL file (up to 64 MB) on
  every replay connection to validate the requested range (`handle_client`,
  `replication_server.rs:120`). The live tail also reopens the active file every
  100 ms (`replication_server.rs:179`) and reads it from byte 0, skipping every
  record below the next seq to send (`WalReader::next`, `target_seq`): it sends
  only new records but still pays the read. Fine for occasional bootstraps;
  under a reconnect storm replay becomes I/O-bound. The `outage_recovery` bench shows
  the per-cycle recovery of a constant 7.5 k-record gap drifting 262 to 786 ms
  across six cycles because the active WAL grows. needs sign-off. **Options:**
  (a) an incremental tail scan (cache highest and the active-file offset);
  (b) a producer-published highest-tip file; (c) leave it. **Default if
  nothing is decided:** (c). No test — perf.

## rsx-matching

- **ME-CRASH-BEFORE-FLUSH-SEQ-REUSE** (MED, correctness) — records are cast to
  risk and marketdata when appended (`fan_out`, `rsx-matching/src/wal.rs:444`;
  the accepted record in `order.rs`) but reach disk at the next 10 ms flush
  (`flush_if_due`, `wal.rs:494`). A crash in that window loses records both
  consumers already hold; boot sets `next_seq` from the replayed disk tail
  (`Recovered::next_seq`, `recovery.rs:162`), so new records reuse the lost
  seqs and the consumers' `seq <= tip` gates (`rsx-risk/src/position.rs`,
  `process_fill`) drop them as duplicates. `GUARANTEES.md` §1.1 and
  `specs/2/48-wal.md` state the window and the reuse; "fills are never lost"
  needs the replica that ME-REPLICA-TAKEOVER leaves unbuilt. needs sign-off.
  **Options:** (a) cast only after the covering flush, which adds up to 10 ms
  to every fill and contradicts the <50 us target; (b) accept the single-node
  window as stated; (c) boot at `disk_tail + 1 + G`, G above the unflushed
  bound of the WAL buffer, with `wal verify` reading a gap of exactly G as a
  restart: every consumer then NAKs the hole, and `handle_nak`
  (`rsx-cast/src/cast.rs`) runs `read_record_at_seq` for each of up to 4096
  seqs, each a scan of the active file (up to 64 MB) on the pinned loop, unless
  `WalWriter::set_next_seq` (`rsx-cast/src/wal.rs:168`) starts a new segment on
  a non-contiguous seq so that a hole seq costs one record read: a frozen-crate
  behaviour change (owner question open; it also makes the seq gaps that
  CAST-WAL-READER-GAP-SEEK-SKIPS-SEGMENTS mishandles). **Default if nothing is
  decided:** (b). Measured: `POST /api/matching/crash/me-pengu` under maker
  load, 2026-10-21: `wal_last_seq` 15650 before the SIGKILL, 15629 after
  recovery; the Matching tab's recovery card shows the tip going back.
- **ME-CONFIG-STATUS-IGNORED** (LOW, design) — `specs/2/21-orderbook.md:78`
  ("Matching validates tick/lot sizes and status") and `specs/2/19-metadata.md`
  (`status` active/paused/halted, `min_notional`, `max_order_qty`).
  `poll_scheduled_configs` (`rsx-matching/src/config.rs`) never selects those
  columns (`symbol_config_schedule.status` exists,
  `rsx-risk/migrations/005_symbol_config.sql`) and `write_applied_config`
  always writes `'active'`; no `OrderFailureReason` exists for a halted symbol.
  The spec names `status` but not which values halt or what the ME emits for an
  order on a halted symbol. owner decision. **Options:** (a) halt rejects new
  orders with a new reason and still allows cancels; (b) `status` is
  Gateway/Risk-only, strike it from 21 § 2.9. **Default if nothing is
  decided:** (b), the ME ignores it. Until a halted reject exists, risk
  treats every rejected liquidation order as an unfilled round
  (`13-liquidator.md` §10); building (a) adds the liquidator's pause on it.
  No test — design.
- **ME-REPLICA-TAKEOVER** (MED, design) — `specs/2/21-orderbook.md` § Replica
  Takeover (a ReplicationConsumer tip and a Postgres advisory lock),
  `specs/2/6-consistency.md:45` (a mirrored stream to a hot-spare ME) and
  `specs/2/41-testing-matching.md:142-148`: the crate has no replica mode and
  no lease (rsx-risk has `lease.rs` and `promotion.rs`). Not built; those
  sections still read as shipped (21 is `status: shipped`). needs sign-off.
  **Options:** (a) build the replica on the rsx-risk lease shape; (b) mark the
  sections "not implemented" in 21, 6 and 41. **Default if nothing is
  decided:** (b). No test — design.
- **ME-CAPTURE-PAUSE-SCALES-WITH-SLAB** (MED, perf) — the checkpoint capture
  that runs on the pinned loop every 10 s (`rsx-matching/src/cast_loop.rs`,
  `MatchingCheckpointV1::capture`, `rsx_book::snapshot::capture`) paused it for
  17-19 ms p50 at 100,000 resting orders and 209-262 ms p50 at 1,000,000
  (`reports/20261020_matching-p99.md`, noisy shared host, three runs). With
  100,000 live orders on a 1,000,000-slot slab it is 148-157 ms p50: the cost
  follows the slab high-water mark, because `capture` sweeps every slot below
  it and walks the free list, not the live order count. Orders arriving during
  the pause wait in the UDP receive buffer; whether a pause this long trips a
  Risk or ME timeout is not measured. The split of the time (slot sweep,
  free-list walk, snapshot allocation) is not profiled. needs sign-off.
  **Options:** (a) profile first, then bound the work per spin (capture in
  slices across loop iterations, consistent because the loop is
  single-threaded); (b) accept it and document the resting-order count above
  which the pause exceeds the timeout. **Default if nothing is decided:** (b).
- **ME-BENCH-NO-P99** (LOW, perf) — `rsx-matching/benches/me_tail_bench.rs`
  reports p50 and p99 for the order path and the checkpoint capture pause, its
  rows are in `bench-baseline.json`, and the dated run is
  `reports/20261020_matching-p99.md`. Still missing: the six Criterion benches
  (`matching_bench`, `process_order_bench`, `match_n_levels_bench`,
  `match_depth_bench`, `match_by_type_bench`, `wal_replay_bench`) report no p99
  and have no `bench-baseline.json` entry, and
  `reports/20260716_book-matching-publication.md` (the crate README's evidence
  link) has no p99. The tail rows' baseline is the largest of three runs on a
  host at 86-95% CPU pressure, so it is lenient; re-save it on a quiet box
  (MATCHING-BENCH-SHARED-HOST-VARIANCE). **Fix:** baseline entries for the
  Criterion ME benches, from a quiet box with the cluster stopped. No test —
  perf.
- **MATCHING-BENCH-SHARED-HOST-VARIANCE** (MED, methodology) — the published
  matching latencies do not reproduce on the shared docker host. 2026-07-09
  idle-box run: `match_by_depth` about 48 ns (vs 30 ns), `me_accept_path/full`
  about 295 ns (vs 266 ns); throughput about 3.46M/s (vs 3.6M, stable, depth
  independence holds). Not a regression (the pre-refactor re-bench already saw
  about 51 ns): shared-host variance of about 1.6x on the tight per-op
  benches. **Fix:** measure on a dedicated box before publishing 30/266 as a
  floor, or re-baseline the docs with an honest variance note. Full-workspace
  `make perf-gate` is nightly-scale (times out in a session). See
  `reports/20260709_matching-benches.md`. No test — needs a quiet box.
- **ME-FLUSH-OVER-10MS-UNDER-LOAD** (LOW, latency) — me-pengu logged `flush
  took 10-14ms` (the warn in `WalWriter::flush`, `rsx-cast/src/wal.rs`) against
  the 10 ms target while marketdata logged `seq gap` warnings (dropped casts).
  Seen once, not reproduced. **Fix:** measure flush latency on a quiet box with
  the perf gate. No test — needs a quiet box.

## rsx-book

- **RECENTER-EAGER-TAIL-SPIKE** (MED, latency) — `maybe_recenter`
  (`rsx-matching/src/order.rs:141`) calls `Orderbook::recenter_now`
  (`rsx-book/src/migration.rs:234`), which migrates the whole book eagerly in
  one call (`complete_migration_eager`, `migration.rs:216`). The cost is
  bounded by the old slot count (about 617k for BTC-PERP), not the order count:
  it scans the old slot array, skips empty levels in O(1) and remaps the
  occupied ones. That is a spike far above a 60 ns match, landing on whichever
  order triggers a >2.5% mid drift, that is, during volatility. Eager is
  deliberate: a lazy per-order `resolve_level(price)` scheme would let a
  marketable order's crossing prices lie outside a partially migrated band and
  miss crossing liquidity (`recenter_now` contract, `migration.rs`). The
  incremental `migrate_batch` walk (`migration.rs:177`) is test and non-live
  only. `docs/concepts/05-slab-and-compression.md` documents the trade. The
  code contradicts `rsx-book/CLAUDE.md` § Rules ("recentering must be lazy and
  incremental, never a one-shot migration on an order's critical path") on
  purpose: the eager migration is the correct one for live matching. needs
  sign-off. **Options:** (a) a correct incremental migration for live matching
  (migrate the crossing band eagerly and the rest on idle, or consult both maps
  during the window), after a design and an adversarial review, which keeps the
  rule; (b) keep the bounded spike and reword the rule to the eager contract
  (`recenter_now`). **Default if nothing is decided:** (b), the code stays and
  the rule stays unreworded until the owner answers. No test — design.
- **BOOK-DEMO-GIFS-CARRY-OLD-COLD-OPEN** (LOW, docs) — `rsx-book/demo/bench-live.sh`
  prints the report's p50 and p99 on the cold-open card, but the tracked
  `book-live-opt.gif` and the `assets/book-versus.gif` intro still carry the
  old "~70 ns, flat" text and the renamed lane label until they are
  re-recorded on a quiet box. **Fix:** `make rec && make gif` and rebuild the
  intro. No test — needs a quiet box.

## rsx-gateway

- **GW-RISK-REPLICATION-ADDR-NAMES-ONE-NODE** (MED, design) — the gateway's
  `RSX_RISK_REPLICATION_ADDR` names one risk node, and only that node's
  `GwStream` WAL holds the stream it serves. After a promotion the new main
  numbers its gateway stream from its own WAL tail; the gateway's receiver
  re-syncs on it (sender-reset path) but a later FAULTED replays from the old
  node, or finds nothing there, and skips the gap. `rsx-cli` gives only the
  primary a `RSX_RISK_REPLICATION_BIND_ADDR`. Read from the code, not run.
  needs sign-off. **Options:** (a) the gateway follows the main (the address
  comes from the lease holder's record in Postgres); (b) every replica binds
  its own address and the gateway tries them in order. **Default if nothing is
  decided:** neither; a gap after a promotion is skipped and the client misses
  that update.
- **GW-BENCH-NO-P99** (LOW, perf) — the repo rule is p50 and p99 in every
  latency table, gated. `rsx-gateway/benches/gw_tail_bench.rs` reports p50 and
  p99 from one run for the order path (`order_accept`) and the route path
  (`route_inserted`, `route_done`, `route_fill`) into
  `target/tail/rsx-gateway.json`, and `bench-baseline.json` holds its eight
  `gw_tail/*` keys. Those keys are twice one run on a shared 2-core box at load
  average 10 (CPU pressure 40-80%), a placeholder that cannot fail on noise; the
  route rows read about 4 ms p50 against 21 us for `order_accept`, with the
  stamp the reactor puts on the frame already about 3.9 ms after the send and
  the bench thread's `schedstat` at 11.9 s on the run queue to 0.5 s on CPU
  (inference: the busy-spinning `gw-cast-recv` thread waits for time slices),
  so they measure the scheduler. Still open: (1) re-seed the `gw_tail/*` keys
  from a quiet-box run; (2) the Criterion gateway benches (`gateway_bench`,
  `ws_parse_bench`, `ws_encode_bench`, `jwt_validate_bench`) report means and
  have no key in `bench-baseline.json`, so `scripts/bench-gate.sh` prints NEW
  and never fails them; (3) the dated report
  `reports/YYYYMMDD_gateway-tail.md` and the crate README's pointer to it (the
  README names only `ws_order_latency`). No test — perf.
- **GATEWAY-LATENCY-QUIET-BOX-REMEASURE** (MED, methodology) — the risk to
  gateway recv runs on its own thread, SPSC ring and doorbell (`cast_recv.rs`),
  and the handler wakes on `select!{readable, egress.wait()}` (`handler.rs`),
  so neither the shared-reactor starvation nor the egress poll remains. The
  effect on `match_done_to_gateway` is not quantified: the 8-sample
  before/after medians (411.6 to 539.3 us) overlap inside host noise (load
  average 8 to 20 on an 8-core box with unrelated tenants; the untouched legs
  stayed tight), see `reports/20260723_gateway-egress.md`. The 11.5 ms
  single-connection WS p50 that motivated the split is in
  `reports/20260530_gateway-ws-rest-latency.md`. **Fix:** re-measure with
  `ws_order_latency` on a quiet box with the recv thread pinned
  (`RSX_GW_CAST_CORE_ID`), p50 and p99 from the same run (GW-BENCH-NO-P99). No
  test — needs a quiet box.

## rsx-cli

- **CLUSTER-RESTART-PRIMARY-HANDS-THE-LOCK-TO-A-REPLICA** (LOW, correctness)
  — `cluster restart risk-0` in a scenario with warm replicas (`full`,
  `stress`) stops the primary, a warm replica takes the released advisory lock
  and is promoted, and the restarted `risk-0` stays in `warm_catchup`: the
  restart fails with "not ready after N s: risk-0 (warm_catchup)" and the
  gateway, which names `risk-0`'s cast address, reaches no main. This is the
  failover working as designed; the launcher has no notion of which node
  holds the lock. Reproduced on `full`; `minimal` restarts cleanly.
  **Fix (if wanted):** restart the replicas with the primary, or have
  `restart` of a primary say a replica took over. needs sign-off.
- **CLI-PLAN-REPLICA-SENDERS-UNBOUND** (LOW, config) — the plan fixes the
  sender sockets (`RSX_GW_SEND_ADDR`, `RSX_RISK_GW_SEND_ADDR`,
  `RSX_RISK_ME_SEND_ADDRS`, `RSX_ME_RISK_SEND_ADDRS`, `RSX_ME_MD_SEND_ADDRS`,
  `RSX_MARK_SEND_ADDR`) for the primary risk only. A warm replica binds
  ephemeral senders after a promotion, and every peer's receiver still sends
  its NAKs to the primary's addresses, so after a failover a NAK reaches no
  sender and a gap runs to FAULTED and replay. Related:
  GW-RISK-REPLICATION-ADDR-NAMES-ONE-NODE (the gateway and the ME still
  address the primary); the replica env sets in `rsx-cli/src/plan.rs`
  (`risk_replica`) carry none of the sender keys.
  Read from the code, not run. no test — config. **Options:** (a) give each
  replica its own sender binds and point the peers at the lease holder;
  (b) bind the primary's addresses in the replica too, safe only because the
  lock admits one main. **Default if nothing is decided:** unchanged.

## rsx-playground and rsx-maker

- **PG-STRESS-LOAD-LEAVES-CROSSING-ORDERS-UNFILLED** (LOW, flake) — a live
  `rsx-cli stress` load through the playground at 50 orders/s for 5 s on the
  minimal scenario (box load average about 4) reported 250 accepted, 235
  filled, and so `status` failed with "accepted crossing orders did not all
  fill". The maker's top levels hold 1,000,000 raw qty each; the cause is not
  established (book depth moving under the run, or IOC orders priced
  `--cross-bps` past a book the maker was re-quoting). Seen once, 2026-10-27.
  **Fix:** rerun on a quiet box; if it repeats, log the unfilled orders' prices
  against the marketdata BBO. No test — needs a live repro.
- **PG-GATE-SCENARIO-SWITCH-LEAVES-UNTRACKED-DAEMONS** (MED, flake) — in one
  `make gate` (Gate 4, box load average 11 to 18, 920 MB available) the
  scenario switch in `play_control.spec.ts:41` ended with "restart failed: ports
  already in use: 8088, 8180, 9110, ...", every later order spec answered
  "gateway (gw-0) not available", and after the run `cluster ps` was empty
  while a gateway (started 17:05) and five more daemons (17:07) were still
  alive and holding the ports; they had to be signalled by PID. Seen once; the
  server log of the run was wiped by the gate. Inferred, not measured: the state
  files under `tmp/cluster/` were deleted while the daemons ran, so `down` found
  nothing to stop. No pytest fixture removes `tmp/cluster/` now, so a
  recurrence points elsewhere. A second candidate, also unmeasured: `do_down`
  (`rsx-cli/src/cluster.rs:846`) removes each record by name after the group
  stop, and `_action_restart` (`rsx-playground/server.py:3683`) runs outside
  `bulk_operation`, so a restart landing in between has its new record deleted
  and its daemon left untracked. 18 Gate 4 failures followed (420 of 448
  passed). **Fix:** on a second occurrence keep the server log and
  `tmp/cluster/` and find who removes the pid files. No test — needs a live
  repro.
- **PG-SAFETY-GATEWAY-CRASH-SOCKET-HANGUP** (LOW, flake) — one `make gate`
  failed the "gateway crash shows error state in topology" test in
  `play_safety.spec.ts` with `apiRequestContext.post: socket hang up` on `POST
  /api/processes/gw-0/start` in `afterEach`, which skipped the 7 serial tests
  after it; the same 8 tests passed alone. The server log of the run was wiped
  by the gate. Inferred, not measured: the request reused a keep-alive
  connection idle for about 5 s, uvicorn's default `timeout_keep_alive`.
  **Fix:** confirm on a second occurrence (the server log must be kept), then
  raise the server's keep-alive above the spec's longest idle wait. No test —
  needs a live repro.
- **MAKER-ORPHANED-QUOTES-ON-CANCEL-MISS** (MED, correctness) — the demo
  maker's cancel-replace fails repeatedly (`log/me-pengu.log`: `cancel: order
  not found user=99 id=...`, the ME's cancel-miss warn in `process_cancel`,
  `rsx-matching/src/cancel.rs`), leaving orphaned quotes that accumulate
  (`quoteCycle` in `rsx-maker/maker.go` cancels the cids it holds in `active`).
  The earlier reading that the ME book locks or crosses is refuted: the
  observed `/api/book/10` bid == ask comes from the playground's marketdata
  cache (the `/api/book/{symbol_id}` route in `rsx-playground/server.py`), and
  the ME cannot rest a crossing order (`process_new_order` matches before
  `insert_resting`, `rsx-book/src/matching.rs`). The gateway-side miss is a
  different mechanism (error 1005 from the pending lookup, not this log line).
  **Fix:** the maker must reconcile its resting set from acks before
  re-quoting; the ME cancel-miss rate is the tell (today a warn with no
  counter; the ME's silence on a cancel miss is ruled, ME-CANCEL-MISS-SILENT).
  Re-observe after the sweep: a lost risk to gateway terminal could leave a
  stale pending entry that a later cancel resolves to an order the ME already
  ended (inference, not measured). No test — needs the live maker.
- **DEMO-MAKER-TOUCH-LOCK** (LOW, maker) — with `RSX_MAKER_JITTER_BPS` on
  (`rsx-maker/config.go`), the maker's jittered cancel-replace leaves same-price
  orders on both sides, so the top of book locks (bid == ask, 0 bps in the GIF)
  instead of showing the 20 bps spread. Cosmetic. deferred. **Fix:** ensure the
  prior ladder fully cancels before the new one rests, or widen the spread
  relative to the jitter. No test — cosmetic.

## Docs, benchmarks and repo hygiene

- **DOC-AI-DEVELOPMENT-METHOD-NOT-EXPLAINED** (MED, traceability) — the repo
  labels the blog AI-generated (`BLOG.md:3`) and `NOTICE` ends with "Development
  assisted by Claude Code from Anthropic.", but no repository-level page
  explains how LLMs were used to develop RSX. A hiring reviewer cannot
  distinguish model-generated drafts from human-selected architecture,
  implementation, adversarial review, measurement and final verification.
  owner decision (public copy). **Options:** (a) add a docs page "How this was
  built with LLMs" with a 3-4 line README disclosure linking to it: for each workflow (specification, implementation, test generation, bug
  hunting, documentation) one concrete input, output and human-review example,
  the model or tool, what it could change, the resulting commit, spec or test,
  and the acceptance command that caught errors, including failures such as the
  zero-acceptance false-green stress result; (b) leave `NOTICE` as the only
  disclosure. **Default if nothing is decided:** (b). No test — traceability.
- **BENCH-CROSSPROCESS-TAIL-LOAD-PROOF-MISSING** (LOW, perf) — roadmap, not a
  defect; `make perf-load` (`rsx-cli stress --mode load`, `docs/benches.md`
  § Route programs) exists, but no dated report in `reports/` records a
  sustained run of the real GW to Risk to ME to Risk to GW path with accepted
  rate, p99, p99.9, max, loss, invariant results and queue saturation. The
  cross-process figures on file are `reports/20260708_bench-rerun.md` (about
  1.1 ms p50, no p99, lines 49 and 59) and `reports/20260530_e2e-ws-probe.md`
  (11,618 us p50, 51,860 us p99, N=842, closed loop). **Fix:** run `make
  perf-load` on a quiet box with the cluster otherwise stopped, three times, and
  write a dated report with commit, command, topology, sample count,
  offered/accepted rates, p50/p99/p99.9/max, CPU and failures; until then the
  README must not claim cross-process parity. No test — needs a quiet box.
- **BENCH-HARDWARE-CONTROLS-NOT-DEMONSTRATED** (LOW, perf) — roadmap, not a
  defect; the strongest load report uses a shared 6-core box with no CPU
  isolation (`reports/20260530_load-curves.md`, § Method), and the July rerun
  calls its tails noisy on a shared host (`reports/20260708_bench-rerun.md`,
  § Caveats). `deploy/README.md` mentions `isolcpus` and pinning, but no
  reproduced result proves real NIC, IRQ and queue placement, NUMA locality,
  frequency and C-state policy, PTP or clock source, or before/after kernel
  tuning. **Fix:** produce one bare-metal report capturing `lscpu -e`,
  `/proc/cmdline`, governor, turbo and C-state state, `numactl --hardware`,
  `ethtool -i/-l/-x/-S`, `/proc/interrupts`, thread-to-core mapping, clock
  source, kernel and compiler versions, and three before/after production-path
  runs; every published hardware claim then links to it, and DPDK, AF_XDP, FPGA
  and SmartNIC stay "not implemented". No test — needs bare metal.
- **REPO-ROOT-UNTRACKED-FILES** (LOW, ops) — the repo root holds untracked files
  that are not project sources: `HEAD`, `config`, `hooks`, `objects` (the
  contents of a bare git directory; `refs/` is gitignored), `tmp_big.txt`,
  `scratch_offsets.txt`, `rsx-landing.png`, the home-directory dotfiles
  (`.bash_profile`, `.bashrc`, `.profile`, `.zprofile`, `.zshrc`, `.gitconfig`,
  `.gitmodules`, `.ripgreprc`, `.mcp.json`, `.codex`, `.idea`, `.vscode`,
  `.mkdocs-venv/`) and `rsx-cast/notes/loss_degradation.rs.wip`. The untracked
  file `HEAD` makes a bare `git ... HEAD` ambiguous. owner decision.
  **Options:** (a) delete or move out what is not RSX, and `.gitignore` the
  rest; (b) leave them. **Default if nothing is decided:** (b); every git
  command uses a full SHA or `HEAD --`. No test — ops.

## Rulings — not defects, do not re-report

- **ME-WIRE-ENUM-LENIENT-DECODE** (ruled, wire) — the ME maps an unknown `tif`
  to GTC and any `side` other than 0 to Sell (`decode_side` and `decode_tif`,
  `rsx-matching/src/wire.rs`, shared by the live path and replay) instead of
  failing the order. Not reachable from client input and not the ME's concern:
  the Gateway rejects `side > 1` and `tif > 2` at parse
  (`rsx-messages/src/frames.rs:178`, `:190`, `:328`),
  `specs/2/47-validation-edge-cases.md` § 1.8 assigns the check to the Gateway,
  and the repo's trust-boundary rule makes the Gateway the one owner of wire
  parsing. The decode is total (no panic, no UB), so the § 1.4 "May panic or UB"
  row does not apply to the ME. The § 1.4 snippet that returns `INVALID_TIF`
  from the matcher (`47-validation-edge-cases.md:133`) is the stale part, a doc
  fix.
- **GW-U-REASON-SLOT-BY-STATUS** (ruled, spec 18) — the `U` frame's reason slot
  (index 4) is one byte with two vocabularies, chosen by the status in index 1:
  status 3 carries a `FailureReason` (18 "FailureReason constants"), status 2
  carries the `CANCEL_REASON_*` code the ME wrote on `OrderDoneRecord.cancel_reason`
  (18 "CancelReason constants"; `route_order_done`), every other status carries
  0. Spec 49 § Gateway to client frames and spec 18 document the slot.
  Consumers read the slot only under status 2 or 3 (`rsx-core/wire/order.go`,
  `rsx-playground/server.py`).
- **GW-REJECTS-PRICE-ZERO** (ruled, validation) — the gateway answers a price 0
  order with `E[1008,"price not tick aligned"]` (`rsx-gateway/src/convert.rs`,
  `admit_order` in `handler.rs`): 47 §§ 1.1-1.2 reject px <= 0 and no frame has
  a market type. The playground sends a marketable limit for "market".
- **GW-SINGLE-SHARD-NO-ROUTING** (ruled, design) — the gateway has one risk
  sender and no `user_id % shard_count` routing: `specs/2/28-risk.md:466` states
  v1 runs single-shard (`shard_count = 1`). A demo limit.
- **GW-MULTI-GATEWAY-ROUTING** (ruled, not built) — Risk answers one
  `RSX_GW_CAST_ADDR`, so a second gateway on the same Risk gets no responses;
  several gateways per Risk need a sender per gateway in Risk and a user to
  gateway map. `specs/2/20-network.md` § Scaling Strategy and
  `specs/2/9-deploy.md` state it, and every scenario starts one gateway
  (`specs/2/30-scenarios.md`).
- **RISK-FUNDING-CROSS-SHARD** (ruled, design) — a global funding zero-sum is
  not guaranteed across risk shards; the demo is single-shard.
- **ME-REPLAY-SKIPS-DOWNSTREAM** (ruled, design) — a matching-engine replay
  does not re-drive downstream consumers; each consumer recovers independently
  through its own replay.
- **BOOK-FAR-PRICE-BUCKETING** (ruled, design) — compression buckets far prices
  (10/100/1000 ticks per slot, `rsx-book/src/compression.rs`), so distinct
  prices share a level: an intentional compressed-book trade. Matching keeps
  price priority inside a slot (`rsx-book/notes/compressed-slot-priority.md`),
  so only the grouping into slots is coarse.
- **CLI-MAIN-HOLDS-THE-WHOLE-CLAP-TREE** (ruled, structure) — `rsx-cli/src/main.rs`
  (1206 lines) is the clap derive tree plus eleven `run_<group>` functions that
  only map flags to the library's input types; none carries a rule, check or
  conversion beyond choosing `PriceInput` or `QtyInput`. `rsx-cli/CLAUDE.md`
  § Keeper "Thin main" states this ("`main.rs` is long because the clap tree is;
  it never gains logic"). Splitting the clap tree from the dispatch into two
  modules deletes nothing, adds module headers and makes every `--help` edit
  touch two files. Ruled by the owner's delegate (sweep decision (d), 2026-10-28): not moved.
- **DOC-UPSTREAM-NOTICE-MISSING** (ruled, traceability) — root `NOTICE` exists
  and records the bundled third party material (htmx 2.0.4, the demo imagery)
  and the `rsx-log` provenance, and `README.md` links it. The arizuko runner
  appears only as text in `rsx-glass/specs/14-contract-support-assistant.md`;
  no `go.mod` or Go file in the tree imports it, so there is no imported code to
  record.
- **ME-CANCEL-MISS-SILENT** (ruled, spec 47) — a cancel that reaches the ME for
  an order that already ended is dropped with a warn and no record
  (`process_cancel`, `rsx-matching/src/cancel.rs`; pinned by
  `cancel_misses_leave_the_book_and_every_stream_untouched`): that order's one
  terminal already went out (invariant 2) and an answer would be a second one.
  The gateway answers `E[1005]` from its pending set.
  `specs/2/47-validation-edge-cases.md` § 3.4.
- **GW-RISK-READINESS-GATE** (ruled, design) — the gateway has no risk-liveness
  signal and no OVERLOADED gate on the risk stream; an order lost to a partition
  is answered TIMEOUT after `RSX_GW_ORDER_TIMEOUT_MS`. `specs/2/20-network.md`
  § Startup Ordering, `specs/2/29-rpc.md` § Network Errors, `GUARANTEES.md`
  § 5.1. An rsx-cast liveness API is a frozen-crate change nobody has asked for.
- **GW-CID-MAP-LOST-ON-RESTART** (ruled, design) — the `(user_id, cid)` map
  (`rsx-gateway/src/client_ids.rs`) is memory-only (`rsx-gateway/CLAUDE.md`
  § Rules), so a gateway crash empties it and a resend mints a fresh order id
  that the ME's `(user_id, order_id_hi, order_id_lo)` dedup accepts as a second
  order; the client reconciles by fills and position (`specs/2/61-cid-idempotency.md`
  § Client requirements, D1 option a). A planned restart drains in-flight orders
  first (`specs/2/9-deploy.md` § Rolling Upgrades). Deriving the order id from
  the cid is rejected in 61 § Rejected.
- **CID-IDEMPOTENCY-PARTS-NOT-BUILT** (ruled, not built) — the cid binding store
  file (D1 option b), the greeting epoch, the order fingerprint and `E[1012]`
  (D4), and the preallocated cap (D3): no in-repo client consumes them or the
  gateway is not a zero-heap loop. `specs/2/61-cid-idempotency.md` § Built and
  not built.
- **GW-STAMP-LATENCY-LIVE** (ruled, spec 59) — the `F` frame and the taker's
  terminal `U` carry the five stamps; the inserted `U`, a resting order's done
  `U` and the failed `U` carry zeros, because their records have no room and a
  hot record is not widened for a diagnostic. Clients show the "internal" leg
  as pending. `specs/2/59-latency-observability.md`.
- **RISK-PG-FILLS-AUDIT-GAP-ON-FAILOVER** (ruled, spec 8) — Postgres `fills` is
  the live main's write-behind: fills applied during a standby's warm catch-up
  get no row (`push_persist` returns without a producer), positions stay
  correct, and the recorder's archive of the ME WAL is the complete fill log.
  No daemon reads the table (only tests do). `specs/2/8-database.md` § Postgres
  `fills` Is the Live Main's Write-Behind.
- **ME-WAL-RETENTION-GC-ON-ROTATE-ONLY** (ruled, ops) — a symbol that stops
  rotating keeps hot segments past 4 h; retention beyond the writer's prune on
  rotate belongs to the recorder archive and an external cleanup
  (`rsx-cast/CLAUDE.md` § Rules: moving segments to an archive or deleting them
  is not the writer's job). A flush-tick prune is a frozen-crate change. Which
  document is true about the writer's prune is CAST-GC-OWNER-DOCS-CONTRADICT.
- **ME-REPLICA-LAG-BACKPRESSURE** (ruled, spec 48) — the producer never stalls
  on a consumer; a lagging replica or consumer catches up over TCP replay.
  `specs/2/48-wal.md` § consumer lag, `rsx-cast/ARCHITECTURE.md` § Transport
  paths ("Slow consumers do not pace the sender"). The replica lag stall rows in
  36 and 42 are marked not built.
- **ME-WAL-OFFLOAD-WORKER** (ruled, spec 48) — the ME has no offload worker; its
  only durable store is its local WAL and `rsx-recorder` archives it over
  replication. `specs/2/48-wal.md` § The Recorder Archives the WAL.
- **DRAIN-REPLAY-DUPLICATED** (ruled, structure) — `drain_replay` has two
  copies of about fifty lines (`rsx-gateway/src/replay.rs`,
  `rsx-marketdata/src/replay.rs`) and a third that differs
  (`rsx-risk/src/replay.rs`: its own apply and tip pre-seed). Each builds a
  tokio runtime, so rsx-cast cannot hold it, and a new shared crate for two
  copies is the premature abstraction `code.md` § Boring code rules out.
- **STRESS-REPLIES-CARRY-NO-CID** (ruled, protocol) — `U`, `F` and `E` replies
  name the server order id, so `rsx-cli stress` binds the first frame naming a
  new oid to the oldest unanswered order of its connection (`Tracker`,
  `rsx-cli/src/stress_tally.rs`); a load mixing rejects with accepted orders
  can mis-attribute individual latencies, the counts stay right, and `load`,
  `probe` and `reject` are unaffected. `U` carries no cid by design
  (`specs/2/61-cid-idempotency.md`).
- **MARKETDATA-SHADOW-BOOK-UNBOUNDED-LEVEL-ALLOC** (ruled, hardening) — a
  `ShadowBook` sizes its level array from the first record's price
  (`ShadowBook::new`, `rsx-marketdata/src/shadow.rs:22`). The crash-loop trigger
  was a torn WAL record decoding to an extreme price, closed by `WalReader`
  returning `InvalidData` for a bad record with data after it
  (`WalReader::next`, `rsx-cast/src/wal.rs`); a CRC-valid extreme price is the
  ME's own output and the sizing contract of `Orderbook::new` is `rsx-book`'s.
- **ME-GW-DIRECT-SPEC-GAPS** (ruled, not built) — the async output split of
  `specs/2/28-risk.md` (the ME sends the fill straight to the gateway) is
  `status: partial` and its status block says it is not implemented: Risk still
  forwards the output leg through `GwStream`. Two audit findings are refuted: a
  reduce-only solvency hole (the ME holds the authoritative per-symbol
  `net_qty` in `rsx-book/src/user.rs` and rejects with
  `OrderFailureReason::ReduceOnly` at match time) and a liquidation race
  (`UserInLiquidation` is checked at Risk pre-flight on the next order).
- **ME-HOLDS-USER-STATE** (ruled, design) — the ME keeps `net_qty`
  (`rsx-book/src/user.rs`) and the `DedupTracker`
  (`rsx-matching/src/dedup.rs`) because both decisions are made at the match
  sequence point: position as of match for reduce-only, already-seen for dedup.
  Neither is an account; balances and margin are Risk's. Moving them to
  `rsx-matching` needs a matching-state snapshot and replay boundary first, or
  reduce-only recovery is incomplete.
- **BOOK-MATCHING-OUTPUT-OWNERSHIP** (ruled, design) — `rsx-book/src/matching.rs`
  owns the cross and rest algorithm and writes events into `Orderbook`, while
  `rsx-matching` owns WAL and transport consumption. A data-structure-only
  `rsx-book` would change snapshot and replay imports, benchmarks and the
  marketdata integration for no behavioural gain; a half-move is worse than
  either side.
- **GW-REST-READ-ENDPOINTS** (ruled, not built) — the gateway serves `/health`
  and `/v1/symbols` only (`rsx-gateway/src/rest.rs`); `specs/2/26-rest.md`
  states the account, positions, orders, fills and funding reads are targets,
  not built.
- **GW-SHARDED-REACTORS** (ruled, not built) — the gateway runs one monoio
  reactor plus the recv thread (`rsx-gateway/src/main.rs`);
  `specs/2/11-gateway.md` § Sharded reactors is marked "proposed, not built".
- **CAST-BIG-GAP-ESCALATION** (ruled, enhancement) — a NAK gap under the 2048
  reorder ring recovers seq by seq (each fill retransmit is a WAL scan); only a
  ring overflow escalates to the TCP replay cold path. The `loss_degradation`
  and `outage_recovery` benches show the overflow path works (a 52 k-record gap
  replays in about 1 s) and the NAK path delivers through about 30% loss;
  escalating on a gap past hot retention is a frozen-crate enhancement, not a
  defect.
- **CAST-SOCKET-COUPLING-BLOCKS-IOURING** (ruled, roadmap) — `CastSender` and
  `CastReceiver` (`rsx-cast/src/cast.rs`) own the `UdpSocket` and couple
  framing with `recv` and `send` (`try_recv_with` does recvfrom and parse in
  one), so an io_uring move in the callers cannot happen there. Two additive
  APIs would lift it; the crate stays on std UDP until the founder signs off.
