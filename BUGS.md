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
  128 (`rsx-cast/src/cast.rs:76`), but `FillRecord`, `BboRecord` and
  `OrderAcceptedRecord` are 128-byte payloads (asserted in
  `rsx-messages/src/lib.rs`), so the frame is 16 + 128 = 144 bytes and
  `send_framed` takes the large-record branch (`cast.rs:212`), which never
  caches the frame. Every NAK for the three most-sent record types misses the
  in-memory ring and reads the disk through `read_record_at_seq`. The comment on
  the constant ("all <= 64 bytes payload") is false. Not a loss (the disk
  fallback works). needs sign-off.
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
- **ME-WAL-RETENTION-GC-ON-ROTATE-ONLY** (LOW, ops) — `prune_old_segments`
  (`rsx-cast/src/wal.rs:303`) runs only inside `WalWriter::rotate`
  (`wal.rs:285`), so a symbol that stops rotating keeps segments past the 4 h
  hot retention (`RETENTION_NS`, `wal.rs:87`). needs sign-off. **Options:**
  (a) an rsx-cast change that prunes on the flush tick; (b) leave it to the
  recorder archive and an external cleanup. **Default if nothing is decided:**
  (b). No test — ops.
- **ME-REPLICA-LAG-BACKPRESSURE** (LOW, design) — `specs/2/48-wal.md:110` and
  `:138` say a replica lag over 100 ms stalls the producer. The ME has no
  consumer-lag signal; it needs feedback from the `ReplicationService` in
  rsx-cast. needs sign-off. **Options:** (a) sign off an rsx-cast lag API;
  (b) strike the clause from 48. **Default if nothing is decided:** (b). No
  test — design.
- **CAST-BIG-GAP-ESCALATION** (LOW, design) — a NAK gap that stays under the
  2048 reorder ring recovers seq by seq (each fill retransmit is a WAL scan);
  only a ring overflow escalates to the TCP replay cold path. A gap far beyond
  WAL retention could escalate to replication instead of grinding. The
  `loss_degradation` and `outage_recovery` benches show the overflow path works
  (a 52 k-record gap replays in about 1 s) and the NAK path delivers through
  about 30% loss; the grind bites only a slowly growing sub-ring gap.
  Enhancement, not a bug. owner decision. **Options:** (a) escalate to replay
  when the gap's oldest seq is past hot retention; (b) leave it. **Default if
  nothing is decided:** (b). No test — design.
- **TIME-NS-HOTPATH-AUDIT** (LOW, perf) — `time_ns()` (`clock_gettime` through
  the vDSO, about 15-30 ns) is called per record in a few places (replication
  server CaughtUp and record stamping, WAL timestamps). Audit the call
  frequency; if one is a per-record hot path, stamp once per batch or use a
  cached clock. Not correctness. deferred — needs a profile first. No test —
  perf.
- **REPLAY-RESCAN-PER-CONNECTION** (MED, perf) — `oldest_and_highest_seq`
  (`rsx-cast/src/wal.rs:631`) full-scans the active WAL file (up to 64 MB) on
  every replay connection to validate the requested range (`handle_client`,
  `replication_server.rs:120`). The live tail also reopens the active file every
  100 ms and reads it from byte 0, discarding every record below the next seq
  to send. Fine for occasional bootstraps; under a
  reconnect storm replay becomes I/O-bound. The `outage_recovery` bench shows
  the per-cycle recovery of a constant 7.5 k-record gap drifting 262 to 786 ms
  across six cycles because the active WAL grows. needs sign-off. **Options:**
  (a) an incremental tail scan (cache highest and the active-file offset);
  (b) a producer-published highest-tip file; (c) leave it. **Default if
  nothing is decided:** (c). No test — perf.
- **CAST-SOCKET-COUPLING-BLOCKS-IOURING** (LOW, design) — roadmap, not a defect;
  `CastSender` and `CastReceiver` (`rsx-cast/src/cast.rs`) own the `UdpSocket`
  and couple framing with `recv` and `send` (`try_recv_with` does
  recvfrom and parse in one; `send_framed` writes on the owned socket), so an
  io_uring move in the callers (matching, gateway, marketdata) cannot happen
  there. needs sign-off. **Options:** (a) two additive APIs, one exposing a
  built `Framed`'s bytes and one parsing already-received bytes; (b) stay on
  std UDP. **Default if nothing is decided:**
  (b). No test — design.

## rsx-matching

- **ME-CANCEL-MISS-SILENT** (MED, correctness) — `process_cancel`
  (`rsx-matching/src/cancel.rs`) logs a warn and returns when `(user,
  order_id)` is not in the order index; nothing reaches the WAL, risk, gateway
  or marketdata (pinned by `cancel_misses_leave_the_book_and_every_stream_untouched`
  in `cancel_test.rs`). `specs/2/47-validation-edge-cases.md` § 3.4 lists
  "REJECT: not found" for an order that never existed or already ended, while
  the gateway answers `E[1005]` from its pending set only for orders it no
  longer holds. `OrderFailureReason` (`rsx-book/src/event.rs`) has no
  not-found value. needs sign-off. **Options:** (a) add a `NotFound` reason
  (wire + book) and emit ORDER_FAILED through the duplicate-reject path, which
  needs a ruling on what Risk (frozen-margin release) and the Gateway
  (`route_order_failed`, `rsx-gateway/src/route.rs`) do with an ORDER_FAILED
  for an order they may already have closed (invariant #2, exactly one
  completion); (b) keep the ME silent and correct 47 § 3.4 to say the gateway
  answers cancel misses from its pending map. **Default if nothing is
  decided:** (b), the current behaviour.
- **ME-CRASH-BEFORE-FLUSH-SEQ-REUSE** (MED, correctness) — records are cast to
  risk and marketdata when appended (`fan_out`, `rsx-matching/src/wal.rs:450`;
  the accepted record in `order.rs`) but reach disk at the next 10 ms flush
  (`flush_if_due`, `wal.rs:500`). A crash in that window loses records both
  consumers already hold; boot sets `next_seq` from the replayed disk tail
  (`Recovered::next_seq`, `recovery.rs`), so new records reuse the lost seqs
  and the consumers' `seq <= tip` gates (`rsx-risk/src/position.rs`,
  `process_fill`) drop them as duplicates. `GUARANTEES.md` (Durability bullet)
  assumes replicas that the ME does not have (ME-REPLICA-TAKEOVER). needs
  sign-off. **Options:** (a) cast only after the covering flush, which adds up
  to 10 ms to every fill and contradicts the <50 us target; (b) accept the
  single-node window and state it in GUARANTEES until the replica exists.
  **Default if nothing is decided:** (b). Measured: `POST
  /api/matching/crash/me-pengu` under maker load, 2026-10-21: `wal_last_seq`
  15650 before the SIGKILL, 15629 after recovery; the Matching tab's recovery
  card shows the tip going back.
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
  no lease (rsx-risk has `lease.rs` and `promotion.rs`). needs sign-off.
  **Options:** (a) build the replica on the rsx-risk lease shape; (b) mark the
  sections "not implemented" in 21, 6 and 41. **Default if nothing is
  decided:** (b). No test — design.
- **ME-WAL-OFFLOAD-WORKER** (LOW, design) — `specs/2/48-wal.md:101` ("Offload
  Worker (Durable Commit)") describes a worker in the ME; none exists, the
  recorder archives instead. owner decision. **Options:** (a) rewrite the 48
  section around the recorder; (b) build the worker. **Default if nothing is
  decided:** (a). No test — design.
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
- **ME-GW-DIRECT-SPEC-GAPS** (HIGH, design) — the async output split in
  `specs/2/28-risk.md` (ME sends the fill straight to the gateway; settle to
  Risk async) is `status: partial` (`28-risk.md:2`, status block at
  `:199-205`) and an adversarial audit found it underspecified. needs
  sign-off; do not hand to a coding agent before the spec is rewritten. Gaps
  to close before any implementation:
  1. No authoritative client sequence once ME (per-symbol seq) and Risk
     (risk-gw seq) both feed the gateway; today Risk restamps one contiguous
     stream (`forward_to_gw`, `rsx-risk/src/rings.rs`).
  2. Reject-edge contradiction: the spec says the Risk to GW
     insufficient-margin reject "cannot be removed" (`28-risk.md:173`) and
     also says Risk drops `forward_to_gw` and the response ring
     (`28-risk.md:203-206`); `forward_to_gw` carries both fills and rejects
     (`rsx-risk/src/cast_loop.rs`). Drop it and a client cannot tell a
     rejection from transport loss.
  3. Gateway replay for the ME to GW stream is undefined: no
     replication address for that stream, no per-symbol tip, replay source or
     dedup;
     today the gateway replays only Risk (`handle_replay`,
     `rsx-gateway/src/cast_recv.rs`; `drain_replay`,
     `rsx-gateway/src/replay.rs`).
  4. ME live-stream contents are under-specified, which gives false FAULTED
     gaps: if the gateway gets only fill, DONE and FAILED, the WAL-seq holes at
     ORDER_ACCEPTED and BBO look like loss.
  5. Duplicate fills: no dedup key (a fill id or `(symbol, seq)`); during a
     mixed rollout the client double-counts (`route_fill`,
     `rsx-gateway/src/route.rs`, pushes blind).
  6. Exactly-one-completion: a direct ORDER_DONE clears pending and a late
     Risk ORDER_FAILED emits a second terminal (`route_order_failed`).
  Settled: the acceptance-commits invariant (once Risk pre-flight accepts, the
  fill is guaranteed; Risk only processes fills and never vetoes) dissolves
  gaps 1, 2 and 6, since per order exactly one terminal producer exists (Risk
  iff pre-flight rejected, else ME). The remaining work is mechanical
  transport: the gateway as a full ME-replication consumer (3, 4) and
  fill-id dedup (5). Two audit findings are refuted: a reduce-only solvency
  hole (the ME holds the authoritative per-symbol `net_qty` in
  `rsx-book/src/user.rs` and clamps or rejects reduce-only synchronously at
  match time, so a second reduce-only hits `net_qty == 0` and fails
  `FAIL_REDUCE_ONLY`; Risk's `Ok(0)` is correct, `28-risk.md` and
  `47-validation-edge-cases.md` assign enforcement to the ME) and a
  liquidation race (`UserInLiquidation` is checked synchronously at Risk
  pre-flight on the next order). **Options:** (a) rewrite the spec around the
  invariant and the transport story, then build; (b) mark the section "not
  implemented". **Default if nothing is decided:** (b). See ME-HOLDS-USER-STATE.
- **ME-HOLDS-USER-STATE** (MED, design) — the ME stores two pieces of per-user
  state in `rsx-book`: `net_qty` (per-symbol position, for the reduce-only
  clamp, `user.rs`) and the `DedupTracker` (`(user_id, order_id)` idempotency,
  `dedup.rs`). Neither is an account (no balances, margin or collateral; those
  are Risk's, which is why Risk shards by user: portfolio margin the ME
  structurally cannot compute). The rule: the ME holds exactly the per-user
  state whose decision must be made at the match sequence point (position as
  of match for reduce-only, already-seen for dedup). Cost: per-symbol slab GC
  for idle users (`order_count`, `zero_since_ns`, `RECLAIM_GRACE_NS`). Open
  question: can the edge hold less? The direction is a pre-authorized risk
  buffer at the ME edge (Risk grants a conservative per-symbol margin
  allowance, the ME draws it down synchronously, Risk tops up or revokes
  async), which would also let the forward Risk hop be skipped (GW to ME to
  GW); the hard parts are buffer sizing, cross-symbol partitioning and
  revocation as the mark moves. State is isolated behind `UserRegistry`
  (`rsx-book/src/user.rs`) as a seam. `UserRegistry` is serialized by
  `UserRegistrySnapshotV1` and `restore_users` in `rsx-book/src/snapshot.rs`,
  and WAL replay rebuilds positions by running the same matching path; moving
  it to `rsx-matching` needs an explicit matching-state snapshot and replay
  boundary first, or reduce-only recovery is incomplete. owner decision.
  **Options:** (a) keep it in `rsx-book`; (b) move it with the boundary above.
  **Default if nothing is decided:** (a). No test — design.

## rsx-book

- **BOOK-MATCHING-OUTPUT-OWNERSHIP** (LOW, design) — not a correctness bug;
  `rsx-book/src/matching.rs` owns the cross/rest algorithm and writes events
  into `Orderbook`, while `rsx-matching` owns WAL and transport consumption.
  The intended boundary is a data-structure-only `rsx-book` plus a
  deterministic `rsx-matching` engine that owns matching policy, typed events
  and a reusable caller-provided output buffer. Moving it also changes
  snapshot and replay imports, benchmarks and marketdata integration.
  deferred — do not half-move it. **Options:** (a) move it; (b) keep the
  boundary. **Default if nothing is decided:** (b). No test — design.
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
  only. `docs/concepts/05-slab-and-compression.md` documents the trade.
  needs sign-off. **Options:** (a) a correct incremental migration for live
  matching (migrate the crossing band eagerly and the rest on idle, or consult
  both maps during the window), after a design and adversarial review;
  (b) keep the bounded spike. **Default if nothing is decided:** (b). No test —
  design.
- **BOOK-DEMO-COLD-OPEN-70NS** (LOW, docs) — `rsx-book/demo/bench-live.sh`
  (lines 20 and 33) still prints "~70 ns, flat" on the cold-open card, a figure
  from `reports/20260716_book-matching-publication.md`; the evidence source is
  `reports/20260727_book-deep-match-tail.md` (p50 50.719 to 53.688 ns, p99
  82.969 to 111.922 ns). The tracked `book-live-opt.gif` and the
  `assets/book-versus.gif` intro carry the old text and the renamed lane label
  until they are re-recorded on a quiet box. **Fix:** change the card to the
  report's p50 and p99, then `make rec && make gif` and rebuild the intro. No
  test — docs.

## rsx-risk

Each of the first four needs a design beyond what its spec states, so it is
recorded as a proposal rather than built (`specs/2/28-risk.md`,
`13-liquidator.md`, `42-testing-risk.md`, `38-testing-liquidator.md`).

- **RISK-PG-FILLS-AUDIT-GAP-ON-FAILOVER** (LOW, ops) — fills that were matched
  (in the ME WAL) but never processed by a live-promoted risk main get no PG
  `fills` row. Warm catchup applies them through `apply_record` →
  `process_fill` while the shard has no persist producer (it is set only after
  promotion, `rsx-risk/src/app.rs:307`), so `push_persist`
  (`rsx-risk/src/persist.rs:532`) drops the `PersistEvent::Fill` that
  `process_fill` builds (`rsx-risk/src/position.rs:206`). After a failover the
  PG fills table permanently misses the crash-window fills (positions stay
  correct, pinned by `failover_handoff_no_lost_fill_no_double_fill`); the ME WAL
  archived by rsx-recorder stays the authoritative fill log, so this is an
  ops/reporting gap, not a state-correctness one. owner decision.
  **Options:** (a) emit the fill rows from the warm-catchup path for in-shard
  fills above the persisted tip; (b) document PG `fills` as live-window-only.
  **Default if nothing is decided:** (b). No test — ops.

## rsx-gateway

- **GW-SHUTDOWN-NO-DRAIN-DOUBLES-RETRIES** (MED, correctness) — shutdown exits
  without draining `pending`; clients resend into a fresh gateway with an empty
  cid map. Spec 61 (drain + store epoch). needs sign-off. **Options:** (a) build
  spec 61; (b) leave it. **Default if nothing is decided:** (b).
- **GW-RISK-READINESS-GATE** (MED, correctness) — 20 § Startup Ordering says
  reject with OVERLOADED until the risk stream is up and CaughtUp; GUARANTEES
  § 5.1 says a partition is detected in 1 s and every pending order fails
  `NETWORK_ERROR` (29 § Network Errors records what the gateway does instead).
  The gateway has no liveness signal: `/ready` reports pending saturation only
  (`rsx-gateway/src/metrics.rs`), and `CastReceiver` consumes risk heartbeats
  internally and exposes only `try_recv_with`, `reset_after_replay` and
  `expected_seq` (`rsx-cast/src/cast.rs`). needs sign-off. **Options:** (a) an
  rsx-cast liveness API (frozen); (b) a gateway-level probe record on the
  existing stream. **Default if nothing is decided:** no gate; `/ready`
  reports pending saturation only.
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
- **GW-MULTI-GATEWAY** (MED, design) — risk sends every response to one
  `RSX_GW_CAST_ADDR` (`rsx-risk/src/config.rs`, `cast_loop.rs`; the plan gives
  risk `addr(GW_CAST, 0)`), but the `stress` and `M3S` scenarios start two
  gateways (`rsx-cli/src/scenario.rs`, `gateways` = 2; `Layout::gateway` in
  `rsx-cli/src/plan.rs` gives each its own cast address); orders on the second
  gateway get no response (inference from the code, not run). 20 § Scaling
  Strategy and 9-deploy describe several gateways. Related ruling:
  GW-SINGLE-SHARD-NO-ROUTING. needs sign-off. **Options:** (a) per-gateway
  senders in risk and a user to gateway map; (b) one gateway per risk, the
  two-gateway scenarios removed and the specs say so. **Default if nothing is
  decided:** unchanged (one gateway peer per risk).
- **GW-REST-READ-ENDPOINTS** (LOW, design) — 26 specifies `/v1/account`,
  `positions`, `orders`, `fills`, `funding` with JWT and 5/s user, 50/s IP
  limits; the gateway serves only `/health` and `/v1/symbols`
  (`rsx-gateway/src/rest.rs`) and no REST limiter, and the playground serves
  only `/v1/symbols` too. 26 states the reads are targets, not built. needs
  sign-off.
  **Options:** (a) the gateway serves them over risk state; (b) they stay
  deferred. **Default if nothing is decided:** (b).
- **GW-SHARDED-REACTORS** (LOW, design) — 11 § Sharded reactors (proposed)
  specifies N pinned reactors with `SO_REUSEPORT`; 11 § Runtime Model states
  the build: one monoio reactor (`rsx-gateway/src/main.rs`,
  `monoio::RuntimeBuilder`) plus the recv thread. needs sign-off.
  **Options:** (a) build the shard model; (b) delete the proposed section from
  11. **Default if nothing is decided:** (b).
- **GW-SHUTDOWN-NO-DRAIN** (LOW, ops) — on the shutdown signal the gateway does
  one cast-sender `tick`, joins the recv thread and exits (`rsx-gateway/src/main.rs`,
  the `shutdown.load` branch): no close frames, and `gauges.live` is never
  cleared (`main.rs:191` is the only store). 9-deploy § Rolling Upgrades (a
  `[STUB]`) wants stop accepting, drain, close. needs sign-off. **Options:**
  (a) stop accepting, flip `/ready`, send close frames, then exit; (b) leave
  it. **Default if nothing is decided:** (b).
- **GW-STAMP-LATENCY-LIVE** (LOW, feature) — spec 59: `F` and the done `U`
  carry all five stamps (`route_fill`, `route_order_done`); the inserted,
  cancelled and failed `U` send zeros (`update`, `rsx-gateway/src/route.rs`).
  `OrderInsertedRecord` and `OrderCancelledRecord` are full 64-byte records
  with no spare field (`rsx-messages/src/lib.rs`); `OrderFailedRecord` has 23
  spare bytes, not enough for five u64 stamps. Clients show the "internal" leg
  as pending (BK-01, `rsx-glass/specs/04-screen-book.md`). needs sign-off.
  **Options:** (a) widen those records (wire change); (b) stamp only what fits
  and say so in 59. **Default if nothing is decided:** unchanged (zeros).
- **GW-CID-MAP-LOST-ON-RESTART** (MED, design) — the `(user_id, cid)` to order
  id map (`rsx-gateway/src/client_ids.rs`) is in memory, so a gateway restart
  empties it. The ME does not make up for it: its dedup key is `(user_id,
  order_id_hi, order_id_lo)` (`rsx-matching/src/dedup.rs`) and the retry after a
  restart mints a fresh order id, so the ME accepts it as a second order. A
  client retries exactly when the gateway died under it, so the window the map
  closes reopens for that case. owner decision. **Options:** (a) accept and
  document: a retry across a gateway restart is a new order and the client
  reconciles by fills and position; (b) persist the bindings (an append-only
  file under `RSX_GW_WAL_DIR`, replayed at boot and bounded by the 1 hour
  window); (c) derive the order id from `(user_id, cid)` so the ME dedup
  catches it, which also reuses an id for a later order with the same cid once
  the ME window ends while the first still rests (cancel by id then names two
  orders). **Default if nothing is decided:** (a). No test — design.
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

## rsx-marketdata

- **WS-CORE-DUPLICATED** (LOW, hardening) — `rsx-gateway/src/ws.rs` and
  `rsx-marketdata/src/ws.rs` each hold `WS_MAGIC`, the accept loop, the
  accept-key hash, the frame writers and the frame reader guards (mask required,
  FIN=1, 4 KiB cap). The crate graph has no shared home for monoio-based WS code
  (`rsx-types` has no runtime, `rsx-cast` is frozen), so only
  `WS_UPGRADE_DEADLINE` is shared. marketdata's `ws_handshake` still does one
  4096-byte `read` and treats it as the whole request (a request split across
  segments is refused for a missing key; over 8 KiB is not answered `431`),
  where the gateway has `read_http_request`. needs sign-off: a new workspace
  crate. **Options:** (a) hoist the domain-agnostic core into one crate both
  depend on and give marketdata the gateway's `read_http_request`; (b) keep two
  copies. **Default if nothing is decided:** (b). No test — duplication.
- **MD-ACCEPT-DROPS-PEER** (LOW, hardening) — `ws_accept_loop`
  (`rsx-marketdata/src/ws.rs:18`) types its handler `Fn(TcpStream)` and
  discards `peer`; the public endpoint has no per-IP or connection cap and the
  handler cannot see the source IP. deferred — the fix adds a `SocketAddr`
  parameter nothing uses until a cap exists. **Fix:** keep the signature
  symmetric with the gateway (`Fn(TcpStream, SocketAddr)`) when a cap is
  built. No test — hardening.
- **MARKETDATA-SHADOW-BOOK-UNBOUNDED-LEVEL-ALLOC** (LOW, hardening) — a
  `ShadowBook` sizes its level array from `mid_price` and `tick_size` through
  the compression map (`ShadowBook::new`, `rsx-marketdata/src/shadow.rs:22`;
  `ensure_book` takes the hint from the first record, `state.rs:157`), and
  nothing bounds the hint. Once a poisoned replay record sized the array at
  1,998,432,706 slots, and the 47,962,384,944-byte allocation (24 B per
  `PriceLevel` then) aborted marketdata in a crash loop. The trigger was a torn WAL record decoding to an
  extreme price; `WalReader` now returns `InvalidData` for a bad record that
  has data after it (`rsx-cast/src/wal.rs`, `WalReader::next`), which closes
  that path. deferred — a CRC-valid extreme price from the ME stream is the
  only remaining trigger, and a bound here would repeat the sizing contract of
  `Orderbook::new` in `rsx-book`. **Fix (if wanted):** reject or clamp a hint
  whose `total_slots` exceeds a cap, in `Orderbook::new`. No test — needs
  sign-off on the cap.
- **DRAIN-REPLAY-DUPLICATED** (LOW, duplication) — `drain_replay` exists in three
  crates: `rsx-gateway/src/replay.rs:23`, `rsx-marketdata/src/replay.rs:19`
  (the gateway's body apart from the log label) and `rsx-risk/src/replay.rs:54`
  (a different apply and tip pre-seed). Each builds a tokio runtime, so its
  natural home (rsx-cast) cannot take it: rsx-cast is frozen and never gets a
  runtime dependency. needs sign-off on a new shared crate. **Options:**
  (a) extract it; (b) keep three copies. **Default if nothing is decided:**
  (b). CONFIRMED at HEAD 2026-10-28. No test — duplication.

## rsx-cli

- **CLI-PLAN-REPLICA-SENDERS-UNBOUND** (LOW, config) — the plan fixes the
  sender sockets (`RSX_GW_SEND_ADDR`, `RSX_RISK_GW_SEND_ADDR`,
  `RSX_RISK_ME_SEND_ADDRS`, `RSX_ME_RISK_SEND_ADDRS`, `RSX_ME_MD_SEND_ADDRS`,
  `RSX_MARK_SEND_ADDR`) for the primary risk only. A warm replica binds
  ephemeral senders after a promotion, and every peer's receiver still sends
  its NAKs to the primary's addresses, so after a failover a NAK reaches no
  sender and a gap runs to FAULTED and replay. Related: the replica
  failover entries above (the gateway and the ME still address the primary).
  Read from the code, not run. no test — config. **Options:** (a) give each
  replica its own sender binds and point the peers at the lease holder;
  (b) bind the primary's addresses in the replica too, safe only because the
  lock admits one main. **Default if nothing is decided:** unchanged.
- **CLI-FOLLOW-SPINS-ON-TWO-DATED-FILES** (LOW, correctness) — `Tail`
  (`rsx-cli/src/tail.rs`, behind `wal dump --follow`) decides a segment still
  has data with `file.is_active || file.last_seq >= next_seq`; a dated archive
  file has `last_seq == u64::MAX`, so a drained dated file passes again.
  With two dated files `Tail::next_record` leaves the first, reopens it, and
  loops without returning: the follow prints the first file and never reaches
  the second (reproduced: a `Tail` over `1_2026-10-26.wal` and
  `1_2026-10-27.wal` did not return in 100 s). **Fix:** remember the newest
  dated file already left and skip dated files up to it in `has_data_from`.
- **STRESS-REPLIES-CARRY-NO-CID** (LOW, protocol) — `N` replies (`U`, `F`, `E`)
  name the server order id, not the client order id, so `rsx-cli stress` binds
  the first frame naming a new oid to the oldest unanswered order of its
  connection (`stress_tally.rs`, `Tracker`). An `E` from the gateway's own
  validation can overtake orders still inside risk or matching, and then takes
  the wrong order's send time; a load mixing rejects with accepted orders can
  mis-attribute individual latencies (the counts stay right). Load and probe
  orders are all valid, and `reject` is closed-loop, so neither mode is
  affected. CONFIRMED by reading the gateway frames. needs sign-off (wire
  change). **Options:** (a) echo the cid in every reply; (b) leave it.
  **Default if nothing is decided:** (b).

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
  (`rsx-cli/src/cluster.rs:842`) removes each record by name after the group
  stop, and `_action_restart` (`rsx-playground/server.py:3674`) runs outside
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
  counter: ME-CANCEL-MISS-SILENT). No test — needs the live maker.
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
  (`rsx-messages/src/frames.rs:168`, `:180`, `:307`),
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
  sender and no `user_id % shard_count` routing: `specs/2/28-risk.md:459` states
  v1 runs single-shard (`shard_count = 1`). A demo limit; the related open half
  is GW-MULTI-GATEWAY.
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
