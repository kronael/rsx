# Bug queue

The review queue: **OPEN** and **DEFERRED** items only. Resolved bugs live
in git (commit refs below) and `CHANGELOG.md` — not here.

## Status — 2026-10-20 — nightly-2026-10-20 toolchain bump

- **CAST-UNUSED-DEPENDENCIES** (LOW, config) — cargo's `unused_dependencies`
  lint flags `webpki-roots`, `toml` and `chrono` in `rsx-cast/Cargo.toml`
  (lines 17, 22, 23); no `.rs` file in the crate references them, so every
  `cargo check` prints three warnings. The crate is frozen
  (`rsx-cast/CLAUDE.md`), so the lines stay until the owner signs off.
  **Fix:** delete the three dependency lines. CONFIRMED at HEAD 2026-10-20.

## rsx-matching

Test pins for the entries below are written with each fix, not before:
`.ship/53-COMPONENT-COMPLETION/matching.md` § 5 names the step and test file
per entry.

- **ME-HEARTBEAT-WARN-FLOOD** (LOW, ops) — the loop warns on every failed
  heartbeat send (`rsx-matching/src/cast_loop.rs:203-208`). `CastSender::tick`
  sets `last_heartbeat` only after a successful send
  (`rsx-cast/src/cast.rs:243-254`), so with a peer whose sends fail the
  heartbeat is due again on the next spin and the pinned loop logs at spin
  rate, the flood `CastPeer` removed for record sends. Read from the code,
  not reproduced; no test — ops. **Fix:** the same per-peer episode flag
  (`CastPeer::tick` warns on the first failure, `info` on recovery).
- **ME-WAL-FLUSH-RETRY-REWRITES-BUFFER** (MED, correctness, rsx-cast frozen) —
  `WalWriter::flush` (`rsx-cast/src/wal.rs:210-223`) clears its buffer only
  after `write_all` and `sync_all` both succeed. The ME retries a failed
  flush until it succeeds (`retry_until_ok`, `rsx-matching/src/wal.rs`, the
  stall of `specs/2/48-wal.md:89`). Read from the code, not reproduced: a
  `write_all` that fails part-way leaves a prefix of the buffer in the
  segment, and the retry appends the whole buffer again, so a torn record is
  followed by a fresh copy and replay stops at the first CRC failure; a
  `sync_all` failure leaves the full buffer written and the retry writes it a
  second time (duplicate seqs). needs sign-off, since the fix touches
  `WalWriter`. **Options:** (a) rsx-cast bugfix: track the bytes already
  written and retry only the remainder, or truncate to `file_size` before the
  rewrite; (b) treat a flush error as fatal in the ME (the authoritative-WAL
  policy) and drop the retry, which contradicts the spec's stall. **Default
  if nothing is decided:** leave the retry. Test: a writer on a full or
  read-only device.
- **ME-BENCH-NO-P99** (LOW, perf) — narrowed. `rsx-matching/benches/me_tail_bench.rs`
  reports p50 and p99 for the order path and the checkpoint capture pause, its
  rows are in `bench-baseline.json`, and the dated run is
  `reports/20261020_matching-p99.md`. Still missing: the six Criterion benches
  (`matching_bench`, `process_order_bench`, `match_n_levels_bench`,
  `match_depth_bench`, `match_by_type_bench`, `wal_replay_bench`) report no p99
  and have no `bench-baseline.json` entry, and
  `reports/20260716_book-matching-publication.md` (the crate README's evidence
  link) still has no p99. The tail rows' baseline is the largest of three runs
  on a host at 86-95% CPU pressure, so it is lenient; re-save it on a quiet box
  (MATCHING-BENCH-SHARED-HOST-VARIANCE). **Fix (fix-now):** baseline entries for
  the Criterion ME benches, from a quiet box. No test (perf).
- **ME-CAPTURE-PAUSE-SCALES-WITH-SLAB** (MED, perf, proposal) — the checkpoint
  capture that runs on the pinned loop every 10 s (`cast_loop.rs`,
  `MatchingCheckpointV1::capture`, `rsx-book/src/snapshot.rs:93`) paused it for
  17-19 ms p50 at 100,000 resting orders and 209-262 ms p50 at 1,000,000
  (`reports/20261020_matching-p99.md`, noisy shared host, three runs). With
  100,000 live orders on a 1,000,000-slot slab it is 148-157 ms p50: the cost
  follows the slab high-water mark, because `capture` sweeps every slot below it
  and walks the free list, not the live order count. Orders arriving during the
  pause wait in the UDP receive buffer; whether a pause this long trips a Risk or
  ME timeout is not measured. The split of the time (slot sweep, free-list walk,
  snapshot allocation) is not profiled. needs sign-off. **Options:** (a) profile
  first, then bound the work per spin (capture in slices across loop iterations,
  consistent because the loop is single-threaded); (b) accept it and document the
  resting-order count above which the pause exceeds the timeout.
  **Default if nothing is decided:** (b).
- **ME-CANCEL-MISS-SILENT** (MED, correctness) —
  `rsx-matching/src/cancel.rs:35-42`: a cancel whose `(user, order_id)` is not
  in the order index logs a warn and returns; nothing reaches the WAL, risk,
  gateway or marketdata. `specs/2/47-validation-edge-cases.md:407-417` (§ 3.4)
  says REJECT and `specs/2/18-messages.md:495-507` says
  `ORDER_FAILED(ORDER_NOT_FOUND / ALREADY_CANCELLED)`; `OrderFailureReason`
  has no such value (`rsx-book/src/event.rs:17-22`). needs sign-off.
  **Options:** (a) add a `NotFound` reason (wire + book) and emit ORDER_FAILED
  through the duplicate-reject path, which needs a ruling on what Risk
  (frozen-margin release) and the Gateway (`route_order_failed`,
  `route.rs:201-216`) do with an ORDER_FAILED for an order they may already
  have closed (invariant #2, exactly one completion); (b) keep the ME silent
  and correct 47 § 3.4 / 18 to say the gateway answers cancel misses from its
  pending map. **Default if nothing is decided:** (b), the current behaviour,
  pinned by the plan step 8 test.
- **ME-REPLAY-IGNORES-CONFIG-CHANGES** (MED, correctness) —
  `replay_wal_after_snapshot` (`wal.rs:452-526`) handles only ORDER_ACCEPTED
  and ORDER_CANCELLED (`wal.rs:480-511`; the side-effect arm at `wal.rs:512`
  skips CONFIG_APPLIED) and
  re-executes ORDER_ACCEPTED through `process_new_order` under the book's
  tick/lot as restored from the checkpoint. `ConfigAppliedRecord` carries
  version and times only (`rsx-messages/src/lib.rs:251-259`); its 16 spare
  bytes cannot hold tick, lot and both decimals (8+8+1+1). An order accepted
  after a post-snapshot config change that is valid under the new tick/lot but
  not the old can be rejected on replay, so the rebuilt book diverges
  (inference from the code; no repro). With a database configured, boot
  replays under the applied row's tick/lot rather than the checkpoint's
  (`recovery.rs`, `recover`), so an order accepted before a config change can
  also be rejected on replay under the newer tick/lot. needs sign-off. **Options:** (a) put
  tick/lot/decimals on the CONFIG_APPLIED record and apply it in seq order
  during replay (wire change in rsx-messages); (b) write a checkpoint on every
  config apply so replay never crosses a config change. **Default if nothing
  is decided:** status quo; the 10 s checkpoint cadence (`cast_loop.rs:191`)
  bounds the exposure window.
- **ME-APPLIED-WRITE-FAILURE-REVERTS-ON-RESTART** (MED, correctness) —
  `apply_due_configs` updates the book, queues the applied config for the
  config sidecar and emits CONFIG_APPLIED; the sidecar's upsert runs up to
  1 s later (`APPLIED_DRAIN_TICK`) and only warns when `write_applied_config`
  fails (`config.rs:354`). A full applied ring drops the write the same way
  (`apply_due_configs` warns). A crash inside that second leaves the same
  state as a failed upsert. The
  `symbol_config_applied` row stays at the older version while the book, the
  next checkpoint and the stream carry the newer one. Boot now installs the
  applied row over the checkpoint's config (`recovery.rs`, `recover`), so a
  restart after such a failure runs the older tick/lot until the next poll
  re-applies the newer version (the first poll comes 600 s after boot, and
  the checkpoint holds no config version to compare). That reverts a config
  that `specs/2/19-metadata.md` says is never reverted. Read from the code,
  not reproduced. **Options:** (a) retry the upsert before emitting
  CONFIG_APPLIED and treat a persistent failure as fatal; (b) store the
  config version in the checkpoint and keep the newer of checkpoint and row.
  **Default if nothing is decided:** leave the warn. Test: a client whose
  upsert fails, then `recover` with a checkpoint that carries the newer tick.
- **ME-BOOT-DB-CONNECT-NO-TIMEOUT** (LOW, ops) — `connect_db`
  (`rsx-matching/src/app.rs:74`) awaits `tokio_postgres::connect` with no
  deadline. A refused connection stops boot at once, but a black-holed host
  blocks `run` before any WAL or socket is opened, with no log line. The URL's
  `connect_timeout` parameter bounds it; `tokio-postgres` applies none by
  default, and the playground URL (`rsx-playground/runtime.py`,
  `DEFAULT_PG_URL`) sets none. **Fix:** set `connect_timeout` on the parsed
  config, or wrap the connect in `tokio::time::timeout` (needs tokio's `time`
  feature in `rsx-matching/Cargo.toml`). Test: connect to a non-routable
  address and assert boot exits within the bound.
- **ME-CRASH-BEFORE-FLUSH-SEQ-REUSE** (MED, correctness) — records are cast
  to risk and marketdata when appended (`fan_out`, `wal.rs:367-374`;
  accepted record `order.rs:76-93`) but reach disk at the next 10 ms flush
  (`flush_if_due`, `wal.rs:415-420`). A crash in that window loses records
  both consumers already hold; boot sets `next_seq` from the replayed disk tail
  (`recovery.rs`, `recover`), so new records reuse the lost seqs and the
  consumers'
  `seq <= tip` gates (`rsx-risk/src/position.rs:149-150`) drop them as
  duplicates. `GUARANTEES.md:13-15` assumes replicas that the ME does not have
  (ME-REPLICA-TAKEOVER). needs sign-off. **Options:** (a) cast only after the
  covering flush, which adds up to 10 ms to every fill and contradicts the
  <50 us target; (b) accept the single-node window and state it in GUARANTEES
  until the replica exists. **Default if nothing is decided:** (b).
  Measured: `POST /api/matching/crash/me-pengu` under maker load, 2026-10-21:
  `wal_last_seq` 15650 before the SIGKILL, 15629 after recovery; the Matching
  tab's recovery card shows the tip going back.
- **ME-CONFIG-STATUS-IGNORED** (LOW, design) — `specs/2/21-orderbook.md:79`
  ("Matching validates tick/lot sizes and status") and
  `specs/2/19-metadata.md:39-41` (`status` active/paused/halted,
  `min_notional`, `max_order_qty`). `poll_scheduled_configs`
  (`config.rs:84-90`) never selects those columns
  (`symbol_config_schedule.status` exists,
  `rsx-risk/migrations/005_symbol_config.sql:27`) and `write_applied_config`
  always writes `'active'` (`config.rs:134`); no `OrderFailureReason` exists for
  a halted symbol. The spec names `status` but not which values halt or what
  the ME emits for an order on a halted symbol. owner decision. **Options:**
  (a) halt rejects new orders with a new reason and still allows cancels;
  (b) `status` is Gateway/Risk-only, strike it from 21 § 2.9. **Default if
  nothing is decided:** (b), the ME ignores it. No test — design.
- **ME-REPLICA-TAKEOVER** (design) — `specs/2/21-orderbook.md:266-271`
  (replica takeover via ReplicationConsumer tip and a Postgres advisory lock),
  `specs/2/6-consistency.md:45` (mirrored stream to a hot-spare ME) and
  `specs/2/41-testing-matching.md:142-148`: the crate has no replica mode and
  no lease (rsx-risk has `lease.rs` and `promotion.rs`). needs sign-off.
  **Options:** (a) build the replica on the rsx-risk lease shape; (b) mark the
  sections "not implemented" in 21, 6 and 41. **Default if nothing is
  decided:** (b). No test — design.
- **ME-REPLICA-LAG-BACKPRESSURE** (design, rsx-cast frozen) —
  `specs/2/48-wal.md:110` and `:138` say a replica lag over 100 ms stalls the
  producer. The ME has no consumer-lag signal; it needs feedback from the
  `ReplicationService` in rsx-cast, which is frozen. needs sign-off.
  **Options:** (a) sign off an rsx-cast lag API; (b) strike the clause from
  48. **Default if nothing is decided:** (b). No test — design.
- **ME-WAL-RETENTION-GC-ON-ROTATE-ONLY** (LOW, ops, rsx-cast frozen) —
  `prune_old_segments` runs only inside `WalWriter::rotate`
  (`rsx-cast/src/wal.rs:274`), so a symbol that stops rotating keeps segments
  past the 4 h hot retention. needs sign-off. **Options:** (a) sign off an
  rsx-cast change that prunes on the flush tick; (b) leave it to the recorder
  archive and an external cleanup. **Default if nothing is decided:** (b).
  No test — ops.
- **ME-WAL-OFFLOAD-WORKER** (design) — `specs/2/48-wal.md:100` ("Offload
  Worker (Durable Commit)") describes a worker in the ME; none exists, the
  recorder archives instead. owner decision. **Options:** (a) rewrite the 48
  section around the recorder; (b) build the worker. **Default if nothing is
  decided:** (a). No test — design.
- **ME-WIRE-ENUM-LENIENT-DECODE** (ruled not a defect) — the ME maps an
  unknown `tif` to GTC and any `side` other than 0 to Sell (`decode_side` and
  `decode_tif`, `rsx-matching/src/wire.rs`, shared by the live path and replay)
  instead of failing the order. Not reachable from client input and not
  the ME's concern: the Gateway rejects `side > 1` and `tif > 2` at parse
  (`rsx-gateway/src/records.rs:178-192`, `:321`),
  `specs/2/47-validation-edge-cases.md:211-221` (§ 1.8) assigns the check to
  the Gateway, and the
  repo's trust-boundary rule makes the Gateway the one owner of wire parsing.
  The decode is total (no panic, no UB), so the § 1.4 "May panic or UB" row
  does not apply to the ME. The § 1.4 snippet that returns `INVALID_TIF` from
  the matcher is the stale part (doc fix, plan step 11).

- **ME-CHECKPOINT-LOAD-FAILURE-FALLS-BACK-TO-FULL-REPLAY** (MED, correctness)
  — `load_checkpoint` (`rsx-matching/src/checkpoint.rs:195-240`) returns
  `None` for every failure of an existing `snapshot.bin`: the open error
  (`.ok()?`, `:197`), a short header, a bad Avro or legacy payload (warn
  "starting empty", `:236`). `recover` (`recovery.rs:110`) reads `None` as
  "no checkpoint": an empty book and a replay from seq 1. Hot retention is
  4 h, so the retained WAL need not reach seq 1, and a book rebuilt from the
  WAL tail alone silently lacks every order that rested before the oldest
  retained record (inference from the code; no repro), while boot succeeds.
  needs sign-off, since the fix changes the `load_checkpoint` signature.
  **Fix:** return `Ok(None)` only when the file is absent and make a present
  but unreadable checkpoint a `RecoveryError`; the operator then removes the
  file on purpose. Test: a garbage `snapshot.bin` makes `recover` return
  `Err`; no file still boots clean.
- **ME-LIVE-LOOP-SKIPS-UNKNOWN-RECORDS-SILENTLY** (LOW, ops) — the receive
  callback (`rsx-matching/src/cast_loop.rs:104-132`) dispatches
  `RECORD_ORDER_REQUEST` and `RECORD_CANCEL_REQUEST` and has no other arm, so
  an unknown record type is dropped without a log, and a request whose payload
  is shorter than `OrderMessage` / `CancelRequest` (`decode_payload` returns
  `None`) is dropped with no completion record. `specs/2/10-replication.md:502`
  says the ME skips unknown types and logs a warning. Boot replay now warns
  once per unknown type; the live path does not. **Fix (fix-now):** warn once
  per unknown type and warn on every undecodable request. Test: a callback
  fed an unknown type and a short request logs and changes no state (needs the
  plan step 8 loop test seam).
- **ME-REPLAY-STOPS-AT-MID-FILE-CRC-FAILURE** (MED, correctness, rsx-cast
  frozen) — `WalReader::next` returns `Ok(None)`, the EOF value, after a warn
  on a partial record (`rsx-cast/src/wal.rs:418`) and on a CRC mismatch
  (`:427`). A torn tail after a crash is the expected case, but corruption in
  the middle of a segment looks the same: `replay_wal_after_snapshot` stops
  there, `recover` returns `Ok`, and `next_seq` is one past the last seq read,
  so the book lacks the orders that follow and live records re-use the seqs of
  intact records still on disk after the damage. Read from the code, not
  reproduced. needs sign-off. **Options:** (a) rsx-cast bugfix: a CRC or
  length failure with more bytes following in the file (or in a later
  segment) is `Err(InvalidData)`, and only a damaged tail of the last segment
  is EOF; (b) in the ME, compare the bytes the reader consumed with the
  segment lengths after replay and fail boot on a gap. **Default if nothing is
  decided:** leave the reader. Test: a segment with a flipped payload byte
  followed by an intact record makes `recover` return `Err`.
- **GW-CANCEL-REASON-DROPPED** (LOW, correctness) — `route_order_cancelled`
  (`rsx-gateway/src/route.rs:155-183`) sends `reason: 0` in every cancelled
  `OrderUpdate` and ignores `OrderCancelledRecord.reason`, so a post-only
  cancel (wire code 4) reaches the client indistinguishable from a user
  cancel. The ME now writes the `CANCEL_REASON_*` codes
  (`specs/2/18-messages.md`), so the field is available. No consumer reads
  the reason today (rsx-term and the playground WAL decoder skip it), so the
  frame field's meaning (cancel-reason code vs failure-reason code, which
  share the `U` frame's last slot) needs an owner decision first.
  **Options:** (a) forward `rec.reason` and document it in
  `specs/2/49-webproto.md`; (b) keep 0 and drop the slot for cancelled
  frames. **Default if nothing is decided:** leave as is. Test: a cancelled
  record with reason 4 yields `OrderUpdate.reason`. Found in step 6 of
  `.ship/53-COMPONENT-COMPLETION/matching.md`.
- **ME-WIRE-RECORD-TAIL-PADDING-UNINITIALISED** (LOW, hardening) — record
  structs built by literal in `rsx-matching/src/wal.rs` (`emit_events`) leave
  their tail padding unwritten (`OrderCancelledRecord`: 8 bytes after `_pad1`;
  `OrderDoneRecord`: 24 bytes after `gw_out_ns`), and `Framed::pack` copies
  those bytes with `as_bytes`. They reach the WAL, the risk and marketdata
  streams and the replication stream as whatever the stack held: a stack
  address was observed in a cancelled record. The CRC covers the garbage, so
  frames verify, but two runs of the same input write different WAL bytes
  (a mirrored-stream byte compare, ME-REPLICA-TAKEOVER, cannot work) and
  pointer values cross process boundaries. Reading padding is also undefined
  behaviour. Affects every producer of an `rsx-messages` record, not only the
  ME. **Fix:** (a) give each `rsx-messages` record an explicit tail `_pad`
  field so no implicit padding remains (wire size unchanged; rsx-cast stays
  frozen); (b) build each record from a zeroed value and assign fields.
  Default if nothing is decided: leave. Measured:
  `rsx-matching/src/cancel_test.rs::known_defect_cancelled_record_tail_padding_is_zero`
  (`#[ignore]`d; fails with `0xAA` bytes after a dirtied stack). CONFIRMED at
  HEAD 2026-10-20.
- **SPEC-FAILURE-REASON-TABLE-DRIFT** (LOW, spec) — `specs/2/49-webproto.md`
  "Failure Reason" lists `8` = POST_ONLY_REJECT, but `rsx_types::FailureReason`,
  `specs/2/29-rpc.md` (NETWORK_ERROR 8) and `specs/2/18-messages.md`
  "FailureReason constants" say `8` = NETWORK_ERROR, and 49 stops at `12`
  while the ME writes `13` = FILL_OR_KILL. A client built from 49 reads a
  network error as a post-only reject. A post-only cross is a cancel
  (`CANCEL_REASON_POST_ONLY_REJECT`), never an ORDER_FAILED reason. **Fix:**
  make 49's table the one in 18, or point 49 at 18.
- **TERM-FAILED-REASON-NOT-DECODED** (LOW, ux) — `rsx-core/wire/order.go`
  (`Rejected`) renders a failed U frame as `failure_reason=<n>`, so rsx-term
  shows "rejected: failure_reason=13" for a fill-or-kill and
  "failure_reason=7" for a reduce-only violation; the playground decodes the
  same byte to text. **Fix:** a spec 18 table in `rsx-core/wire`, tested
  against `rsx_types::FailureReason`.
- **PLAYGROUND-LOCAL-ONLY-BATCH-ORDERS** (LOW, correctness) — `/api/orders/batch`,
  `/random` and `/invalid` append `submitted` rows to the recent-orders list
  without sending anything to the gateway, and Cancel on such a row flips it
  to `cancelled` locally. Real orders from `/api/orders/test` and the Matching
  tab are `resting` and cancel through the gateway. **Fix:** send them through
  `send_order_to_gateway`, or label them as bookkeeping rows.
- **PLAYGROUND-TRADE-SPEC-EMPTY-RESULT** (MED, test) —
  `rsx-playground/tests/play_trade.spec.ts:70` ("logged in: places a resting
  order and cancels it by cid") fails with `#trade-order-result` still empty
  2.5 s after Submit; the same failure with the playground tree at `a02c04dc`
  (before the Matching tab), at load average 4.5. Cause not found: a curl of
  `POST /api/trade/orders` with a signed cookie answers in the same shape as
  `/api/orders/test`. CONFIRMED at HEAD 2026-10-21.


Found while checking `specs/2/28-risk.md`, `13-liquidator.md`,
`42-testing-risk.md` and `38-testing-liquidator.md` against the crate.
Each needs a design beyond what the spec states, so it is recorded as a
proposal rather than built.

- **RISK-SIDECAR-THREADS-INHERIT-HOT-CORE-ON-REENTRY** (MED, latency) —
  `run_node` pins the calling thread with `setup_hot_thread`
  (`rsx-risk/src/app.rs:307`) after spawning the lease thread (`:276`) and
  the persist worker (`:298`), so the first promotion is right. `run` calls
  `run_node` again from the same thread after a demote (`app.rs:107-122`), and
  nothing restores that thread's affinity, so the second promotion spawns both
  sidecars from a pinned thread; a Linux thread inherits its creator's mask,
  and the persist worker's Postgres writes and the lease renewals then
  preempt the busy-spin. Read from the code, not reproduced (a demote needs a
  lost advisory lock); no test — needs Postgres and a lease loss. **Fix:**
  run each `run_node` cycle on a fresh thread, or save the affinity mask
  before the pin and restore it before spawning.
- **MD-MARK-SIDECAR-THREADS-INHERIT-HOT-CORE** (MED, latency) — the same
  class in two more tiles: `rsx-marketdata/src/main.rs:259` pins before the
  health server thread is spawned (`:282`), and `rsx-mark/src/main.rs:64`
  pins before the multi-thread tokio runtime (`:72`) and the health server
  (`:152`), so their threads inherit the single-core mask. Read from the code
  only; the gateway (`rsx-gateway/src/main.rs:233`) and the threads each tile
  spawns in other modules are unchecked. No test — latency, needs a running
  process. **Fix:** as the ME did, pin last; the `/proc/<pid>/task/*/status`
  `Cpus_allowed_list` check in `rsx-matching/tests/me_process_test.rs` is the
  shape.

- **RISK-FUNDING-SETTLES-ONE-INTERVAL-PER-BOOT** (MED, correctness) —
  `RiskShard::last_funding_id` starts at 0 and is never persisted, so
  `maybe_settle_funding` (`rsx-risk/src/funding.rs`) finds a settlement due
  on the first tick of every boot and charges one full interval, then sets
  `last_funding_id` to the current interval. Two restarts inside one 8h
  interval settle it twice; a node down for three intervals settles one.
  28-risk.md §5 "Missed intervals: settle on next startup" and 42 R25 are
  therefore not met, and the idempotency key `(symbol_id, interval_id)` is
  not enforced anywhere. **Fix (proposal, needs sign-off):** persist the
  last settled `interval_id` (a `funding_state` row per shard, or derive it
  at `load_from_postgres` from `max(settlement_ts)` in `funding`), settle
  each missed interval once at boot, and make `insert_funding` unique on
  `(user_id, symbol_id, interval_id)`.
- **RISK-FUNDING-CONFIG-NOT-PER-SYMBOL** (MED, spec gap) —
  `symbol_config_schedule` carries `funding_interval_sec`,
  `funding_rate_min_bps` and `funding_rate_max_bps` per symbol and
  28-risk.md §5 says "Interval: 8h, configurable per symbol", but the crate
  settles every symbol on one global `FundingConfig` (28800 s, ±100 bps) and
  `load_symbol_configs` does not read those columns. **Fix (proposal, needs
  sign-off):** per-symbol `FundingConfig` inside `SymbolRiskConfig`, a
  per-symbol last-settled interval (which also needs the persisted state
  above), and asymmetric min/max clamping in `calculate_rate`.
- **RISK-LIQUIDATION-ENTRY-DOES-NOT-CANCEL-OPEN-ORDERS** (MED, spec gap) —
  13-liquidator.md §4 and §6 (38 L9/L10/L14) require cancelling the user's
  pending non-liquidation orders when liquidation starts and whenever margin
  recovers, releasing their frozen margin. `LiquidationEngine::enqueue`
  cancels nothing: the frozen orders stay on the book and keep margin
  reserved until they fill or the client cancels. Risk has no cancel egress
  and `frozen_orders` does not record the order's `symbol_id`, so it cannot
  route a `CancelRequest` to the right ME. **Fix (proposal, needs
  sign-off):** keep `symbol_id` in the freeze entry (`confirm_freeze`
  already receives it), add a cancel producer to `ShardRings` the live loop
  drains onto the per-ME senders, and emit one cancel per frozen order on
  entry and on recovery.
- **RISK-POST-MAX-ROUNDS-SOCIALIZES-FULL-NOTIONAL** (decision owed) —
  13-liquidator.md §9 says that past `max_rounds` liquidation "continues at
  `max_slip_bps` with no further delay" and only the *remaining loss* is
  socialized; `LiquidationEngine::maybe_process` instead emits
  `SocializedLoss` for the whole position at the first round past
  `max_rounds`, `process_socialized_loss` deducts `qty * mark` from the
  insurance fund, and the position re-enters the cascade after
  `socialized_cooldown_ns` (the founder's 2026-09-03 call on re-triggering).
  The spec and the code disagree on the amount and on the no-delay phase;
  one of them has to move.

## Status — 2026-08-10 — replication and dedup audit

- **CAST-WAL-RESTART-ACTIVE-FIRST-SEQ-1-SILENT-GAP** (HIGH, correctness,
  rsx-cast frozen) — `WalWriter::new` hardcodes `first_seq: 1`
  (`rsx-cast/src/wal.rs:149`) even when reopening an existing active file
  after restart, and `set_next_seq` (`wal.rs:159-164`) never corrects it.
  The next rotation names the segment `{id}_1_{last}.wal` although its
  records start much higher. After retention GC has pruned the real early
  segments, `oldest_and_highest_seq` (`wal.rs:546-572`) then reports
  oldest=1 from that filename, so `ReplicationService::handle_client`'s
  `endpoint_can_serve` check (`rsx-cast/src/replication_server.rs:120-126`)
  ACCEPTS a `from_seq` it cannot serve and streams from the first record
  it actually has — no `REPLICATION_NOT_AVAILABLE`, no archive fallback,
  no FAULTED. The consumer's `tip = max(tip, seq)` jumps the hole
  silently: positions ≠ sum(fills) (invariant #4) with zero alarms. This
  defeats the exact mechanism spec 10-replication.md §10.5 relies on.
  **Fix (proposal, needs sign-off):** persist/derive the true `first_seq`
  when reopening an active file (scan it, as `oldest_and_highest_seq`
  already does for the active file) and use it in `rotate()`; integration
  regression test = GC + producer restart + stale-tip consumer must get
  `REPLICATION_NOT_AVAILABLE`, never a short stream.
- **CAST-REPLICATION-LIVE-TAIL-RESTREAMS-ACTIVE-FILE** (MED-HIGH,
  correctness/perf, rsx-cast frozen) — `WalReader::open_from_seq` never
  seeks within a file to `target_seq` (`rsx-cast/src/wal.rs:372-390`;
  spec 10-replication.md §4 requires "Seek within file ... until seq >=
  target_seq"; the unit test `reader_skips_to_target_seq_within_file`
  (`wal_test.rs:262`) actually asserts the OPPOSITE — all 10 records
  returned). Consequence in the live-tail loop
  (`rsx-cast/src/replication_server.rs:177-206`): every 100ms the server
  reopens at `last_seq+1`, `pick_start_idx` falls through to the active
  file, and the server re-streams the ENTIRE active file from byte 0 —
  unconditionally, forever. Risk/marketdata use bounded `run_once` +
  seq-dedup (state-correct, but the server re-reads up to 64MB per 100ms
  per tailing consumer). The recorder uses `run()` live-tail and archives
  every record with NO seq guard (`rsx-recorder/src/main.rs:66-81,
  263-274`) → ARCHIVE files accumulate duplicates continuously, and
  again on every recorder reconnect. Archive-replay consumers dedup by
  seq so positions survive, but the "infinite retention" tier
  (CRASH-SCENARIOS.md S17) is silently bloated/dirty. Verified by code
  read; not reproduced live (playground stack was down). **Fix
  (proposal, needs sign-off):** skip records with `seq <= last_seq` in
  the live-tail loop (server-side filter restores spec §10.12 semantics
  without touching the reader); recorder-side seq guard would mask, not
  fix.
- **CAST-WAL-ROTATE-SEQ-RANGE-MISLABEL** (MED, correctness, rsx-cast
  frozen) — `flush()` rotates BEFORE writing the buffer when the write
  would cross `max_file_size` (`rsx-cast/src/wal.rs:217-219`), but
  `rotate()` names the closed segment with `self.last_seq`
  (`wal.rs:245-249`), which `append_framed` (:194) has already advanced
  to the highest BUFFERED-not-yet-written seq, and sets the new file's
  `first_seq = next_seq` (:272). Every pre-write rotation therefore
  mislabels one flush batch: those seqs are claimed by the old segment
  (which doesn't contain them) and disclaimed by the new one (which
  does). `pick_file_for_seq` (`wal.rs:640-650`) then scans the wrong
  file and `read_record_at_seq` returns `Ok(None)` for records that ARE
  durable → casting NAK WAL-fallback misses them → unnecessary FAULTED +
  TCP replay escalation (sequential TCP replay is unaffected — it reads
  all files in order). **Fix (proposal, needs sign-off):** name the
  rotated segment from the last FLUSHED seq (track it separately from
  buffered `last_seq`), and set the new file's `first_seq` to the first
  buffered seq.
- **GW-DEDUP-WINDOW-DOCUMENTED-NOT-IMPLEMENTED** (LOW-MED, docs) —
  GUARANTEES.md §7 promises "Gateway orders | order_id (UUIDv7) | 5min |
  Reject" (and §2.3 "Duplicate orders rejected via UUIDv7 dedup (5min
  window)"), but no dedup exists anywhere in `rsx-gateway/src` (grep
  dedup/duplicate: nothing; `find_by_client_order_id` is used only for
  cancel lookup, `handler.rs:420`). Correctness is actually carried by
  the ME's WAL-backed `(user_id, order_id)` dedup, as GUARANTEES §1.0
  itself says. **Fix:** align GUARANTEES §7/§2.3 with reality (dedup
  owner = ME), or decide the gateway window is wanted and spec it —
  don't leave two owners on paper.
- **SPEC-DEDUP-WINDOW-5MIN-VS-1H** (LOW, docs) — owner decision. The code
  window is 1 hour on purpose (`rsx-matching/src/dedup.rs:6`,
  `DEDUP_WINDOW = 3600s`; WAL rebuild `recovery.rs`, `recover`; `421971e5`;
  `rsx-matching/notes/dedup-persistence.md`), but
  `specs/2/10-replication.md:150` and
  `specs/2/47-validation-edge-cases.md:388,586` say 5 minutes.
  **Options:** (a) update the specs to 1 h (the larger window is the safer
  side and the WAL rebuild already covers it); (b) shrink the constant to
  5 min. **Default if nothing is decided:** (a); the spec reconciliation
  (plan step 11) waits on the call. No test — docs.
- **TESTSPEC-REPLICATION-EDGE-CASES-OVERCLAIMED** (MED, docs) —
  specs/2/36-testing-replication.md cites `rsx-cast/tests/wal_test.rs`
  (actual location: `rsx-cast/src/wal_test.rs`) and its "Edge Case
  Tests" section claims coverage of crash mid-rotation, partial records
  at EOF, replay from future seq, interleaved rotation during replay,
  orphaned active files, concurrent readers, tip-file corruption,
  CaughtUp timing during concurrent appends, TCP disconnect/reconnect
  mid-live-tail, and writer flush-lag stall — NONE of which exist among
  the 34 test fns in that file (verified by listing them; nearest
  neighbors test adjacent happy paths, e.g. clean EOF instead of torn
  record). Several `replication_client_test.rs` cases
  (`consumer_dedup_by_seq`, `consumer_tip_never_decreases_on_reordered_seq`,
  the backoff tests) assert locally re-implemented logic on bare
  variables, not `ReplicationConsumer` itself, and
  `cast_v4_test.rs::oldest_missing_run_naks_sequentially` has zero
  assertions. Anyone scoping the integration tier from this spec would
  believe these seams are covered. **Fix:** rewrite §Edge Case Tests to
  list what exists, and mark the rest as planned (the audit catalogue in
  `.ship/49-FAILURE-MODE-AUDIT/` is the gap list).

## Status — 2026-08-10 — risk crash-guarantee test build-out

Found while writing `rsx-risk/tests/crash_guarantees_test.rs` (commit
`71873ae5`). Record-only per triage — the tests pin what the system DOES
guarantee; these are the seams they deliberately do not assert.

- **RISK-TIPS-UPSERT-NO-MONOTONIC-GUARD** (MED, correctness) —
  `upsert_tips` (`rsx-risk/src/persist.rs:205-222`) does
  `ON CONFLICT ... DO UPDATE SET last_seq = $3` unconditionally, so a
  stale flush can REGRESS the persisted tip below an already-committed
  higher seq, violating invariant #5 at the PG layer. Unreachable under
  a single in-order worker, but reachable in the demote window (see next
  entry) when the old main's final flush commits after the new main's
  first flush. A regressed tip makes the next cold start replay from
  too low — safe only as long as replay stays idempotent — or, paired
  with the positions/accounts no-version-guard (already documented by
  `persist_no_version_guard_on_upsert`), overwrites newer money state
  with older. **Fix:** guard the upsert
  (`DO UPDATE SET last_seq = GREATEST(tips.last_seq, EXCLUDED.last_seq)`)
  and version-guard positions/accounts the same way.
- **RISK-DEMOTED-FINAL-FLUSH-AFTER-LEASE-LOSS** (MED-HIGH, correctness) —
  on lease loss `run_main` calls `stop_persist_worker`
  (`rsx-risk/src/main.rs:965-966`), and the worker's shutdown path
  performs one FINAL FLUSH of pending events
  (`rsx-risk/src/persist.rs:398-410`) — after the advisory lock is
  already gone. A promoted new main may have flushed newer state in the
  gap (lease poll interval ~1s of pending events), so the demoted main's
  unguarded upserts (no version guard, no tip guard) can overwrite newer
  positions/accounts/tips with stale values. This is the one window
  where "at most one main" (invariant #10) does not imply "at most one
  writer". **Fix (needs sign-off — lifecycle semantics):** drop pending
  events on demote (replay recovers them), or guard every upsert by
  version/GREATEST so stale writes are no-ops.
- **CAST-NAK-WAL-FALLBACK-TEST-LOAD-FLAKE** (LOW, test, rsx-cast frozen)
  — `rsx-cast/tests/nak_wal_fallback_test.rs:138`
  (`nak_wal_fallback_delivers_evicted_seq`) failed once under a full
  parallel `make test` on this loaded shared box ("WAL-fallback
  retransmit did not deliver seq=1" after 9.9s; the box also runs the
  live RSX cluster + dockerd), then passed 3/3 consecutive isolated runs
  in ~0.1s and passed a clean full `make test` rerun (1010 passed).
  Timing-window UDP retransmit assertion, load-sensitive, not a code
  regression (no rsx-cast change in the tree). **Fix (needs sign-off,
  frozen crate):** widen the retransmit wait window or gate the test's
  deadline on wall-clock load; record-only until then.
- **RISK-PG-FILLS-AUDIT-GAP-ON-FAILOVER** (LOW, ops) — fills that were
  matched (in the ME WAL) but never live-processed by any risk main
  never get a PG `fills` row: `apply_record`'s replay path folds them
  into positions only; `PersistEvent::Fill` is emitted by live
  `process_fill` alone (`rsx-risk/src/shard.rs:314`). After a failover,
  the PG fills audit table permanently misses the crash-window fills
  (positions stay correct — pinned by
  `failover_handoff_no_lost_fill_no_double_fill`). The ME WAL archived
  by rsx-recorder remains the authoritative fill log, so this is an
  ops/reporting gap, not a state-correctness one. **Fix (if wanted):**
  emit `PersistEvent::Fill` from the replay path for in-shard fills
  above the persisted tip, or document PG `fills` as live-window-only.

## Status — 2026-08-03 — playground exchange-frontend build (Chunk 3)

Found while implementing `.ship/48-PLAYGROUND-EXCHANGE-FRONTEND` Chunk 3
(Trade tab). Record-only per triage; not fixed as part of that work.

- **PLAYGROUND-V1-ORDERS-SYMBOL-ID-KEY-MISMATCH** (MED, correctness) —
  `rsx-playground/server.py`'s `/v1/orders` handler reads
  `order.get("symbol_id", 0)`, but every order dict appended to
  `recent_orders` (both `/api/orders/test` and the new
  `/api/trade/orders`) stores the symbol under the key `"symbol"` (a
  string, e.g. `"10"`), never `"symbol_id"`. `/v1/orders` therefore
  always returns `symbolId: 0` for every order regardless of the real
  symbol. Any consumer keying off `symbolId` (e.g. a per-symbol order
  list) silently misattributes every row to symbol 0. **Fix:** read
  `int(order.get("symbol", 0) or 0)` (matching the `str(symbol_id)`
  storage format), or store a real `symbol_id` int alongside `symbol`
  in the order dict and read that.
- **PLAYGROUND-MAKER-STALE-AFTER-DASHBOARD-RESTART** (LOW-MED, ops) —
  restarting only the dashboard server (`./playground restart`, or the
  dashboard process dying/being respawned) does not restart the
  already-running `maker` subprocess. If the exchange fleet itself is
  also cycled (stop/start) while an old `maker` process survives, the
  maker keeps its stale WS connection to the *previous* `gw-0`
  generation; its quote loop then fails silently (`log/maker.log`:
  `write: broken pipe`, `dial tcp 127.0.0.1:8180: connect: connection
  refused`) and never re-seeds the book. Observed during this session:
  repeated `play_readiness.spec.ts` "book: best_bid > 0" failures
  (`no bids in book`) after several dashboard/pytest-triggered process
  cycles; required `./playground reset --yes && start-all` (full wipe)
  to recover, not just a process restart. Related to the already-open
  `PLAYGROUND-DASHBOARD-STALE-PID-RESTART-RACE` entry below (same
  family: restart doesn't reconcile actual running state) but distinct
  root cause (maker's own WS handle goes stale, not a PID-file race).
  **Fix (defer, record only):** `/api/processes/all/start` (or the
  maker's own supervisor) should detect a stale upstream connection
  (broken pipe / repeated dial failures) and restart the maker itself,
  rather than leaving it running-but-non-functional.
- **PLAYGROUND-SERVER-PY-PYRIGHT-110-ERRORS** (LOW-MED, hardening) —
  `pyright rsx-playground/server.py rsx-playground/pages.py` reports
  110 pre-existing errors (verified: same count/lines on the
  pre-Chunk-3 commit `7278bc8d`, none introduced by this work).
  Recurring shapes: `start_mod.SYMBOLS` assigned at runtime
  (`server.py:103`) onto a `ModuleType` pyright can't widen, cascading
  into dozens of "Argument of type `str | UploadFile`"/"Literal['bid_px']
  not assignable to int" errors wherever that dynamically-typed data
  flows (order form parsing, BBO dict access with string keys
  pyright infers as `dict[int, ...]`); a few standalone ones (`server.py:942`
  possibly-`None` `.readline()`, `:7459`/`:7654` `No parameter named
  "ws_close"`, `:7912` iterating a possibly-`None` dict, `pages.py:303`
  `"Never" is not iterable` on the `TABS` loop from a narrowed empty-list
  inference). **Fix (defer, record only):** needs a real typing pass
  (a `SymbolConfig` dataclass instead of a raw dict-of-dicts on a
  module attribute, `Form(...)` fields typed as `str` not `str |
  UploadFile`, BBO snapshots as a typed dict/dataclass) — not a
  one-line fix, and not touched here to stay inside this session's
  scope.
- **PLAYGROUND-TESTS-NO-TSCONFIG** (LOW, hardening) — `rsx-playground/
  tests/` had no `tsconfig.json` and no `typescript`/`@types/node`
  devDependency at all, so `.ts` test files (e.g. this session's new
  `test_helpers.ts`, using `crypto`/`Buffer`) had no real type-checking
  path — only Playwright's untyped esbuild transform at run time.
  Added a minimal `tsconfig.json` + `typescript`/`@types/node`
  devDependencies (`tests/package.json`) as part of this session's own
  files; `tsc --noEmit -p tsconfig.json` is now clean for every file
  this session touched. That same run surfaces 13 pre-existing errors
  in `play_maker.spec.ts` (previously invisible with no tsconfig:
  `TestDetails`/`APIRequestContext` type mismatches around lines 7-113
  and 340, `Property 'post'/'fetch'/'get' does not exist on type
  'never'`) — not this session's file, not fixed here. **Fix (defer,
  record only):** likely a stale Playwright-vs-TS type shape in that
  file's helper signatures; needs its own look.
- **PLAYGROUND-E2E-TEST-STALE-RAW-FIXED-POINT-FORM-DATA** (LOW-MED,
  test) — `rsx-playground/tests/api_e2e_test.py` has 4 pre-existing
  failures verified UNCHANGED on the pre-session baseline (commit
  `7278bc8d`, via a throwaway detached worktree, same "4 failed, 81
  passed"): `test_api_orders_test_with_form_data`,
  `test_order_flow_test_then_recent`,
  `test_test_order_appears_in_recent_orders`,
  `test_v1_orders_returns_list`. All four POST `/api/orders/test`
  with `price=50000, qty=100000` and assert a fill/resting/accepted
  response, but `_build_order_msg` treats those as HUMAN units
  (`price=50000` × `qty=100000` × `10^8` scale each overflows i64
  notional), so the actual response is "notional too large" —
  the test's form values are stale raw-fixed-point-style numbers from
  before the human-units convention was added elsewhere in the form
  (`api_orders_test.py`'s own equivalent tests already use
  `price=0.049` etc., see `git blame` on that file's newer cases).
  This file isn't in `_gate-api`'s tracked list (`Makefile:58`), so
  `make gate`/`make test` never surface it. **Fix (defer, record
  only):** update these 4 fixtures to human-unit values matching the
  rest of the suite's convention.

## Status — 2026-07-27 — rsx-book documentation and architecture review

- **BOOK-COMPRESSED-PRICE-PRIORITY** (CRITICAL, correctness) — zones 1–4
  place several exact prices in one insertion-ordered chain.
  `rsx-book/src/matching.rs:275` walks that chain FIFO, and
  `rsx-book/tests/matching_test.rs:509` explicitly expects the earlier
  worse-priced maker to fill before the later better-priced maker. This is
  not price-before-time priority, despite README/spec limit-book framing.
  The older `BOOK-FAR-PRICE-BUCKETING` entry labels this merely "coarse."
  **Keep this in review; do not add a permanent exact-price hierarchy
  prematurely.** The simplest candidate keeps zone 0 unchanged, drains every
  fully consumed compressed bucket once, and performs exact-price/FIFO
  selection only in the final bucket where the aggressor runs out. If that
  rare scan is measurably expensive, zones 1–3 can stable-bucketize the
  existing intrusive chain by exact tick (`W <= 1,000`) in `O(N + W)`,
  preserving same-price FIFO and leaving the residual chain sorted; zone 4
  recenters before execution. Settle whether fill-event order inside a fully
  cleared bucket is observable, then benchmark the minimal scan before adding
  cached sublevels or another tree.
- **BOOK-POSTONLY-COMPLETION-SPLIT** (HIGH, correctness) —
  `rsx-book/src/matching.rs:96` emits only `OrderCancelled` when post-only
  would cross, while `specs/2/6-consistency.md:194` requires exactly one
  `ORDER_DONE xor ORDER_FAILED`. Explicit user cancel emits
  `OrderCancelled` then `OrderDone`, so the two cancellation paths disagree.
  **Fix:** choose the protocol contract, then synchronize code, spec, WAL,
  Risk margin release, and docs.
- **BOOK-DOC-OCCUPANCY-OLD-INVARIANT** (HIGH, docs) —
  `rsx-book/ARCHITECTURE.md:115` and `notes/occupancy.md:45` describe
  aggregate-empty/head-side occupancy. Live code correctly maintains
  `bid_count` and `ask_count` independently because compressed slots can hold
  both sides. Following the docs would recreate stale occupancy and skipped
  liquidity. **Fix:** document per-side 0↔1 transitions throughout.
- **BOOK-COMPARE-STREAM-NOT-DEPTH** (HIGH, bench) —
  `rsx-book/compare/README.md:57` labels 20/100/1000 generated price indices
  as live depth and concludes latency is flat across depth, but
  `gen_full_cycle_ops` inserts, reduces, and cancels one level before moving
  on; at most one level is live. **Fix:** call it stream cardinality/aggregate
  level churn and use `compare_naive_bench` or `deep_match_tail` for actual
  resting-depth evidence.
- **BOOK-COMPARE-P99-MISSING** (HIGH, bench) — all four latency tables in
  `rsx-book/compare/README.md:55` publish Criterion medians without same-run
  p99, contradicting root and crate-local evidence rules. **Fix:** extend the
  contender harness to collect p50+p99, or remove exact tables until it does.
- **BOOK-DOC-ARCHITECTURE-DRIFT** (MED-HIGH, docs) —
  `rsx-book/ARCHITECTURE.md` says live recentering is incremental/no-pause
  although ME calls eager `recenter_now`; reports `PriceLevel` as 24 B instead
  of 32 B; counts two level arrays although migration holds three; reports an
  ~8 MB event buffer without a layout assertion; says matches touch only one
  cache line although maker order IDs are in the second; and treats every FOK
  level as one price although compressed slots require an order walk. README
  also says slab pages fault in lazily, but `Slab::new` initializes every
  slot. **Fix:** derive the architecture tables and paths from current types
  and live callers; keep latency numbers in dated p50+p99 reports.
- **BOOK-DOC-RATIONALE-DRIFT** (MED, docs) — `rsx-book/WHY.md` and
  `notes/{align,arena,hotcold,occupancy}.md` generalize one fixed-work sweep
  into constant-work matching, publish point estimates without p99, describe
  the wrong cache-line offsets/accesses, omit `occupancy.md` from the index,
  and retain stale memory/adjacency claims. **Fix:** rewrite each note as
  Problem→Fix→Cost, remove raw latency, and match the live structs.
- **BOOK-DOC-COMPARE-DEMO-DRIFT** (MED, docs) —
  `compare/hftbacktest.md` still calls next-level lookup a linear scan;
  `compare/naive-btree.md` describes already-merged work as a future sibling
  worktree; cited comparison numbers lack direct primary links; demo docs
  retain the removed depth-invariant/~70 ns story, call the fixed event buffer
  a ring, use nonexistent "Δ-compression" terminology, and label illustrative
  lanes as measured winners. **Fix:** scope every claim to its workload and
  make the 2026-07-27 p50/SE/p99 report the demo evidence source.

## Status — 2026-07-27 — book event-buffer overflow

- **BOOK-EVENT-OVERFLOW-NONRESUMABLE** (HIGH, correctness) —
  `rsx-book/src/book.rs:128` panics when one order emits more than 65,536
  events. The match path decrements maker/taker quantities and level totals
  before emitting `Event::Fill` (`rsx-book/src/matching.rs:317`), so replacing
  the panic with `false` would still leave a partially applied order that
  cannot be retried safely. `rsx-book/benches/deep_million_bench.rs:70`
  explicitly verifies the overflow panic. Accumulating events across several
  orders does not solve this; `process_new_order` clears the reserved buffer,
  and multi-order buffering increases the uncommitted state covered by one
  drain. The fixed cap is retained only to keep the
  current implementation educationally compact. **Fix requires design
  sign-off:** make matching
  resumable with an explicit continuation containing taker remaining quantity,
  current price level and maker handle, plus terminal-event state; stop before
  the next mutation when output capacity is insufficient, drain a bounded
  batch, then resume exactly once.

## Status — 2026-07-27 — book/matching ownership boundary

- **BOOK-MATCHING-OUTPUT-OWNERSHIP** (design — not a correctness bug) —
  `rsx-book/src/matching.rs` owns the cross/rest algorithm and writes events
  into `Orderbook`, while `rsx-matching` owns WAL and transport consumption.
  The intended boundary is a data-structure-only `rsx-book` plus a
  deterministic `rsx-matching` engine that owns matching policy, typed events,
  and a reusable caller-provided output buffer. **Deferred:** moving it also
  changes snapshot/replay imports, benchmarks, marketdata integration, and the
  event-overflow continuation design above; do not half-move it.

## Status — 2026-07-23 — gateway egress latency truthing (in progress)

Founder-directed follow-up to GATEWAY-LATENCY below. Found while re-measuring
the live cluster with the embedded per-leg stamps (`server_latency_ns`:
`internal`, `gateway_to_risk`, `risk_to_me`, `matching`,
`match_done_to_gateway`), never client wall-clock.

- **GW-PENDING-TIMEOUT-STRANDS-RESTING-ORDERS** (MED, correctness) — the
  gateway's `pending` tracking map (keyed by order_id/client_order_id, used by
  the `Cancel` frame's `find_by_order_id`/`find_by_client_order_id` lookup) is
  swept for "stale" entries every 100ms using `cutoff = now -
  order_timeout_ms` (`RSX_GW_ORDER_TIMEOUT_MS`, default **10_000ms** —
  `rsx-gateway/src/config.rs:94`), keyed off the order's ORIGINAL submit
  timestamp — not its terminal state. A GTC order still resting in the book
  past 10s is evicted from `pending` anyway (`route_order_inserted` never
  removes it, only done/cancelled/failed do; the sweep doesn't check
  liveness), so `Cancel` can no longer find it and returns error 1005 "order
  not found" for an order that is very much still live. Reproduced live: the
  Go market-maker's own graceful-shutdown cancel-all
  (`rsx-maker/maker.go:shutdown`) silently failed for every resting order
  older than ~10s, leaving orphaned resting levels behind on every maker
  restart (observed piling up across several `/api/maker/config` restarts
  during this session — 7 distinct stale bid levels, 7 distinct stale ask
  levels, all un-cancellable). Any real client with a long-lived GTC order
  hits the same wall. **Fix (not applied — record only, needs sign-off since
  it changes pending-map lifetime semantics):** either stop sweeping an
  order once `route_order_inserted` confirms it rests (move it to a separate
  "resting" index with no TTL, only removed on done/cancelled/failed), or key
  the sweep off last-known-state instead of submit time. Out of scope for the
  gateway-egress work below; recorded so a future cancel-reliability pass
  doesn't have to re-discover it.

- **bench-match-rt models 2 hops, not 4** (methodology note, not a bug) —
  `rsx-cli/src/bin/bench_match_rt.rs` simulates `gw_send → udp_to_me →
  me_dedup → me_wal_accept → me_match → me_wal_events → me_send → udp_to_gw`:
  ONE cast-send/recv pair standing in for "gateway ↔ ME" directly. It never
  calls into `rsx-risk` at all — no margin check, no persist ring, no
  risk-side cast hop. The real production path is
  gateway→risk→ME→risk→gateway (two risk hops bench-match-rt doesn't model).
  So its 12µs/17µs/37µs (p50/p95/p99) is a real, useful lower bound on the
  cast-transport + matching cost, but is not directly comparable to a live
  4-hop measurement — the live path has strictly more work in it by
  construction. Noted here so a future reader doesn't treat the two numbers
  as measuring the same thing.

- **Live cluster runs debug builds by default; no release build existed**
  (methodology fix, not a code bug) — `rsx-playground/runtime.py
  build_spawn_plan(..., release=False)` is hardcoded from
  `server.py get_spawn_plan`, and `target/release/` had no binaries at all
  before this session. Comparing live per-leg numbers (debug, ~5-20x slower
  on CPU-bound Rust) against bench-match-rt's release-mode figure was
  apples-to-oranges. Added a measurement-only opt-in,
  `RSX_PLAYGROUND_RELEASE=1` (env, read by `get_spawn_plan`, default
  unchanged/off) so `perf-load`-style runs can be driven against
  `target/release` binaries. `do_build()` (debug rebuild on every
  `start-all`) is untouched — it doesn't touch `target/release`.

- **Shared, loaded host** — `uptime` showed load average ~10 on an 8-core
  box even before starting any stress load (dockerd, an unrelated
  funding-bot, a headless Chromium instance, journald all resident). Every
  process here (including risk's busy-spin hot loop, which is not
  core-pinned in the playground's default env) is exposed to OS-scheduler
  preemption from unrelated neighbors. At higher connection counts
  (30+ concurrent WS conns) `risk_to_me` — not `match_done_to_gateway` —
  became the dominant leg (tens of ms), which is consistent with
  noisy-neighbor preemption of risk's un-pinned spin loop, not necessarily a
  risk-side architecture defect. The gateway-egress fix below is evaluated
  against a modest, valid, low-noise load point (10-20 connections,
  2000 orders/s) where the whole system stays out of saturation; see
  `reports/20260723_gateway-egress.md` for the numbers and this caveat
  spelled out.

### Plan: gateway egress fix (root cause → change → invariants)

**Root cause (unchanged from the original GATEWAY-LATENCY diagnosis,
reconfirmed live):** the gateway is ONE monoio single-threaded runtime.
The WS accept loop, every per-connection handler task, AND the
casting-recv/route loop (risk→gateway response path) are all cooperatively
scheduled on the SAME OS thread via `Rc<RefCell<GatewayState>>` (`!Send`
by design). The casting-recv loop's `try_recv_with` — and therefore
`route_fill`/`route_order_done`/etc, and therefore the WS write that
finally reaches the client — cannot run until this ONE thread's executor
gets back around to polling it, which competes with however many WS
connection tasks are also runnable. `match_done_to_gateway` (ME's
match-done timestamp → gateway's egress-serialize timestamp) is exactly
this dependency.

**Proposed change (the tile-split BUGS.md already named as the deferred
fix — implementing it now per founder direction; no smaller structural
lever was found — see report for what was tried first):** move the
casting-recv loop off the monoio thread entirely, onto its own dedicated
`std::thread` running a plain busy-spin `try_recv_with` loop (same pattern
already proven by `rsx-risk`'s hot loop: std non-blocking UDP, no async
runtime). That thread ONLY decodes wire records (all `Copy`/`repr(C)`,
trivially `Send`) into a small owned enum and pushes them onto an `rtrb`
SPSC ring — it never touches `GatewayState` (still `Rc<RefCell>`, still
single-thread-owned). The monoio thread drains that ring (cheap,
non-blocking `pop`, no syscall) and calls the EXISTING `route_*` functions
unchanged. Cross-thread wake mirrors the readiness-dup trick already in
`main.rs` (a small loopback UDP "doorbell": the recv thread pings it after
each drained batch; the monoio thread wraps its own end via
`monoio::net::udp::UdpSocket::from_std` + `readable()`, exactly like the
existing cast-fd dup) — no new dependency category, same technique twice.
FAULTED/RECONNECT handling (`handle_replay` + `reset_after_replay`) moves
to the new thread with it; `handle_replay` is already a free function with
no `Rc`/`RefCell` in its signature, so it's unaffected by the thread move.

**Status: IMPLEMENTED, verified functionally; live measurement
inconclusive on this host (2026-07-23).** `rsx-gateway/src/cast_recv.rs`
(new module) + `main.rs` wiring landed exactly as planned above. `cargo
test -p rsx-gateway` (52 tests, 9 new), `cargo test --workspace --lib`,
`cargo clippy --workspace --all-targets -- -D warnings` all green. Live
fault-injection (a partial gateway restart mid-stream) exercised
FAULTED→`handle_replay`→`reset_after_replay` for real on the new thread
and the cluster recovered. **What the live numbers do NOT show:** a
confident, quantified win for `match_done_to_gateway` — 8-sample
before/after medians on this shared host (411.6µs → 539.3µs) overlap
inside the noise band, while the two untouched legs (`gateway_to_risk`,
`matching`) stayed tight and nearly identical before/after (13.0→12.6µs,
11.6→11.8µs across all 16 runs) — proof the swings elsewhere are real
host contention (`uptime` load average ranged 8→20 on this 8-core box,
unrelated Docker tenants resident throughout), not generic jitter, and
most plausibly land on `risk`'s un-pinned busy-spin loop rather than the
gateway. Full numbers, caveats, and the honest conclusion:
`reports/20260723_gateway-egress.md`. Not closing this item — a
follow-up on a quiet/pinned host is needed to cleanly attribute the
remaining tail.

**What it must not break:**
- rsx-cast is untouched — `CastReceiver`/`CastSender` are used exactly as
  today, from a different call site.
- `GatewayState` stays `Rc<RefCell<...>>`, single-thread-owned — no `Arc`/
  `Mutex` rewrite, no change to `handler.rs`/`egress.rs`/`route.rs`
  call signatures.
- Every existing gateway invariant: fills precede ORDER_DONE, exactly-one
  completion per order, FIFO/ordering within a `user_id`'s stream (the ring
  is FIFO per producer/consumer pair, so per-symbol/per-user ordering as
  currently delivered by `CastReceiver` is preserved).
- Graceful shutdown: the new thread must exit on the same shutdown signal
  (`install_shutdown_handler`'s `Arc<AtomicBool>`, shared) and be joined
  before the process exits, mirroring `rsx-risk`'s persist-thread
  shutdown/join pattern.
- `make test`, `cargo test -p rsx-gateway`, `make lint` green throughout.

## Status — 2026-07-20 — docs foundation for the ELI13 tooltip walkthrough

Record-only per triage; found while building the tooltip/walkthrough source of
truth. Blocks terminal-doc tooltip reuse, NOT the glossary (decoupled).

- **DOCS-TERMINAL-CRATE-STALE-RSX-TUI** (MED, docs) — the shipped terminal is
  `rsx-term` (Go binary with `go.mod`, connects over WebSocket, Bubble Tea),
  but the docs describe a non-existent `rsx-tui` (Rust/ratatui,
  protobuf-over-QUIC): `docs/concepts/10-trading-terminal.md` (entire page),
  `docs/concepts/index.md:51-55`, and the project `CLAUDE.md` crate-layout
  ("rsx-tui/ … ratatui trading terminal"). `rsx-tui/` does not exist and is
  not a Cargo workspace member; 22 md files carry the stale `rsx-tui` name
  (`rsx-term` is the real one, 49 md files). A tooltip reusing
  `10-trading-terminal.md` would misdescribe the real terminal.
  **Fix (defer, record only):** rewrite `10-trading-terminal.md` to describe
  rsx-term (Go / WebSocket / Bubble Tea) grounded in
  `rsx-term/{README,ARCHITECTURE}.md` + `specs/2/{54-tui-access,55-terminal}.md`;
  fix the `index.md` line + `CLAUDE.md` crate count; sweep the other 20
  `rsx-tui` mentions.

## Status — 2026-07-18 — whole-exchange walkthrough recording

Record-only per triage; found while frame-verifying the whole-exchange
demo (`demo/exchange-walkthrough.sh`).

- **GW-HEALTH-ORDERS-PROCESSED-NEVER-INCREMENTED** (LOW, ops) — the
  gateway exports an `orders_processed` health counter
  (`rsx-gateway/src/main.rs:215`) but nothing in the crate ever
  increments it (risk does, `rsx-risk/src/main.rs:724`): on a fresh trio
  with a traced, filled order the gateway's `/metrics` still reports
  `orders_processed: 0` while `connections: 1` and the pending queue show
  traffic. Downstream, the Topology page's Cast Connections "Gateway →
  Risk" leg truthfully renders the 0 (`rsx-playground/server.py`
  `_cast_pipe_counts`), reading as a dead pipe next to live Risk → ME /
  ME → Mktdata counters. **Fix:** `fetch_add` the gauge where the gateway
  forwards an accepted order onto the cast bridge.

## Status — 2026-07-18 — playground dashboard walkthrough recording

Record-only per triage; found while frame-verifying the scene-driven
dashboard tour (`rsx-playground/demo/dashboard-walkthrough.sh`). Both are
playground display bugs, not exchange faults.

- **PG-RISK-NEGATIVE-EQUITY-UNSIGNED** (MED, correctness) — the risk page
  renders account equity through `format_notional(abs(equity))`
  (`rsx-playground/pages.py:4285`), so a NEGATIVE equity displays as a
  positive dollar figure: during the demo scene the liquidation victim's
  equity −$25,007.86 (collateral $12,015.14 + uPnL −$37,023.00) showed as
  "$25,007.85" — an underwater account reads as solvent. **Fix:** drop the
  `abs()`, format the sign (and color red when negative) like the uPnL cell.
- **PG-TRADE-AGG-LAST-PX-IS-OLDEST** (LOW, correctness) — the book page's
  Trade Aggregation card shows "Last Px" of the OLDEST fill in its window:
  `parse_wal_fills()` returns newest-first (`server.py:2209`), and
  `render_trade_agg` overwrites `last_px` on every iterated row
  (`pages.py:5168`), leaving the tail = oldest. Observed frozen at 30187
  through the whole +45% pump while Live Fills printed 0.0415. **Fix:**
  set `last_px` only on the first row seen per symbol (or iterate oldest
  → newest).

## Status — 2026-07-12 — demo-scene rehearsals (record-only)

Found while rehearsing the dynamic-market demo scene (`playground
demo-scene`). The md ShadowBook default-anchor aliasing found in the same
session was FIXED in rsx-marketdata (subscribe no longer creates books;
shadow book recenters like the ME) — these remain open:

- **RISK-SINGLE-REPLAY-ENDPOINT-PANICS-TRIO** (HIGH, availability) — risk
  has ONE gap-replay endpoint (`RSX_ME_REPLICATION_ADDR`, wired by
  runtime.py to the FIRST symbol's ME). On the trio, a gap on the SOL
  stream asks the PENGU ME's replication server, is refused (`replay
  refused stream_id=3 … my_oldest=0 my_highest=0`) and risk PANICS
  (`failover.rs:102`) — observed mid-rehearsal, supervisor restart loses
  in-RAM state (a staged under-margin victim survived only because its
  position had hit PG write-behind). **Fix:** per-stream replay endpoints
  (mirror `RSX_ME_CAST_ADDRS`) or refuse-tolerant failover; a refused
  replay must not be fatal.
- **MD-REPLAY-APPLY-IS-NOOP** (MED, correctness) — marketdata's
  FAULT/RECONNECT gap replay (`rsx-marketdata/src/main.rs handle_replay`)
  passes an `apply` callback that only `tracing::debug!`s the record —
  replayed inserts/cancels/fills are never folded into the shadow book,
  so every FAULT silently corrupts md's depth until restart. **Fix:**
  fold replayed records through the same handle_insert/cancel/fill path,
  then resend snapshots.
- **RISK-CMP-SEQ-DOMAIN-CONFLATION (diagnosis note for
  RISK-CMP-DESYNC-ORDER-EVENT-STARVE below)** — in the wedged state risk
  loops `replay drained … new_tip=13321` → `cmp sender reset detected:
  seq=11956 expected=13322` forever: the replay tip (WAL record seq) runs
  AHEAD of the live cmp stream's seq counter, so after every replay the
  next live packet looks like a sender reset and re-FAULTs. The two seq
  domains (replayed record seq vs live cmp seq) appear conflated in the
  receiver's reset_after_replay. Once diverged the boot never recovers —
  matches the "order events die minutes after boot" symptom.

## Status — 2026-07-12 — rsx-term live walkthrough recording

Record-only per triage, found while recording the rsx-term feature-tour GIF on
the live `make demo` trio. Neither is an rsx-term fault; both reproduce with
the terminal as a plain WS client.

- **RISK-CMP-DESYNC-ORDER-EVENT-STARVE** (HIGH, correctness) — minutes after a
  fresh `reset + make demo` boot, user orders stop producing completion events:
  the terminal's `sent BUY … [IOC]` gets no Accepted/Done/Rejected/Fill back
  (first ~1 min: normal ~12 ms RTT; then multi-second RTTs; then nothing).
  `log/risk-0.log` shows `rsx_cast::cast: cmp sender reset detected: seq=N
  expected=M, re-sync` every ~2-3 s throughout — the risk↔ME cmp path is
  permanently desynced, so order round-trips are lost. Same window shows
  crossed/stale books at the terminal (md shadow diverges from `/api/book`).
  Likely the same family as `.ship/25-CMP-RELIABILITY-V2`. **Fix:** make the
  cmp re-sync actually recover in-flight completions (or fault loudly), and
  alert when `cmp sender reset` repeats at steady state.

## Status — 2026-07-11 — 3-token demo (display + maker polish)

The exchange core is fixed and correct — 3 tokens trade live, orders fill, MEs
FAULT=0 (the SEQ-1 per-ME fan-out re-sequencing + reset PG-wipe, in git). These
two are display/maker-tuning follow-ups; neither is an exchange fault.

- **DEMO-MD-DISPLAY-STARVE** (MED, playground) — the dashboard's own marketdata
  reconstruction (`rsx-playground/server.py` `_book_snap`, the MD-WS subscriber
  ~200-300) starves after the racy demo startup (drops the app-level heartbeat
  or misses the initial `B` snapshot), so `/api/bbo` + `/api/book` freeze and
  drift crossed. The EXCHANGE is correct: the authoritative direct MD WS
  (`ws://127.0.0.1:8180`) shows moving, uncrossed books (that's what the demo
  GIF `demo/run.py` reads). The BBO-reconcile commit uncrosses the top but does
  not cure the starve. Fix candidate: reconnect + re-subscribe + re-request a
  snapshot on drop; keep the heartbeat alive.
- **DEMO-MAKER-TOUCH-LOCK** (LOW, maker) — with `RSX_MAKER_JITTER_BPS` on, the
  maker's jittered cancel-replace leaves same-price orders on both sides, so the
  top-of-book locks (bid==ask, 0bps in the GIF) instead of showing the 20bps
  spread. Cosmetic. Fix candidate: ensure the prior ladder fully cancels before
  the new one rests (or widen spread relative to jitter).
- **MAKER-ORPHANED-QUOTES-ON-CANCEL-MISS** (MED, correctness) — the demo
  maker's cancel-replace fails repeatedly (`log/me-pengu.log`: `cancel: order
  not found user=99 id=...`, the ME's cancel-miss warn at
  `rsx-matching/src/cancel.rs:35-42`), leaving orphaned quotes that
  accumulate. The earlier reading that the ME book locks or crosses is
  refuted: the observed `/api/book/10` bid == ask == 30254 comes from the
  playground's marketdata cache (`rsx-playground/server.py:8505-8510`; see
  DEMO-MD-DISPLAY-STARVE), and the ME cannot rest a crossing order
  (`process_new_order` matches before `insert_resting`,
  `rsx-book/src/matching.rs:28-36`). The gateway-side miss is a different
  mechanism (GW-PENDING-TIMEOUT-STRANDS-RESTING-ORDERS, error 1005, not this
  log line). **Fix:** the maker must reconcile its resting set from acks
  before re-quoting; the ME cancel-miss rate is the tell (today it is a warn
  with no counter: ME-CANCEL-MISS-SILENT, ME-METRICS-MISSING-DROPS-AND-
  REJECTS). No test — needs the live maker.

## Status — 2026-07-11 — HFT portfolio and AI-development audit

Record-only from the career-facing evidence audit. These findings do not claim
that RSX is production-ready; they identify where public claims, measurements,
and the demo currently fail to prove the low-latency/HFT capability they are
intended to demonstrate. Existing runtime issues remain under their original
entries below.

- **DOC-UNSUPPORTED-LOWEST-LATENCY-CLAIM** (HIGH, traceability) — the first
  public claim is “Your Lowest-Latency Derivatives exchange” at `README.md:3`,
  but the same README reports the actual cross-process path at ~1.1 ms
  (`README.md:91-94,275-281`) and contains no controlled competitor comparison.
  A careful hiring reviewer can disprove the superlative from the next screen,
  weakening trust in the valid component results. **Fix:** replace the headline
  with a factual scope statement (“A spec-first derivatives exchange and
  low-latency systems laboratory”), keep the <50 µs number explicitly labelled
  as a target, and add a generated claim check that fails if “lowest”, “fastest”,
  or “sub-10 µs” appears without “target”/“aspirational” in the same paragraph.
  **Accept:** `rg -n -i 'lowest.latency|fastest' README.md ONEPAGER.md` returns
  no matches, while the How-fast table still exposes in-process and
  cross-process results separately.

- **STRESS-ZERO-ACCEPT-FALSE-PASS** (HIGH, bench) — the stress reporter treats
  an empty latency sample as zero and compares it with the target, producing
  `submitted=10 accepted=0 ... p99=0us` followed by `p99=0us ... pass`
  (`log/stress.log:127-150`; at least 20 such runs in the current sample).
  This converts total order-path failure into a green performance result.
  **Fix:** require `accepted > 0`, `accepted/submitted >= 0.95`, a configured
  minimum sample count (default 1,000), and no invariant failures before
  computing or grading percentiles; emit `FAIL: no accepted samples` with a
  non-zero exit code otherwise. Add fixtures for zero accepted, partial
  acceptance, all accepted, and missing latency samples. **Accept:** the
  zero-acceptance fixture exits non-zero and contains no `pass`; a live run's
  JSON records submitted, accepted, rejected by reason, acceptance ratio,
  sample count, p50/p99/p99.9/max, and overall status.

- **BENCH-CROSSPROCESS-TAIL-LOAD-PROOF-MISSING** (roadmap, not a defect) — the
  public proof ladder stops at a cross-process p50 of 1,128 µs with no p99
  (`docs/benches.md:132`), while the strong 7.82/22.3 µs p50/p99 result is
  explicitly in-process (`docs/benches.md:137-147`). There is no reproducible
  sustained production-path report combining accepted throughput, p99/p99.9,
  maximum, loss, invariant results, CPU/queue saturation, and recovery. **Fix:**
  add a checked-in `scripts/portfolio-load.sh` that runs the real
  GW→Risk→ME→Risk→GW path for ten minutes, fails below 95% acceptance or on any
  invariant/loss, repeats three times, and writes a dated report containing
  commit, exact command, topology, sample count, offered/accepted rates,
  p50/p99/p99.9/max, CPU, queue depth, and failures. **Accept:** three successive
  reports complete without manual edits and p99 varies by <=10%; until then the
  README labels cross-process performance as incomplete evidence, not HFT parity.

- **BENCH-HARDWARE-CONTROLS-NOT-DEMONSTRATED** (roadmap, not a defect) — the
  strongest load report explicitly uses a shared 6-core box with no CPU
  isolation (`reports/20260530_load-curves.md:11-14`), and the July rerun calls
  its tails noisy on a shared host (`reports/20260708_bench-rerun.md:55-59`).
  Deployment prose mentions `isolcpus`/pinning, but no reproduced result proves
  real NIC, IRQ/queue placement, NUMA locality, frequency/C-state policy, PTP or
  clock source, or before/after kernel tuning. **Fix:** produce one bare-metal
  report capturing `lscpu -e`, `/proc/cmdline`, governor/turbo/C-state state,
  `numactl --hardware`, `ethtool -i/-l/-x/-S`, `/proc/interrupts`, thread/core
  mapping, clock source/PTP status, kernel/compiler versions, and three
  before/after production-path runs. **Accept:** every published hardware claim
  links to that report; DPDK/AF_XDP/FPGA/SmartNIC remain explicitly “not
  implemented” until a runnable artifact and measurement exist.

- **DEMO-RISK-REPLICA-MISSING-REPLAY-CONFIG** (HIGH, config) — **FIXED
  2026-07-11 (commit pending — refine round 1).** The replica spawn in
  `rsx-playground/runtime.py build_spawn_plan` now sets `RSX_ME_REPLICATION_ADDR`
  = `me_replication_addr` (the same single first-symbol replay addr the primary
  uses) and `RSX_RISK_MAX_SYMBOLS` = `max(id)+1` (was `len(symbols)`, which made
  the warm replica index OOB on the first PENGU fill, id 10). Verified: `demo
  full` brings up all 10 processes incl `risk-0-replica-0` (state=running); the
  replica warm-catches ONE stream (`dxs consumer stream_id=10 … 127.0.0.1:9710`,
  `warm catchup: consuming ME replication stream_id=10 … caught up`); and `rg -i
  'panic|Name or service not known' log/risk-0-replica-0.log` is empty. The
  original report follows.
  The current
  replica log reaches its health-listener startup and then panics because
  `RSX_ME_REPLICATION_ADDR` is absent (`log/risk-0-replica-0.log:32-34`). This
  overlaps `STARTUP-ORDERING-FRAGILITY` below but adds a concrete spawn-plan
  configuration defect: a portfolio demo can display a partially green cluster
  while redundancy is dead. **Fix:** make the playground spawn-plan constructor
  derive and validate the ME replay address for every risk primary/replica
  before launching any process; fail the whole scenario atomically with a named
  config error, and make readiness require every declared replica. Add a spawn-
  plan unit test plus a clean-start acceptance test. **Accept:** after
  `./rsx-playground/playground reset && make demo`,
  `rg -i 'panic|fatal|required.*NotPresent' log/risk-*-replica-*.log` returns no
  matches, the process API reports all declared processes ready, and Verify
  fails if any replica is removed.

- **DOC-HIRING-FIRST-FOLD-BURIES-EVIDENCE** (MED, docs) — `README.md:5-21`
  spends the first screen on a poem before stating scope, while ownership,
  measured-vs-target status, and the strongest reproducible result are scattered
  later. The material is distinctive but delays the evidence a recruiter needs
  to classify the project. **Fix:** move the poem into an expandable/later
  “personality” section and make the first fold contain: one-sentence problem,
  one-sentence personal contribution, `make demo`, a four-row proof ladder, and
  an explicit “research system; no production users/SLA” boundary. **Accept:**
  the first 60 rendered README lines contain those five elements and every
  headline number links to a dated report.

- **DOC-AI-DEVELOPMENT-METHOD-NOT-EXPLAINED** (MED, traceability) — the
  repo labels the blog AI-generated (`README.md:468`, `BLOG.md:3`) but has no
  repository-level explanation of how LLMs were used to develop RSX. A hiring
  reviewer cannot distinguish model-generated drafts from human-selected
  architecture, implementation, adversarial review, measurement, and final
  verification; the commit volume and internal agent artifacts make that
  ambiguity more visible, not less. **Fix:** add
  `docs/ai-development.md` (“How this was built with LLMs”) and only a 3–4 line
  README disclosure linking to it. In the detailed page, for each workflow (specification,
  implementation, test generation, bug hunting, documentation), show one
  concrete input→output→human-review example; name the model/tool; state what
  it could change; link the resulting commit/spec/test; and show the independent
  acceptance command or benchmark that caught errors. Explicitly document that
  the maintainer chose the architecture, resolved conflicting suggestions,
  executed verification, and owns every shipped defect. Include failures such
  as the zero-acceptance false-green stress result to demonstrate that AI output
  is treated as untrusted until checked. **Accept:** the page lets a reviewer
  trace at least three shipped features from prompt/plan through code review and
  executable verification, distinguishes generated text from measured evidence,
  and avoids both “AI built it autonomously” and “no AI was used” narratives.

- **DOC-UPSTREAM-NOTICE-MISSING** (MED, traceability) — no `NOTICE` file records
  imported or adapted upstream work, including the arizuko runner used by the
  terminal assistant (`rsx-term/ARCHITECTURE.md:93-108`). **Fix:** inventory
  named upstream code and add `NOTICE` entries with author, URL, license,
  copyright, and whether it was imported, adapted, or merely invoked; retain
  upstream headers and licenses. **Accept:** `NOTICE` exists, every named
  adapted/imported upstream has provenance, and the README links it.

## Status — 2026-07-11 — maker refine pass (deferred findings)

Review-pass findings verified but deliberately not applied (low severity /
speculative / risky-to-fix). Applied fixes are in git: fisheye inverse
`e7a0373`, ui stub/leak `3587538`, HL Close `30b7e86`, maker dial-timeout
`4a2829f`. TERM-CONN-RECONNECT-CLOSE-TOCTOU (fixed `69f7188`) and
TERM-STARTUP-DOUBLE-SYMBOLS-FETCH (fixed `471b027`) are resolved — closed
below. TERM-HEATMAP-NO-OVERFLOW-GUARD, TERM-STREAM-LEGACY-FOLD, and
TERM-DEFAULT-NOT-OFFLINE were speculative/by-design, not actionable bugs —
dropped from the queue.

- **MAKER-READCONFIGMID-PER-SYMBOL** (LOW, efficiency) — `maker.go` re-reads +
  unmarshals the config file once per symbol per cycle; a non-issue while
  `Config.Symbols` is length 1. Hoist out of the loop if multi-symbol lands.

## Status — 2026-07-09 — matching release runtime checks (see .ship/41-MATCHING-RELEASE/HANDOFF.md)

- **MATCHING-BENCH-SHARED-HOST-VARIANCE** (MED, methodology) — the published
  matching latencies do not reproduce on the shared docker host. 2026-07-09
  idle-box run: `match_by_depth` ~48 ns (vs 30 ns), `me_accept_path/full`
  ~295 ns (vs 266 ns); throughput ~3.46M/s (vs 3.6M — stable, depth-independence
  holds). Not a regression (the pre-refactor re-bench already saw ~51 ns) —
  shared-host variance ~1.6× on the tight per-op benches. Fix: measure on a
  dedicated box before publishing 30/266 as a floor, or re-baseline the docs
  with an honest variance note. Full-workspace `make perf-gate` is nightly-scale
  (times out in-session). See `reports/20260709_matching-benches.md`.
## Status — 2026-07-09 — demo unusable: replication cert was CA-as-leaf (RESOLVED)

The "takers time out with the ME WAL empty" symptom last pass was NOT the
concurrent playground rework — it was a real bug. `gen-snakeoil-certs.sh`
wrote a single CA:TRUE self-signed cert and copied it to both `cert.pem`
and `ca.pem`; rustls/webpki reject a CA cert presented as a server leaf
(`CaUsedAsEndEntity`), so the replication TLS handshake never completed.
Risk stayed in WarmCatchup (which does no gateway ingress, `failover.rs:156`),
never promoted to Live, and every order timed out with an empty ME WAL.
Fixed: proper CA + distinct end-entity leaf (`fix` commit), playground
self-heals old certs on start, demo-trade.sh + docs updated to the live
`/api/orders/test` API and `tmp/wal/pengu/10` path, `rsx-cli` given a
`default-run`. Verified end-to-end (maker rests, taker fills, FILL in WAL):
demo-trade.sh PASS. Closes DEMO-TRADE-SH-STALE + DEMO-TRADE-SUBMIT-ORDER-404.

## Status — 2026-07-08 — book+matching refine critique (merged from .matching-book-refine)

Correctness findings from the refine-pass critique, confirmed still present in
main. The minimization/doc findings were applied (`b2d3c52`).

- **MD-SHADOW-MIXED-SLOT-BBO-L2** (MED, correctness — marketdata) — the shadow
  book reimplements BBO/L2 with the FIFO-head anti-pattern rsx-book fixed:
  `rsx-marketdata/src/shadow.rs` `derive_bbo` (~:129) + `side_levels` use
  `level.total_qty`/`order_count`/`head.price`, which in a mixed compressed
  slot drops a side / misattributes qty. Fix: `derive_bbo` → `book.current_bbo()`;
  `side_levels` → `bid_count`/`ask_count` per raw price.
## Status — 2026-07-08 — live latency legs not stamped (rsx-term speed strip)

Design settled → **`specs/2/59-latency-observability.md`** (planned). The
live internal/engine legs are delivered by **per-hop timestamps embedded in
the normal records** (not a bespoke delta frame), derived under assumed clock
sync (PTP in prod, single-box in dev); the client `net` leg stays
client-measured. Aggregate latency is Prometheus via the existing
`33-telemetry.md` Vector pipeline, not hand-rolled. The two tasks below are
now increments of spec 59:

- **GW-STAMP-LATENCY-LIVE** (feature, spec 59 inc 1-2) — gateway stamps
  `gw_in_ns`/`gw_out_ns` into the returned record (generalising
  `taker_ts_ns`); today it only logs a `latency_sample!` trace
  (`rsx-gateway/src/handler.rs:301`). With the risk/ME stamps present the
  terminal derives `internal = gw_out − gw_in` itself.
- **ME-ENGINE-LATENCY-NOT-REPORTED** (feature, spec 59 step 3) — the ME side
  is done: it stamps `me_in_ns` and `match_done_ns` on every fill and on the
  taker's OrderDone (`rsx-matching/src/order.rs:50,99`,
  `rsx-matching/src/wal.rs:101-110,219-241`) and the gateway forwards them in
  the `Fill`/`OrderUpdate` frames (`rsx-gateway/src/records.rs:25-42`). What
  is left is the consumer: `rsx-term` never reads them and its RTT fold sets
  `EngineNs` to `NsUnknown` (`rsx-term/ui/update.go:773-779`), so the speed
  strip's engine leg stays "pending" (`rsx-term/ui/view.go:726-729`, which
  cites this id). **Fix:** derive `engine = match_done_ns - me_in_ns` from the
  frames (`specs/2/59-latency-observability.md` step 3). No test — feature.

## Status — 2026-07-08 — ME→GW-direct spec is NOT ship-ready (codex adversarial audit)

- **ME-GW-DIRECT-SPEC-GAPS** (design blocker — do NOT hand to a coding agent
  yet) — the async-output-split in `specs/2/28-risk.md` (ME sends fill straight
  to gateway; settle to Risk async) is `status: partial`/not-implemented, and an
  adversarial audit found it underspecified and unsafe as written.
  Frontmatter now reads `status: partial` (`28-risk.md:2`), matching the
  status block at `28-risk.md:199-205`. Verdict: **needs-spec-work-first**.
  Gaps that must be closed BEFORE any implementation:
  1. **No authoritative client sequence** once ME (per-symbol seq) and Risk
     (risk-gw seq) both feed the gateway — two clients/replay can disagree on
     order. Today Risk restamps one
     contiguous stream (`rsx-risk/src/rings.rs:33`, `forward_to_gw`).
  2. **Reject edge contradiction** — spec says the Risk→GW insufficient-margin
     reject "cannot be removed" (`28-risk.md:171`) but also says Risk drops
     `forward_to_gw`/response ring (`28-risk.md:203-205`); `forward_to_gw` carries
     BOTH fills and rejects (`rsx-risk/src/cast_loop.rs:233`, `:303`). Drop
     it and rejects vanish → client can't tell rejection from transport loss.
  3. **Gateway replay for the ME→GW stream is undefined** — no
     `RSX_ME_GW_REPLICATION_ADDR`, no per-symbol tip/replay-source/dedup; today
     gateway replays only Risk (`rsx-gateway/src/cast_recv.rs:86-99`,
     `replay.rs:24`). A dropped direct fill never reaches the client.
  4. **ME live-stream contents under-specified** → false FAULTED gaps: if the
     gateway gets only fill/DONE/FAILED, the WAL-seq holes at ORDER_ACCEPTED/BBO
     look like loss.
  5. **Duplicate fills** — no dedup key (`fill_id` / `(symbol,seq)`); during any
     mixed rollout the client double-counts (`rsx-gateway/src/route.rs:35`
     pushes blind).
  6. **Reduce-only solvency — FALSE POSITIVE (2026-07-08).** ME *does* hold
     an authoritative per-symbol position (`net_qty`, `rsx-book/src/user.rs`)
     and clamps/rejects reduce-only against it synchronously at match time
     (`matching.rs:59-94`); the view is never stale (single-threaded, in-order
     fills). The posited long-1 → double-reduce → naked-short cannot occur: the
     second reduce-only hits `net_qty==0` → `FAIL_REDUCE_ONLY`. Risk's `Ok(0)`
     is correct (reduce frees margin; enforcement is ME's — `28-risk.md:351`,
     `47-validation-edge-cases.md:145`). Position recovery is exact (snapshot
     `net_qty` + replay through `process_new_order`). No fix.
  7. **Liquidation race — FALSE POSITIVE (2026-07-08).** `UserInLiquidation`
     is checked synchronously at Risk pre-flight on the next order; the only
     stale-position hazard was reduce-only, which ME owns (see 6). No
     independent hole.
  8. **Exactly-one-completion** at risk — direct ORDER_DONE clears pending, a
     late Risk ORDER_FAILED emits a second terminal (`route.rs:201-216`).
  **Settled 2026-07-08.** The acceptance-commits invariant — once Risk
  pre-flight accepts, the fill is guaranteed; Risk only *processes* fills, it
  never vetoes — dissolves gaps 1, 2, 8: per order exactly one terminal
  producer (Risk iff pre-flight-rejected, else ME), no cross-stream ordering
  conflict, no late Risk terminal. Gaps 6, 7 are false-positives (above).
  Remaining work is **mechanical transport only**: gateway as a full
  ME-replication consumer (3, 4) + `fill_id` dedup (5). Still needs the spec
  rewritten around the invariant + the transport story before implementation,
  but it is a bounded feature, not the high-risk rewrite the audit implied.
  See ME-HOLDS-USER-STATE for the related edge-buffer direction.

- **ME-HOLDS-USER-STATE** (design, OPEN/DEFERRED — record-only) — ME
  currently stores two pieces of per-user state in `rsx-book`: `net_qty`
  (per-symbol
  position, for the reduce-only clamp, `user.rs`) and the `DedupTracker`
  (`(user_id, order_id)` idempotency, `dedup.rs`). Neither is an account — no
  balances/margin/collateral (those are Risk's, which is *why* Risk shards by
  user: portfolio margin ME structurally can't compute). The rule: **ME holds
  exactly the per-user state whose decision must be made at the match sequence
  point** — position-as-of-match for reduce-only, already-seen for dedup — the
  truths Risk's async view can't provide in time. Cost: it drags in per-symbol
  slab-GC for idle users (`order_count`/`zero_since_ns`/`RECLAIM_GRACE_NS`).
  Open question: can the edge hold *less*? Direction is the generalization —
  a pre-authorized risk **buffer** at the ME edge (Risk grants a conservative
  per-symbol margin allowance; ME draws down synchronously; Risk tops-up /
  revokes async), which would also let the *forward* Risk hop be skipped
  (`GW→ME→GW`). Hard part = buffer sizing + cross-symbol partitioning +
  revocation as mark moves; not the hot path. State is isolated behind
  `UserRegistry` (`rsx-book/src/user.rs`) as a clean seam to change/remove.
  **Deferred:** `UserRegistry` is serialized by
  `rsx-book/src/snapshot.rs:73-76,183-188`, and WAL replay rebuilds
  positions by running the same matching path. Move it to `rsx-matching` only together with an
  explicit matching-state snapshot/replay boundary; moving the type alone
  would make reduce-only recovery incomplete. Unsolved.

## Status — 2026-07-08 — eager recenter is a tail-latency spike (by design, revisit)

- **RECENTER-EAGER-TAIL-SPIKE** (MED, latency/design — not a correctness bug) —
  `maybe_recenter` → `Orderbook::recenter_now` migrates the whole book
  **eagerly in one shot** (`rsx-book/src/migration.rs:254`,
  `complete_migration_eager:236`). Cost is O(old slot count ≈ 617k for
  BTC-PERP): it scans the old slot array, skips empty levels O(1), and remaps
  the occupied ones — bounded by slot count, not order count, but still a
  spike far above a 60 ns match, landing on whichever order triggers a >2.5%
  mid drift (i.e. during volatility, when latency matters most). **Why eager:**
  a lazy per-order `resolve_level(price)` scheme would let a marketable order's
  crossing prices lie outside a partially-migrated band, so a half-migrated
  book would miss crossing liquidity and break price-time priority
  (`recenter_now` contract, `migration.rs:247`). The incremental
  `migrate_batch` frontier walk exists but is test/non-live-only. **Fix (to
  remove the spike):** a correct lazy/incremental migration for live matching —
  either migrate only the crossing band eagerly and rest lazily on idle, or
  make the matcher consult both old+new maps safely during the migration
  window. Non-trivial ME change; needs a design + adversarial review before
  code. Until then the engine trades a rare bounded spike for the correctness
  guarantee. Concept page (`docs/concepts/05-slab-and-compression.md`) documents
  the tradeoff honestly.

## Status — 2026-07-08 — stale latency/size numbers across docs (concepts CTO review)

Surfaced while reconciling `docs/concepts/*` to the code. Both are
doc-vs-doc / doc-vs-code number drift, not runtime defects. The concept
pages were corrected to the code; the *authoritative* docs below still
carry the stale figures. Record-only.

- **DOC-RT-FLOOR-DRIFT** (LOW, docs) — the in-process round-trip floor is
  stated three ways: `README.md:234` and the concept index say 7.5 µs p50 /
  16.9 µs p99 ("measured"); `docs/benches.md:128` has `bench-match-rt` at
  9.58 µs; `reports/20260703_cast-benches.md:13` has casting RTT *alone* at
  8.802 µs p50 (which can't be under a full round-trip). The concept index
  mirrors README per house rule (match the authoritative doc), so the fix is
  to reconcile README ↔ benches.md ↔ the cast-bench report to one current,
  dated number — not to edit the concept in isolation.
- **DOC-PRICELEVEL-24B-STALE** (LOW, docs) — `specs/2/21-orderbook.md:309`
  says `PriceLevel` is 24 B; the code is 32 B (`rsx-book/src/level.rs:20`,
  `const _: assert!(size_of::<PriceLevel>() == 32)`), and `Orderbook::new`
  allocates two arrays of them (active + staging), so ~40 MB of level
  storage for the 617k-slot BTC-PERP example, not the ~15 MB the spec's math
  implies. Concept page fixed; spec still stale.

## Status — 2026-07-08 — userspace-UDP blocked by cast socket coupling

- **CAST-SOCKET-COUPLING-BLOCKS-IOURING** (roadmap, not a defect) — the
  userspace-UDP / io_uring move (matching ME hot-path `recvfrom`/`sendto`;
  gateway/marketdata edges) can't be done in the callers today because
  `CastSender`/`CastReceiver` (`rsx-cast/src/cast.rs`) **own the `UdpSocket`
  and couple framing with `recv`/`send`** — `try_recv_with` does recvfrom +
  parse in one; `send_framed` writes the built `Framed` on the owned socket.
  io_uring must live in the caller (rsx-cast stays runtime-dep-free), so the
  caller needs to own the socket. **Fix (two additive cast APIs, a sanctioned
  frozen-cast extension — needs founder sign-off, not a redesign):** (1) expose
  a built `Framed`'s bytes (`Framed::as_bytes()` or similar) so the caller
  io_uring-sends them; (2) a parse-already-received-bytes entry
  (`CastReceiver::process(&[u8])` / a standalone frame parser) so the caller
  io_uring-recvs then hands bytes to cast for framing/WAL. Single-packet
  request-response gains little from batching — the real lever is SQPOLL — so
  this is a throughput/scaling step, on the roadmap (README §Roadmap), not a
  correctness fix. rsx-cast itself is sound for the current std-UDP path.

## Status — 2026-07-08 — gateway ↔ marketdata alignment (symmetry audit)

Gateway (client-WS in → cast out) and marketdata (cast in → public-WS out) are
two forks of one monoio shape (single reactor, `Rc<RefCell<State>>`,
`ConnectionState.outbound: VecDeque`, `drain_outbound`, per-source
`CastReceiver` + fault/replay). Most divergence is accidental drift; the two
real bugs are **mirror halves of the same missing symmetry** — each crate has
the fix the other lacks. Record-only per triage. (SPSC/tile note: neither uses
rtrb rings — that's Risk; these are the monoio fan-in/fan-out processes.)

- **MD-EGRESS-STALL** (HIGH, latency) — **FIXED 2026-07-08** (`353b347`).
  marketdata's handler blocked on `ws_read_frame` with no egress wake, so a
  listen-only subscriber only got updates when it next sent a frame. Fixed
  with the notify-based egress wake (below) — better than the originally
  proposed bounded-poll port. A later fable analysis found the *gateway's*
  own "500µs" poll was actually ~1–1.5 ms (monoio 0.2.4's timer wheel is
  ms-granular and rounds up), so both gateway and marketdata got the
  event-driven fix: a hand-rolled `!Sync` `EgressWaker` (`egress.rs` in each
  crate), the per-conn handler `select!`s `{readable, egress.wait()}`, and
  every `outbound.push_back` producer signals the waker. Zero extra cores,
  cancel-safe (awaits readiness, not a buffered read). `+TCP_NODELAY` on
  accept (`1ac4efa`). Commits `12de610` (gw) / `353b347` (md) / `1ac4efa`.
- **GW-OUTBOUND-UNBOUNDED** (MED, resource/DoS) — CONFIRMED. `rsx-gateway/src/
  state.rs:144` (`push_to_user`) / `:152` (`broadcast_heartbeat`) `push_back`
  onto `outbound` with no cap; a slow/non-reading authed client accumulates
  unbounded `Arc<str>`. Marketdata bounds the identical structure at
  `state.rs:73` (`max_outbound` → drop + snapshot resync). Adopt that cap +
  `RSX_GW_MAX_OUTBOUND`. (Mirror of MD-EGRESS-STALL, other direction.)
- **WS-FORK-DRIFT** (MED, duplication) — CONFIRMED. `rsx-gateway/src/ws.rs`
  and `rsx-marketdata/src/ws.rs` are forked copies of one WS impl; the gateway
  hardened its reader (mask required, FIN=0 reject, 4 KB cap) and marketdata
  kept the looser copy (mask optional, fragments accepted, **1 MB cap** on an
  unauthenticated public endpoint — MD-FRAME-CAP / MD-WS-UNMASKED). Converge
  the behavior in place (port the guards + 4 KB cap to marketdata); only THEN
  consider hoisting the domain-agnostic core (`WS_MAGIC`, accept loop, frame
  read/write) into a shared module. Auth/jti stays gateway-only.
- **MD-ACCEPT-DROPS-PEER** (LOW, hardening) — CONFIRMED. `rsx-marketdata/src/
  ws.rs:17` types the accept closure `Fn(TcpStream)` and discards `peer`
  (`main.rs:332`); the public endpoint has no per-IP/connection cap and can't
  even see the source IP. Keep the signature symmetric with the gateway
  (`Fn(TcpStream, SocketAddr)`) so a cap becomes possible.
- **MD-PIN-DOC-CONTRADICTION** (LOW, docs) — CONFIRMED. marketdata pinning is
  described three contradictory ways: `ARCHITECTURE.md:174` "no core_affinity",
  README "busy-spin loop", root `CLAUDE.md` "pinned for keep-up"; code pins iff
  `RSX_MD_CORE_ID` set + busy-polls via `sleep(Duration::ZERO)`. Reconcile to
  one story (busy-poll-for-keep-up, optionally pinned).
- **GW/MD-README-ENV-DRIFT** (LOW, docs) — CONFIRMED. Both READMEs document
  env names the code doesn't read (`RSX_GW_LISTEN_ADDR` vs code `RSX_GW_LISTEN`;
  `RSX_GW_HEARTBEAT_INTERVAL_MS` vs `_S`; marketdata README `RSX_MKT_*` vs code
  `RSX_MD_*`). Also `rsx-gateway/ARCHITECTURE.md:133` says "10ms" where code is
  500µs. Fix the env tables on both.
- **GW-HEARTBEAT-DUAL-PATH** (LOW, complexity) — CONFIRMED. Gateway sends
  heartbeats from both the per-connection handler (`handler.rs:88`) and the
  main-loop `broadcast_heartbeat` (`main.rs:477`), which doesn't update
  `last_heartbeat_sent_ns` → ~2× heartbeats. Marketdata does only the main-loop
  path. Collapse the gateway to the simpler shape.
- **DRAIN-REPLAY-4X-DUP** (LOW, duplication — do NOT act) — `drain_replay` is
  byte-identical across gateway/marketdata/matching/risk (4 copies) but builds
  a tokio runtime, so its natural home (rsx-cast) can't take it — rsx-cast is
  FROZEN and never gets a runtime dep. Extraction needs founder sign-off on a
  NEW shared crate. Record only.

## Status — 2026-05-30

**OPEN (triage):**
- **ENV-TEST-PARALLEL-RACE** (LOW, test-isolation) — **Status: FIXED
  2026-07-07.** The two env_* tests in each `me_cast_addrs_test.rs`
  (marketdata + risk) mutate process-global `RSX_ME_CAST_ADDR(S)` and raced
  when run on parallel threads within a binary. Fixed with a file-local
  `static ENV_LOCK: Mutex<()>` (poison-tolerant `.unwrap_or_else(into_inner)`)
  guarding the two env tests in each file — no new dep. Verified: 10/10
  green over repeated default-parallel runs.
- **CAST-BIG-GAP-ESCALATION** (LOW, design) — a NAK gap that stays *under* the
  2048 reorder ring recovers seq-by-seq (each fill retransmit is a WAL scan);
  only ring overflow escalates to the TCP-replay cold path. A gap far beyond
  WAL retention could proactively escalate to replication instead of grinding.
  Characterized 2026-07-07 by the landed `loss_degradation` + `outage_recovery`
  benches: the overflow→replay path works (52 k-record gap replays in ~1 s),
  and the live NAK path reliably delivers through ~30% loss; the grind only
  bites a slow-growing sub-ring gap. Enhancement, not a bug. (The 2026-07-06
  "run_once hung" finding was a HARNESS bug — the callback didn't stop on
  `RECORD_CAUGHT_UP` so it tailed live forever, plus a WAL firehose; fixed in
  the landed outage bench, 25c9aa2 / 50c78f9. Not a cast defect.)
- **TIME-NS-HOTPATH-AUDIT** (LOW, perf) — flagged 2026-07-06 during the
  rsx-cast review. `time_ns()` (clock_gettime via vDSO, ~15-30 ns) is called
  per-record in a few spots (replication-server CaughtUp/record stamping, WAL
  timestamps). Audit call frequency; if any is a true per-record hot path,
  stamp once per batch or use a coarser/cached clock. Not correctness — record
  + reconsider later.
- **REPLAY-RESCAN-PER-CONNECTION** (MED, perf) — flagged 2026-07-06. The
  replication server's `oldest_and_highest_seq` full-scans the *active* WAL
  file end-to-end (≤64 MB) on **every** replay connection to validate the
  request range. Fine for occasional bootstraps, but under a **reconnect
  storm** (a consumer fault-looping) it re-scans repeatedly — disk replay
  becomes I/O-bound / near-impractical. Server is a stateless dir-reader by
  design, so options that keep that: (1) incremental tail-scan (cache
  highest + active-file offset, scan only new bytes); (2) producer-published
  highest-tip file (~10 ms stale is fine for the refusal check); (3) shared
  atomic if server+writer co-located (most coupling). Leave for now.
  Confirmed 2026-07-07 by the `outage_recovery` bench: per-cycle recovery of a
  constant ~7.5 k-record gap drifts 262 → 786 ms across six cycles purely
  because the active WAL grows and each connection re-scans it.
- **BENCH-NO-TIMEOUT-GATE** (FIXED 2026-07-07) — flagged 2026-07-07;
  nothing time-bounded bench execution (no `timeout` in
  `scripts/bench-gate.sh` or the Makefile bench targets, Criterion has no
  per-bench deadline), so a hanging bench (CAST-RTT-BENCH-DEADLOCKS-ON-LOSS
  hung 50 min, then 240 s again 2026-07-07) read as "slow", not FAILED.
  Fixed: `scripts/bench-gate.sh` wraps its `cargo bench --workspace` in
  `timeout 600`; `make perf` wraps its `cargo bench` the same way. A hang now
  exits 124 -> gate fails.
- **CAST-RTT-BENCH-DEADLOCKS-ON-LOSS** (FIXED 2026-07-07) — flagged 2026-07-06.
  TWO stacked hangs, both fixed in the bench: (1) deterministic — the
  send<T> removal (bb6c1a0) moved seq ownership to the caller, and the
  bench's counter lived inside Criterion's |b| closure, which re-invokes
  per phase → counter reset → receiver dedup-drops everything → deadlock
  at warmup→collection (100% repro). Hoisted the counter out. (2) the
  original probabilistic loss-deadlock below — echo-wait now pumps
  tick+recv_control on a coarse cadence. Verified 3 clean runs
  (~9.4-9.7 µs). Original analysis:
  `cast_rtt_bench`'s A-side echo-wait is `loop { try_recv; spin_loop }` with NO
  in-loop NAK recovery: A sends seq=N, spins until B echoes. If that datagram
  (or its echo) drops on loopback, B never echoes, A never advances `iter`, so A
  never reaches its `iter & 0x3FF == 0` recv_control/tick — B's NAK goes
  unanswered → permanent deadlock (observed: 50 min stuck in a 3 s warmup).
  Pre-existing (predates the send<T>→send_framed migration, which only changed
  the seq source, values identical); earlier runs got lucky on a loss-free
  window. Fix: recv_control()/tick() *inside* the echo-wait spin (with a bounded
  spin count) so a lost frame retransmits, or add a per-iter deadline. Until
  then the casting-RTT row is measured opportunistically (~9.6 µs, 2026-07-06).
- **STARTUP-ORDERING-FRAGILITY** (MED, ops) — "can't start the system via the
  playground" traced to a chain of order-dependencies, none self-healing:
  (1) **Postgres must be up first** — if the `rsx-postgres` container is stopped/
  gone, risk-0 crash-loops "error connecting to server" and nothing comes up;
  the playground still reports "started 6 processes" (misleading). Mitigated:
  `docker update --restart unless-stopped rsx-postgres` so it survives.
  (2) **Playground manages the cluster in-memory** — restarting the playground
  server orphans the processes it spawned and they die; the new instance starts
  with an empty process table. (3) **Playground marketdata subscriber
  circuit-breaks** if marketdata isn't live when the playground starts ("md
  subscriber circuit open: N failures; pausing fan-out") and does NOT
  auto-recover → `_book_snap` stays empty → orders can't price → rest/reject.
  (4) Fresh PG needs `seed_accounts` (runs at playground startup) + a risk
  restart to load them. (5) `/api/risk/users/N` mislabels an empty-positions
  result as "no postgres connection". Correct bring-up order: PG → playground
  server → cluster (start-all) → maker → deposits. Fix: a supervised start
  sequence (or make PG a compose service + the playground reconnect its md
  subscriber + not lose the process table on restart).

- **HEALTH-PORT-BAND-OVERLAP-AT-SID-10** (FIXED 2026-07-07) — flagged by the
  deploy authoring (2026-07-06). The `start`/spawn-plan health-port band math
  (`base + sid` / `base + shard`) collided at sid≥10: PENGU sid=10 → health
  `9810` overlapped the risk health base `9810`. Fix: widened the per-tier
  stride from 10 to 100 ports (`BASE_ME_HEALTH=9800`, `BASE_RISK_HEALTH=9900`,
  `BASE_GW_HEALTH=10000`, `BASE_MARK_HEALTH=10100`, `BASE_MD_HEALTH=10200`,
  `BASE_RECORDER_HEALTH=10300` in `./start`) — no `base+sid`/`base+shard`
  offset below 100 can now cross into another tier's band. Verified by
  simulating sid/shard 0..15 across all tiers: zero cross-tier collisions
  (see RISK-REPLICA-HEALTH-INTRATIER-OVERLAP below for a pre-existing,
  separate intra-tier collision this did NOT touch).
- **RISK-ENV-VARS-SET-NOT-READ** (FIXED 2026-07-07) — `RSX_RISK_RESET_FROZEN_
  ON_START` and `RSX_RISK_IS_REPLICA` were set by the `start` spawn plan (and
  `RSX_RISK_IS_REPLICA` also by `rsx-playground/tests/acceptance_test.py`)
  but read nowhere in `rsx-risk/src`. Fix: dropped both from `./start` and the
  acceptance test's risk env dict; also scrubbed `rsx-risk/README.md`'s
  now-false claim that `RSX_RISK_IS_REPLICA=true` selects replica mode
  (role is decided by advisory-lock acquisition at startup, per
  `specs/2/28-risk.md`'s own description of `main()`'s state machine).
- **RISK-REPLICA-HEALTH-INTRATIER-OVERLAP** (LOW, config, newly noticed
  2026-07-07 while verifying the fix above) — inside the risk health band
  itself, the replica offset formula (`BASE_RISK_HEALTH + 10 + shard*2 + r`)
  can collide with a *primary* shard's health port (`BASE_RISK_HEALTH +
  shard`) once `risk_shards` gets into double digits, e.g. shard=10 primary
  == shard=0,r=0 replica (`base+10`). Pre-existing (not introduced by the
  100-port-stride fix above, just carried the same shape forward); latent
  because current scenarios use ≤3-4 risk shards. Not fixed here — separate
  from the requested sid≥10 cross-tier bug, recorded for later triage.

- **FOK-AVAILABLE-LIQUIDITY-ON-SCAN** (MED, bench) — **Status: FIXED
  2026-07-04.** `match_by_type/fok_full` was ~296 µs after the occupancy-
  bitmap fix (`da9a2b4`), unlike every other order type (60-145 ns). Cause:
  the old `available_liquidity` was a SEPARATE O(N-resting-orders) full-book
  scan run on FOK's hot path — it walked every one of the ~100k active
  levels and every order on each, summing crossable qty BEFORE matching.
  Fix: no new structure. FOK is just "try to match it, take it or don't", so
  `can_fill_fully` (`rsx-book/src/matching.rs`) now walks only the *crossing*
  levels in price order — the same traversal a match performs, via the
  book's existing best-level index — summing each level's already-maintained
  `total_qty` and stopping the instant the running total reaches the order
  size. O(levels crossed, early-exit) instead of O(slots + orders); a whole
  level shares one price so `total_qty` counts it exactly (no per-order
  walk). Pinned to brute-force by `tests/fok_liquidity_test.rs` (3000 FOK
  probes over multi-zone random flow). Post-fix `fok_full`: ~118 ns
  (was ~296 µs, −99.95% per Criterion) — now in the same band as every
  other order type. (Number from a lightly-contended box; clean re-run
  pending, but the 300x magnitude is unambiguous.)
- **RECORDER-ARCHIVE-DEV-DISK** (MED, *reframed 2026-07-04*) — the recorder
  archives the FULL ME WAL stream (every order/fill/BBO/done record, verbatim)
  to `tmp/wal/archive/<sid>/<sid>_<date>.wal` as the permanent system-of-record.
  **Unbounded is BY DESIGN** — this is the exchange's audit trail + replay-from-
  genesis tier ("ARCHIVE handles long-term durability", hot tier keeps only 4h).
  Do NOT add retention/GC that deletes records — that destroys the point.
  The actual defect is PLACEMENT: it writes to the local dev root (237 GB) with
  no offload, so continuous maker quoting grew it to 59 GB and ENOSPC'd the box
  (killed a subagent + failed cluster stop/start) twice this session. Fix is
  storage, not deletion: put the archive on a separate/dedicated volume, offload
  to object storage (S3/GCS) with local pruning of already-offloaded segments,
  and/or a DEV-ONLY guard (cap or recorder-off in the playground) so the dev box
  can't fill. Currently mitigated by keeping the recorder stopped in dev.
  Cleared manually (find -delete + kill recorder to release the fd) → 85 G free.
- **BENCH-MOLD-SOUP-UNPINNED** (FIXED 2026-07-07) — flagged as `compare_moldudp64`
  + `compare_soupbintcp` never pinning their threads while casting/raw-UDP/
  KCP/Aeron pin client->core2/echo->core3. Verified already fixed by
  `cad490d`/`aa6ff17`/`e48a36c` (both benches call the same `pick_cores()` +
  `core_affinity::set_for_current` pattern as `compare_all`) — no code change
  needed, entry closed.

- **BENCH-QUINN-ACCEPT-BI** (LOW, *unmasked by the KCP fix*) — with KCP no
  longer aborting the run, `compare_all` now panics at
  `benches/compare_all.rs:356`: `srv_conn.accept_bi().await.unwrap()`. QUIC
  opens a bidirectional stream lazily — the client's `open_bi()` sends nothing
  on the wire until the first `write`, so the server's `accept_bi()` never sees
  the stream (resolves to a connection error / hang). The Quinn row was masked
  by the earlier KCP abort, so this has likely been broken since KCP regressed;
  the README "~37 µs" is last-measured 2026-05-24, not reproducible now. Fix:
  have the client write one priming byte after `open_bi()` before the server
  `accept_bi()`, or restructure the stream handshake. Bench-only. Flagged, not
  patched (separate from the KCP one-liner; QUIC stream-lifecycle surgery).
- **GATEWAY-LATENCY** (MED, *readiness fix landed*) — the cast-recv yield-spin is
  gone: the gateway now awaits io_uring readiness on the CastReceiver fd
  (`946b71d` + `7454187` exposes the fd), event-driven not polled. Earlier
  500µs conn-side egress poll (`5a578d3`) may remain; re-measure with
  `ws_order_latency` on a QUIET box to confirm the win + whether the conn poll
  still shows. Detail below.

**FIXED 2026-07-04** (detail section below): IOC-NOT-HONORED.

**NEW 2026-07-04** (sonnet bug-hunt, all verified genuine — detail at end):
COMPRESSION-ZONE-TICK-UNIT (HIGH, latent), WAL-ROTATE-PREWRITE-MISLABEL (HIGH),
CLI-PTR-READ-UNALIGNED-UB (HIGH), CAST-SEND-RING-TOO-SMALL (MED),
GW-CANCEL-NO-RATELIMIT (MED), GW-CANCEL-NOT-USER-SCOPED (MED),
RISK-DEDUCT-FEE-UNCHECKED (LOW), MARK-PARSE-NEG-ZERO (LOW).

**NEW 2026-07-04** (T4 `.ship/33-TUI-SPEED-TESTS`, `rsx-tui/tests/
e2e_guarantees.rs` fixture debugging — detail at end): VERIFY-WAL-FILLS-
ALWAYS-ZERO (LOW), DEMO-TRADE-SUBMIT-ORDER-404 (MED).

**DEFERRED — book session** (founder: "solve once we're dealing with book"):
BOOK-FAR-PRICE-BUCKETING (`[D]` by-design, no action). Detail below.
**FIXED 2026-07-04** (book session): FOK-RESTS-IN-COMPRESSED-ZONES (new,
HIGH), BOOK-SLAB-FREE-UNGUARDED, BOOK-STALE-HANDLE-REUSE — detail
below. (BOOK-BBO-COMPRESSED-INDEX +
BOOK-SCAN-NEXT-BID-OFFBY were fixed 2026-07-03 — see git/CHANGELOG.)

**BY-DESIGN (no action):** RISK-FUNDING-CROSS-SHARD (global zero-sum not
guaranteed across shards; demo is single-shard), GW-SINGLE-SHARD-NO-ROUTING
(one risk sender, no `user_id % shard_count`; demo limit), ME-REPLAY-SKIPS-
DOWNSTREAM (each consumer recovers independently via its own replay).

---

## IOC-NOT-HONORED — cancelled IOC surfaced to client as "resting" (MED)

**Status: FIXED 2026-07-04.** Root cause was NOT tif propagation (the original
triage guessed that and was wrong). Verified on the live cluster with a runtime
trace: `tif=1` reaches `risk_in` AND `me_in` intact, and the matching engine
**correctly cancels** the residual IOC (`rsx-book/src/matching.rs` residual
branch fires → `OrderDone { reason: REASON_CANCELLED }`). The order does NOT
actually rest in the book. The bug was a code-space collision: the ME wrote
`OrderDoneRecord.final_status = reason` (raw matching reason, `REASON_CANCELLED
= 1`), but per `specs/2/49-webproto.md` `final_status` is a webproto U-frame
status where **1 = RESTING, 2 = CANCELLED**. So a cancelled IOC's `final_status
= 1` was forwarded by the gateway as status 1 → the client saw "resting".
`REASON_FILLED = 0` happened to equal webproto FILLED(0), so fills looked fine
and hid the collision. **Fix:** `rsx-matching/src/wal_integration.rs` now
translates the matching reason → webproto status (`done_final_status`:
CANCELLED→2, FILLED→0) at both OrderDone-build sites; the gateway
`route_order_done` mapping was already spec-correct. Regression:
`rsx-matching` `tests/wal_integration_test.rs::
ioc_cancel_final_status_is_webproto_cancelled`. Live-verified: an empty-book /
non-crossing IOC now reports "cancelled" (fills still "filled", resting GTC
still "resting").

## GATEWAY-LATENCY — casting-recv poll-loop starvation dominates e2e (HIGH, mitigated)

**Status: MITIGATED (full fix deferred).** Single-order stage trace (live
cluster): the response left Risk by ~571µs (`me_out`) but the gateway didn't
receive it (`gateway_cast_recv`) until much later — the response sat in the
gateway UDP socket buffer waiting for the casting-recv poll loop to get a turn
on the shared monoio reactor (WS accept + per-conn handlers + casting-recv all
on one reactor). The egress-drain poll was tightened 10ms→500µs (`5a578d3` +
handler), which dropped WS single-stream p50 from 11.5ms to 2.25ms
(`reports/20260530_e2e-ws-latency.md`). **Remaining fix (deferred):** tile-split
the casting-recv response path to a dedicated pinned busy-spin thread (off the
reactor) → SPSC ring → WS writer tasks (same pattern as Risk/ME). Biggest single
e2e win; deferred per founder ("don't split now").

## Deferred book-session bugs (detail)

Founder: solve these when we next work the book. All verified against source;
`[V]` real, `[?]` needs one more check, `[D]` by-design/known-limitation.

- **FOK-RESTS-IN-COMPRESSED-ZONES (HIGH, latent correctness).** Status: FIXED
  2026-07-04. `can_fill_fully` (`matching.rs`) summed a whole level's
  `total_qty` but tested only the HEAD order's price against the taker limit. In
  a compressed zone (≥1: 10/100/1000 ticks/slot, plus the zone-4 catch-all) a
  level holds orders at DISTINCT raw prices, so it over-counted crossable
  liquidity → FOK feasibility passed → `match_at_level` skipped the non-crossing
  makers → residual > 0 → and the residual branch only cancelled IOC, so the FOK
  fell through to `insert_resting` and RESTED (all-or-nothing violation, with a
  partial fill). Two-part fix: (a) `can_fill_fully` now walks the level's orders
  and sums only qty whose ACTUAL raw price crosses the limit whenever the slot
  is in a compressed zone (`t >= zone_slots[0]`); zone 0 keeps the O(1)
  `total_qty` shortcut (single price per slot, the near-BBO happy path), and
  early-exit is preserved (stop once a whole band sits beyond the limit). (b)
  the residual branch now rejects FOK (`OrderFailed(FAIL_FOK)`, book untouched)
  instead of resting — defense in depth behind the now-exact pre-check
  (`debug_assert!(false)` guards the can't-happen residual). Regression:
  `tests/fok_liquidity_test.rs` (`fok_compressed_zone_insufficient_true_
  liquidity_rejected`, `_tick50_insufficient_rejected`, `_sufficient_liquidity_
  fills`), each fails on the old `matching.rs`.
- **BOOK-SLAB-FREE-UNGUARDED `[V]` (hardening).** Status: FIXED 2026-07-04.
  `slab.free` now `debug_assert`s `idx < bump_next` (never-allocated) and
  `!is_free(idx)` (already on the freelist) — a double-free / freelist cycle is
  caught in debug rather than aliasing a slot handed out twice. O(free)
  is-free walk stays behind `debug_assert` (off in release; the ME bounds open
  orders upstream). Regression: `tests/slab_test.rs::slab_double_free_panics`,
  `slab_free_never_allocated_panics`.
- **BOOK-STALE-HANDLE-REUSE `[?]`.** Status: FIXED 2026-07-04 (defensive).
  Full generational handles would ripple the `u32` handle meaning across
  matching/events/index (too invasive), so instead: added
  `Orderbook::cancel_order_checked(handle, user_id, order_id_hi, order_id_lo)`
  which re-checks the slot's identity before cancelling (returns false, book
  untouched, on a reused/inactive slot), and documented on `cancel_order` the
  exact cross-crate invariant rsx-matching must uphold (verify identity, or use
  the checked path). rsx-matching's user-cancel path already does this drift
  check inline (`main.rs:1002-1017`); the WAL-replay path trusts its own
  `order_index`. Regression: `tests/book_test.rs::
  cancel_order_checked_rejects_stale_handle`.
- **BOOK-FAR-PRICE-BUCKETING `[D]`.** `compression.rs:48,118` — compression
  buckets far prices (10/100/1000 ticks per slot), so distinct prices share a
  level → price-time priority is coarse far from mid. Intentional compressed-book
  tradeoff; logged as a known design risk, not a defect.

### MIGRATE-SKIPS-NEW-MID-LEVEL — order resting at new_mid orphaned on recenter (HIGH)
**Status: FIXED 2026-07-04.** `trigger_recenter` now migrates the `new_mid`
level once, up front (no-op if empty, within-frontier so never migrated twice),
so an order resting exactly at new_mid survives recenter. Covers both the lazy
(`advance_frontier_to`) and eager (`migrate_batch`) paths — new_mid is empty in
`old_levels` by the time either runs. Regression: the adversarial tick-size test
now recenters ONTO a resting level (`new_mid = mid + 505*tick`) and asserts slab
no-leak. Found by the distribution tests. Original triage below.

**[Original — OPEN]** `migration.rs` —
`trigger_recenter` sets `bid_frontier = ask_frontier = new_mid`, then both
`migrate_batch` and `advance_frontier_to` (lazy path) step the frontier AWAY
from `new_mid` BEFORE migrating (bid: `bid_frontier -= tick` then migrate; ask:
`ask_frontier += tick` then migrate). So the OLD level covering exactly
`new_mid` is never visited — any order resting at `new_mid` is left in
`old_levels` and dropped when `complete_migration` clears them. Loss is silent:
one live order vanishes → violates invariant #8 (slab no-leak: allocated != free
+ active) and invariant #4 (position = sum of fills) downstream. Reproduced for
tick_size ∈ {1,10,50}: rest a fat book, recenter to a `new_mid` that coincides
with a resting level, migrate fully → exactly one order missing (len 800, free
40, active 759). Not triggered when `new_mid` falls between levels (the common
case), which is why it stayed latent. Fix: migrate the `new_mid` level once at
`trigger_recenter` (or seed the frontier so the first step includes `new_mid` —
e.g. migrate before decrement/increment). The distribution recenter test
(`rsx-book/tests/distribution_test.rs`) sidesteps it by recentering off a level;
remove that workaround once fixed.

## Dashboard stability + RSX process flapping (2026-05-31, task 29-#12)
- Playground dashboard runs as a SINGLE uvicorn worker (server.py:8127,
  workers=1 reload=False) with no self-watchdog. Any kill = full outage
  until manual `./playground start`. NOTE: several apparent "crashes" this
  session were self-inflicted (`pkill -f server.py` matched the running
  shell; `fuser -k 49171/tcp` deliberate) — the server is more stable than
  it appeared; the real gap is no supervisor.
- RSX processes flap (e.g. 4/7 running): auto-restart supervisor with a
  circuit-breaker opening after 5 crashes -> `blocked` (server.py:344-357).
  Root crash causes: Postgres down (10.0.2.1:5432 unreachable),
  marketdata rcvbuf -> FAULTED, maker-induced cast FAULTED (per MEMORY).
- FIX options (needs decision; edit blocked while overview agent owns
  server.py): (a) run dashboard under systemd/pm2 or a small watchdog;
  (b) bring Postgres up so PG-dependent processes stop crashing;
  (c) `make tune-host` (rmem_max) before enabling the auto-maker to avoid
  the rcvbuf FAULTED loop.

## Control Stop/Start buttons don't work (2026-05-31, task 29)
- Owner reports per-process Stop/Start (control grid + faults page) do
  nothing. Suspect: buttons post ./api/processes/{name}/{action} WITHOUT
  an x-confirm header (only the walkthrough all/start at pages.py:663 sends
  hx-headers x-confirm), while the all/* endpoints require check_confirm.
  Verify the {name}/{action} handler's confirm/run_id gate and either send
  the header from the buttons or drop the gate for single-process actions.
  Must be covered by the audit + Playwright play-tests.

  CORRECTION: the {name}/{action} handler (server.py:3942) needs only
  loopback (no confirm gate), so it's NOT a header issue. Likely cause:
  (a) the raw-PID stop fallback (3958-3972, hit when name not in `managed`
  after a dashboard restart drops the in-memory dict) SIGTERMs but does NOT
  mark intentional/update _restart_state, so the auto-restart watcher
  revives the process; and/or (b) no visible feedback. Fix: route single-
  process stop through the intentional-flag path (like stop_process) even
  on the raw-PID branch, and surface the result. Confirm by clicking Stop
  and watching whether the process reappears.

## Sonnet bug-hunt findings — 2026-07-04 (all verified genuine)

Four read-only sonnet hunters over the whole tree; every finding below was
re-verified against source by the main agent. Recorded, not fixed (bug-triage
protocol — awaiting prioritization).

### COMPRESSION-ZONE-TICK-UNIT — zones mis-sized for tick_size != 1 (HIGH, latent)
**Status: FIXED 2026-07-04 (`9089e50`).** `CompressionMap::new` now stores zone
thresholds as RAW-price distances (matching `price_to_index`'s `price -
mid_price`), and converts to slot counts by dividing by `compression_ticks *
tick_size` — so a 5% move lands in the right zone for any tick_size, not the
2-slot catch-all. Verified: `tick50_five_pct_lands_in_zone0_not_catchall`,
`tick50_zone0_is_one_tick_per_slot`, `tick_size_stored_and_thresholds_raw`,
`adversarial_tick_sizes_matching_and_recenter` (tick 1/10/50 through real
matching + recenter). `migration.rs::should_recenter` checked too. Original
triage below.

**[Original triage — OPEN]** `rsx-book/src/compression.rs:29-45` computes zone thresholds
in TICK units (`pct_5 = mid*5/(100*tick_size)`, comment line 24 "…/ tick_size
ticks"), but `price_to_index` (`compression.rs:84-85`) compares them against a
RAW-price distance (`distance = price - mid_price`, never divided by
tick_size). For `tick_size=1` they coincide (every test/bench uses tick_size=1,
masking it). For `tick_size != 1` (BTC=50, ETH=10 in the symbol config) the
units diverge: a price only 5% from mid lands in the 2-slot zone-4 catch-all →
most of the book collapses into shared price levels → price-time priority
broken, wrong fills. Same mismatch reachable via `migration.rs:16-22`
(`should_recenter`) and recentering. Fix: divide `distance` by `tick_size` (or
store raw-price thresholds). Latent because the demo trades PENGU (tick=1).

### WAL-ROTATE-PREWRITE-MISLABEL — boundary records unreachable for NAK/replay (HIGH)
**Status: OPEN (rsx-cast bugfix candidate).** `rsx-cast/src/wal.rs:220-224` the
pre-write rotation fires before `write_all`, but `rotate()` (line 252-256) names
the old file with `self.last_seq`, which `append_framed` (line 197) already
advanced to include the still-buffered (unwritten) records. So the old file is
labeled `[first_seq..last_seq]` while physically holding only up to the
previously-flushed seq; the buffered records then go to the NEW file whose
`first_seq` is set to `next_seq` (line 282) — above them. Those boundary records
become unreachable via `read_record_at_seq` (NAK retransmit) and
`open_from_seq` (TCP replication catch-up) → both silently return nothing for a
record that exists. Pre-write is the PRIMARY rotation path (same threshold as
post-write, fires first); the existing `write_rotate_read_across_files` test only
hits the correct post-write path (file_size==0 on its single flush). Fix: label
rotate with the last seq actually written to that file, not `self.last_seq`.

### CAST-SEND-RING-TOO-SMALL — NAK fast-path dead for hot 128B records (MED)
**Status: OPEN (rsx-cast bugfix candidate).** `rsx-cast/src/cast.rs:77`
`SEND_RING_FRAME_BYTES=128`, but `FillRecord`/`BboRecord`/`OrderAcceptedRecord`
are 128-byte payloads (rsx-messages asserts) → total 16+128=144 > 128 → the send
path (cast.rs:261/318) takes the "large record" branch that zeroes the ring slot
(never caches). Every NAK for those (the 3 most-sent record types) misses the
in-memory ring and falls to a disk `read_record_at_seq`. The constant's comment
(cast.rs:72-77 "all <= 64 bytes payload … with headroom") is stale/false. Not a
correctness loss (disk fallback still works, modulo the WAL-ROTATE bug) but the
documented fast-path recovery cache is silently inert for the hottest traffic.
Fix: size the ring frame to cover current records (>=144).

### CLI-PTR-READ-UNALIGNED-UB — aligned ptr::read on unaligned WAL buffer (HIGH, soundness)
**Status: OPEN.** `rsx-cli/src/main.rs:343` (and every decode arm: 381,408,439,
468,499,519,537,574,595,619,643,674,700) uses `std::ptr::read(payload.as_ptr()
as *const _)` on `#[repr(C, align(64))]` records read from a `Vec<u8>` payload
(heap alloc, 8–16-byte aligned) — `ptr::read` requires the source satisfy the
type's alignment; violating it is UB. The canonical decoder
`rsx-cast/src/encode_utils.rs:53-55` uses `std::ptr::read_unaligned` for exactly
this reason; the CLI hand-rolled the decode with the wrong primitive. Can
produce garbage fields / crash under non-16-aligning allocators or higher opt
levels, undermining the WAL-inspection tool. Fix: `read_unaligned`.

### GW-CANCEL-NO-RATELIMIT — Cancel bypasses rate limit + circuit breaker (MED)
**Status: OPEN.** `rsx-gateway/src/handler.rs:547-649` the `WsFrame::Cancel` arm
calls none of `ip_limiter`, per-user `RateLimiter`, or `circuit.allow()` — unlike
the `NewOrder` arm (314-364). A client can flood `{"C":[...]}` frames straight to
risk/ME casting with zero throttling, even while throttled/tripped on the order
path. DoS gap on the casting channel. Fix: gate Cancel through the same limiters.

### GW-CANCEL-NOT-USER-SCOPED — cid collision breaks self-cancel (MED)
**Status: OPEN.** `rsx-gateway/src/handler.rs:580,618` `find_by_order_id` /
`find_by_client_order_id` scan the process-global `pending` queue with NO user
filter, and `build_cancel` (584,622) sends the requester's `user_id` + the found
order's id. cid is client-chosen and unnamespaced, so two users can collide:
user A's self-cancel-by-cid can find user B's same-cid order first → ME composite
key `(A, B_oid)` misses → silent no-op → A cannot cancel its own resting order.
Unauthorized cancel of B is blocked by the ME key (incidental), but the missing
user-scope + self-cancel breakage are real. Fix: scope the pending lookup to
`user_id`.

### RISK-DEDUCT-FEE-UNCHECKED — lone non-saturating ledger op (LOW)
**Status: OPEN.** `rsx-risk/src/account.rs:22` `deduct_fee` uses `self.collateral
-= fee` while every other money op in the crate uses saturating/i128-widened
arithmetic. If `collateral` and `fee` sit at opposite i64 extremes it overflows
(debug panic = DoS; release wrap = manufactured solvency). Reaching it needs
absurd values that order-entry notional caps prevent — latent consistency /
defense-in-depth gap, not live-impact. Fix: `saturating_sub`.

### MARK-PARSE-NEG-ZERO — sign lost for "-0.x" price strings (LOW, dead on real feeds)
**Status: OPEN.** `rsx-mark/src/source.rs:275-303` `parse_price("-0.5")`: `whole
= "-0"` parses to `0`, so `whole_val == 0` takes the add branch → `+frac` instead
of `-frac` (sign flip). Dead on real CEX spot feeds (never negative); flagged as
a latent edge only. Fix: track the sign separately from `whole_val`.

### LATENCY-TRACE-STAGES-AGGREGATION — mixed-population median subtraction (MED)
**Status: FIXED 2026-07-04 (`8b33bbc`).** My initial diagnosis was WRONG: I
claimed the return leg (`risk_out`/`gateway_out`) never emits and that taker
completions bypass `route_fill`. An opus trace disproved both — 9/9 spaced taker
FILLS emit `risk_out` (risk main.rs:560) AND `gateway_out` (route.rs:68 via the
real fill path ME→risk RECORD_FILL→forward_to_gw→gateway route_fill), all
anchored on the shared t0. The "zero samples" I saw were: (a) my probes RESTED
(a resting limit / IOC-cancel emits no Fill → correctly no egress sample), and
(b) the **real bug**: `/api/latency-stages` took independent per-stage medians
across a MIXED oid population then subtracted them — cold/rested orders carry
only forward stages (huge cold `me_in`≈5200µs), filled takers carry the return
leg, so subtracting medians from different oid sets yielded a meaningless egress
delta (`risk_out` clamped to 0, `me_out` polluted to ~5ms). Fix (Python only,
server.py `_segment_deltas`/`_cumulative_from_deltas`): each segment's median is
computed over exactly the oids carrying BOTH its endpoints. Sparse capture was
the token-bucket rate limiter (state.rs:69, cap 10 / 1 token per 100ms) dropping
rapid probes before `gateway_in` — space probes ≥1s. **Verified egress**
`me_out→gateway_out` per-oid: `[35,41,43,45,231,1846,2662,3437,6541]µs` —
BIMODAL: fast ~35-45µs (gateway spinning) + ms-scale parked tail (the
POLL_ADD-relevant signal). Caveat: verified by running the exact new computation
against real logs (the live playground couldn't be reloaded without risking the
child trace daemons). **Remaining (deferred, wire change):** non-fill records
(ORDER_INSERTED/DONE/CANCELLED/FAILED) carry only ME-emit `ts_ns`, not an origin
timestamp, so they can't compose into the forward-leg profile without adding an
origin-ts field to rsx-messages + WAL + parser; only the taker-fill leg
(`FillRecord.taker_ts_ns`) composes today. GATEWAY-LATENCY egress is now
measurable for fills; the bimodal tail is the next thing to chase.

## RETURN-PATH-INTERMITTENT-DROP — was a test-fixture bug, not a gateway/risk drop (RESOLVED)

**Status: RESOLVED 2026-07-04.** Root cause was a TEST FIXTURE bug, NOT a
gateway/risk/casting defect. `cluster::seed_book` posted a maker BUY @ 60_000,
but the shared long-lived book (rebuilt from WAL, never reset) already carries
resting asks ~50_000 — so the "maker" CROSSED those asks and filled instead of
resting as a bid. The crossing taker then had no resting bid to hit → no fill →
`wait_for(fills==1)` timed out, which read as a dropped return-leg. Fixed by
seeding the maker BELOW the asks (49_000) with qty matched to the taker so it
rests as the best bid and self-cleans on the fill (`rsx-tui/tests/support/
cluster.rs` + `e2e_orders.rs` price bands all <50_000 + a `LIVE_BOOK` mutex
serialising the two book-sharing fill tests). Verified: `submit_ioc_fills` +
`order_lifecycle_accepted_then_done` PASS against the live cluster. The
mid-investigation "confirmed real, persistent-WS-specific" call was WRONG — the
persistent-vs-transient difference was a coincidence (the playground probe used
a correct maker-sell/taker-buy cross; the e2e used a broken maker-buy).
Residual, NOT this bug: (a) casting is UDP so an occasional order/event
genuinely drops by design (`rsx-matching` FAULTED, "clients re-send dropped
pre-ack orders") — the tests carry a resubmit-once retry; (b) test-infra: the
long-lived shared book means a mid-way-failed run can leave a resting bid that
pollutes the next run's level — matched-qty self-cleaning avoids this in steady
state; a fresh cluster is pollution-free. Original (wrong) triage below.

**[SUPERSEDED — original triage guessed the root cause wrong]** Found writing `.ship/33-TUI-SPEED-TESTS` T3
(`rsx-tui/tests/e2e_orders.rs`) against the live minimal cluster
(gw-0/risk-0/me-pengu, symbol_id=10). Repro: two separate WS connections
(distinct seeded `user_id`s, e.g. 2 and 3), maker posts a resting GTC that
rests fine (`me-pengu.log` `me_in..me_out` completes, gateway relays the
`U` status=1 accept — confirmed working every time), then a second
connection submits a lot-aligned crossing IOC. `me-pengu.log` shows the
crossing order fully processed (`me_in -> me_dedup_done ->
me_wal_accepted_done -> me_match_done -> me_wal_events_done ->
me_index_done -> me_out` all present — ME believes it matched and emitted
Fill+Done), but `risk-0.log` shows only the inbound `risk_in` for that
oid and nothing after (no `risk_out`/`risk_cast_send_done`), and
`gw-0.log` shows only `gateway_in`, no `gateway_cast_recv` — the taker's
own `WsConn` never receives a `Fill` or terminal `U` frame and hangs until
test timeout. Independently reproduced outside the Rust suite with raw
`wscat` sessions against the same live cluster (see repro commands in the
session that filed this entry): a first order on a fresh connection got
no reply within 2s; an identical resubmit (same price, fresh cid, new
connection) was acked within the same short window. Ruled out: not
`InsufficientMargin` (all repros use `_SEED_USERS` with ample collateral),
not tick/lot misalignment (validated multiples of `lot=100000`), not the
gateway's `NewOrder` rate limiter (that path `send_error`s explicitly —
code 1006 "rate limited" — before minting an oid or logging
`gateway_in`; every failing oid here already has a `gateway_in` line, so
it passed rate-limit/circuit checks). Root cause not isolated — candidates
worth checking: WAL dedup replay on a resubmitted cid returning the
original accept silently without re-emitting a cast event; the in-progress
"risk return path RESPEC'd → ME→GW-direct" migration noted in project
memory (partial, not fully implemented) leaving the old ME→Risk→GW leg
half-wired for the fill/done case specifically (accept-path via risk
still works; fill-path does not); or plain casting/UDP loss on the
ME→Risk leg specifically for fills under the concurrent multi-connection
load this test suite generates. `rsx-tui/tests/e2e_orders.rs`'s
`submit_ioc_fills`/`order_lifecycle_accepted_then_done` work around this
with a resubmit-once retry (matching `rsx-matching`'s own documented
mitigation, "clients re-send dropped pre-ack orders (WAL dedup =
exactly-once)"), but even that isn't always enough — both tests can still
fail against this cluster instance. Not a T3 test-file defect; do not
"fix" by weakening the test assertions.

- **Severity:** high
- **Scope:** rsx-risk / rsx-matching / rsx-gateway return-path (ME→Risk→GW)
- **Affected:** fill/done confirmation delivery to the ordering client
- **Source:** `log/me-pengu.log`, `log/risk-0.log`, `log/gw-0.log` around
  2026-07-04T11:08-11:13 (oids `019f2cd110b679719c02d72391586007`,
  `019f2cd4930f7121bea15cf37753ad93`, `019f2cd4c6ad7cd1829930d0494aa843`,
  `019f2cd4da3b7ef3aea4fbf33401b3fa` and others); see also `.ship/
  33-TUI-SPEED-TESTS` session transcript for the raw `wscat` repro
- **Status:** open
- **Fix:** —

## VERIFY-WAL-FILLS-ALWAYS-ZERO — playground /api/verify never sees real WAL fills (LOW)

**Status: OPEN.** `_run_invariant_checks`'s "Fills precede ORDER_DONE (per
order)" check (`server.py:4631-4689`, `_wal_stream_dirs()` scan) reports
`"WAL fills=0 but session fills=183 — sources disagree"` (status `fail`)
even immediately after a real fill was driven through the gateway and
confirmed on the wire (`GwEvent::Fill` observed by an `rsx-tui` `WsConn`
client, T4 `.ship/33-TUI-SPEED-TESTS`). The ME's actual WAL directory is
`RSX_ME_WAL_DIR=./tmp/wal/pengu` (confirmed via `/proc/<me-pid>/environ`
+ `find`, landing at `tmp/wal/pengu/10/10_active.wal`, which does grow on
fills), but the playground's own WAL-dir resolution apparently looks
elsewhere and finds nothing, permanently reporting 0. The "session fills"
counter (Python-local, 183 in the same run) only counts orders submitted
through the playground's own REST endpoints, not real WAL state, so it
isn't a substitute either — net effect, this check is not a usable fill-
durability oracle for orders submitted via any route today.

- **Severity:** low
- **Scope:** rsx-playground/server.py `_run_invariant_checks`
- **Affected:** `/api/verify`, `/api/verify/run-json`, `/verify` page
- **Source:** rsx-playground/server.py:4631-4689; observed via
  `rsx-tui/tests/e2e_guarantees.rs`'s `fill_durability_recorded_in_wal`
  test, which works around it by reading the ME's active WAL file size
  directly instead of this endpoint.
- **Status:** open
- **Fix:** —

## MARKETDATA-SHADOW-BOOK-UNBOUNDED-LEVEL-ALLOC [OPEN]
**Severity:** HIGH (crash-loop, takes cluster non-green)
**Where:** rsx-marketdata shadow book construction/recenter — `shadow.rs:29`
`Orderbook::new(config, capacity, mid_price, ...)` sizes the level array from
`mid_price`+`tick_size` via the compression map. `mid_price` for a symbol's
shadow book is derived from replayed events, and is NOT bounds-checked.
**Symptom:** `memory allocation of 47962384944 bytes failed` (repeating) in
`log/marketdata.log`, right after `replay bootstrap complete` (seq 11385).
48 GB = 1,998,432,706 × 24 B (`PriceLevel`) → a ~2-billion-slot level array.
**Root cause (suspected):** a torn/garbage WAL record (from the earlier
ENOSPC + OOM crashes) decodes to an extreme price during replay; the shadow
book is constructed/recentered around that price, so the compression map
computes ~2 B slots instead of ~120k, and the level-array alloc aborts.
**Immediate fix:** clean-state reset (fresh WAL) so replay has no poisoned
record → marketdata bootstraps at mid=50000 (~120k slots). Unblocks the
cluster; does NOT fix the underlying vulnerability.
**Real fix (defer, record only):** bound the shadow book's derived
mid_price / computed slot count — reject or clamp a replay-derived price that
would size the level array beyond a sane cap (e.g. a few million slots), and
harden replay record decode against torn records (length/price sanity).

## PLAYGROUND-MARKET-ORDER-REJECTED-BY-GATEWAY [OPEN]
**Severity:** MED (a whole order type is unusable from the playground)
**Where:** `rsx-playground/server.py` `api_orders_test` market path
(`order_type == "market"` → `price_int = 0`) → gateway.
**Symptom:** submitting a market order (`order_type=market`, `price=0`) via
`/api/orders/test` returns `rejected: price not tick aligned`. The dashboard's
own tick check is correctly skipped for market orders, so the rejection comes
from the **gateway** (an E frame): it treats `price_raw=0` as non-tick-aligned
rather than as a market marker.
**Repro:** `curl -X POST .../api/orders/test -d 'side=buy&order_type=market&price=0&qty=10&user_id=1'`
→ red "rejected: price not tick aligned" (200). A tick-aligned crossing LIMIT
(e.g. buy 51000 vs asks ~50150) fills fine.
**Impact:** market orders don't work end-to-end; the latency test was switched
to a crossing limit as a workaround (`play_latency.spec.ts`).
**Also observed:** an unaligned LIMIT (buy 50201, tick=50) FILLED — the gateway
does NOT strictly enforce tick alignment for limits, yet rejects price-0 market
orders for it. Inconsistent. Needs a spec decision: does the gateway support
market orders (price 0 / a market flag), or is the exchange limit-only?
**Fix (defer, record only):** either (a) gateway accepts a market order marker
(price 0 or an explicit type) and matches at best-opposite, or (b) the
playground sends a crossing tick-aligned limit for "market" and drops the
price-0 path. Decide the product answer first.

## PLAYGROUND-DASHBOARD-STALE-PID-RESTART-RACE [OPEN]
**Severity:** LOW (dev ergonomics)
**Where:** `rsx-playground/playground` restart vs a dashboard started outside
the wrapper (no PID file).
**Symptom:** if a dashboard is launched directly (not via `./playground
start`), `./playground restart` can't see it (no PID file) and aborts on a
stale-PID race; subs had to `kill` by PID and relaunch via the wrapper.
**Fix (defer, record only):** have restart fall back to port-owner lookup
(`ss -ltnp` on 49171) when the PID file is missing/stale.

## GATEWAY-CRASH-UNDER-WS-CHURN (F20) [RESOLVED]
**Severity:** MED (gateway stability under load; cascades into other tests)
**Where:** rsx-gateway, exercised by `play_readiness.spec.ts:189` "gw-0 survives
WS connection churn (F20)".
**Root cause (verified from `log/gw-0.log`):** NOT a connection-handling bug.
The gateway's per-connection path is robust — a direct churn driver ran ~30k
full authed connects + ~20k mid-handshake aborts across distinct users; gw-0
never died, fd peaked at 129 and returned to baseline 9 (no leak on any
disconnect path, including mid-handshake abort and broken-pipe writes). The ONLY
crash signature ever recorded is `main.rs:269` — the cast-receiver UDP rebind
failing with `AddrInUse` (94 occurrences, all one restart storm). Chain: a
supervisor restart overlaps the prior gw-0's `RSX_GW_CAST_ADDR` (:9300) socket
release → fresh gw-0 hits `AddrInUse` on `CastReceiver::new` → retried only
30×200ms=6s then **panicked** → `install_panic_handler` `exit(1)` → supervisor
respawns → re-races the port → tight ~6s crash-storm the test catches as "gw-0
not running during churn" (and pid-churn cascades into the next test).
**Fix:** the overlap trigger was fixed in the supervisor (`f1f2d11`,
start_all idempotency: derive port clear-set from spawn plan incl. 9300/98xx,
poll-for-port-free, detach-before-pkill, reap orphans — verified zero AddrInUse).
Gateway-side hardening (this change): the cast rebind retry budget went 30→100
(6s→20s) with a clearer terminal message, so a transient handover self-heals
instead of hard-crash-storming; a genuinely permanent conflict (two gateways on
one port) still fails fast. See `rsx-gateway/src/main.rs` (`CAST_REBIND_RETRIES`).
**Verified:** churn test PASSES (`play_readiness.spec.ts --grep churn` → 18
passed, gw-0 pid stable through the 90s churn); new binary boots clean (binds
:9300/:8080/:9820 first try); rebind path exercised in isolation reaches
`retry 40/100` with zero panic (old binary died at `retry 30/30` → main.rs:269).
**Also (still open, separate):** `--grep-invert "@long"` did NOT exclude the
@long readiness soak/churn tests in a `bunx playwright test <files>` run — the
tag filter isn't scoping as expected. And make the `play_overview` button test
establish its own clean baseline (Stop All → 0 first) so it can't cascade-fail.

## RECORDER-DEAD-BUT-HEALTHY (durability + false health) [OPEN]
**Severity:** HIGH (a durability demo whose durability is silently broken)
**Where:** rsx-recorder + rsx-health; found by the 2026-07-05 playground audit
(`.ship/40-PLAYGROUND-AUDIT/FINDINGS.md` #1).
**Symptom:** recorder process table / topology / `/component/recorder` / recovery
feed all report **running / healthy / "WAL files found"**, but the recorder log
ends with `BLOCKED: 21 consecutive stream errors exhausted retry budget (20): No
such file or directory` — ME replication (`127.0.0.1:9710`) can't serve old seq
(`56844…58462`), recorder fell behind retention and gave up. ~29 min silent
while every health surface said "fine". Archival replication is dead + invisible.
**Fix (defer, record only):** (1) health must reflect **replication liveness**
(last-consumed-seq advancing), not just pid-alive; a recorder that's BLOCKED must
surface as degraded/red. (2) recorder must **catch up from cold WAL random-access**
when it starts behind the hot retention horizon, instead of exhausting a 20-retry
budget on the live stream. Part of Phase-2 recorder/marketdata → cast quality.

## MKTDATA-DROPS-SHADOW-BOOK-DIVERGENCE [OPEN]
**Severity:** MED-HIGH (real correctness divergence, surfaced by Verify)
**Where:** rsx-marketdata; audit #3/#26.
**Symptom:** Verify FAILs `WAL self-consistency (shadow vs WAL BBO) 1/1 mismatch`;
marketdata logs continuous `WRN seq gap sym=10 expected=N got=N+1` (dropped
casts) and me-pengu `flush took 10-14ms` (>10ms target). The shadow book is
missing events it dropped, so its BBO diverges from the WAL-derived BBO.
**Fix (defer):** mktdata rcvbuf/keep-up (drain the ME casting firehose without
RcvbufErrors); investigate the flush latency; surface a drops/gaps metric.

## MARK-PRODUCES-NO-INDEX / RISK-PHANTOM-POSITIONS / ARCHIVE-WAL-BALLOON [OPEN]
**Severity:** MED (grouped audit findings — see FINDINGS.md #18, #10, #28, #29)
- **#18 mark:** connects Binance but never writes/produces an index — Verify
  SKIPs "no index (mark down)", mark WAL 0 bytes, Risk INDEX 0, while mark shows
  "running" everywhere. Make mark produce/persist, or make health say "no index yet".
- **#10 risk phantom positions:** Risk Lookup shows a large long + PnL for user 1
  with **zero backing fills** (WAL FILL = "no WAL events yet") — stale persisted
  position data. One source of truth; clear/reconcile persisted positions on Reset.
- **#28 archive WAL balloon:** 6.9→10.2 GB in ~7 min from maker quote churn with
  no crosses — drives the Dump OOM + slow flushes + fills disk. Need archive
  retention/rotation so a demo doesn't accrue GB/min.
- **#29 high auto-restart counts** on a "healthy" cluster (gw-0 11, others 7) —
  surface as instability rather than buried in the recovery feed.
**Fix (defer, record only):** these are the Phase-2 recorder/marketdata/gateway
→ cast-quality work; fix carefully, not as dashboard patches.

## CAST-RTT-BENCH-HANGS-AFTER-SEND-REMOVAL [ROOT-CAUSED — duplicate of CAST-RTT-BENCH-DEADLOCKS-ON-LOSS]
**Severity:** MED (bench-only; the shipped library is fine).
**Where:** `rsx-cast/benches/cast_rtt_bench.rs:196-203`.
**Root cause (2026-07-06 read-only audit):** the original hypothesis here
(a `send<T>`-removal regression) is WRONG — `send_framed` populates the NAK
ring byte-identically to the removed path (verified against `git show
bb6c1a0`). The real cause: side A's reply-wait is an unbounded
`loop { if Data break; spin_loop() }` with no timeout, no `tick()`, no
`recv_control()`. One dropped A→B loopback datagram under Criterion's
high-rate warmup → B never echoes → A blocks forever, emitting no heartbeat
and serving no NAK retransmit → permanent two-thread spin-deadlock (matches
the observed ~176% CPU at "Warming up"). Same finding as the other session's
CAST-RTT-BENCH-DEADLOCKS-ON-LOSS (commit 01754ef). Fix belongs in the BENCH
(bound the wait; pump `tick`+`recv_control` inside it), not the frozen lib.
Also noted (LOW, frozen — record only): stale flow-control comment at
cast_rtt_bench.rs:145-149; dangling `CastReceiver::poll` doc links at
cast.rs:510,993-994,1096 (method is `try_recv_with`); `read_record_at_seq`
re-export has 0 external callers; manual header+payload framing duplicates
`encode_record` at replication_client.rs:242-248, replication_server.rs:193-197
and :250-254.
**Fix (defer, record only):** re-run after the in-flight cast.rs refactor
settles; if it still hangs, bisect `cast_rtt_bench.rs`'s client/server ping
loop against pre-`bb6c1a0` to find where the reply stops arriving. Do not
