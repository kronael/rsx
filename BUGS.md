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

## rsx-playground

- **DEMO-SCENE-PREP-VICTIM-SHORT-CANNOT-FILL** (MED, demo) —
  `demo_scenario.py prep` sells `VICTIM_COINS` (300,000 PENGU, 3,000,000,000
  raw) into the book and raises "victim short only filled 5000000/3000000000".
  The scene writes `qty: 20000` and `levels: 5` into `tmp/maker-config.json`,
  but the Go maker reads only `mids` and `depth_profile`
  (`rsx-maker/maker.go`, `readConfigOverrides`), so each level is 1,000,000 raw and
  the five levels hold 5,000,000. The victim position, and with it the
  liquidation the scene exists to show, cannot form. Found live in step 21
  (`prep` then `run` against `minimal`); the `run` stage works. **Fix:** size
  the victim and winner to the book the Go maker quotes, or give the maker
  level size as a config key. Not fixed.
- **PG-ACCEPTANCE-TEST-STACK-STALE** (LOW, test) —
  `rsx-playground/tests/acceptance_test.py` is in no make target and its own
  stack no longer works. It defaults to a Postgres at `10.0.2.1`, seeds
  `accounts` before any migration exists, and starts risk without a mark feed.
  Against a scratch database with the schema applied by `rsx-cli account seed`
  and orders sized to the Postgres config (lot 100000), every order answers
  `timeout`: the gateway accepts it and no update comes back. The step 21
  conversion to `rsx-cli order place|cancel` is therefore unrun. **Fix:** boot
  the stack through `rsx-cli cluster` with a scenario, or delete the file
  (`make e2e` covers the same round trips). Found live; not fixed.
- **PG-TEST-CLEAN-TMP-WIPES-THE-REPO-TMP** (MED, test) — the `clean_tmp`
  fixture (`rsx-playground/tests/api_orders_test.py`) is `shutil.rmtree(TMP)`
  where `TMP` is `server.TMP`, the repo's real `tmp/`.
  `test_stress_low_10_orders_per_sec_60s` and `test_stress_creates_wal_files`
  use it, so every `make _gate-api` deletes
  `tmp/` whole: the WAL, `tmp/pids`, `tmp/cli-pids`, stress reports, and any log
  written there (a gate log redirected to `tmp/` vanished mid-run in step 20,
  twice). **Fix:** point the fixture at a `tmp_path` and patch `server.TMP`,
  or delete the two tests' dependency on it. Found live; not fixed.
- **CLUSTER-PLAN-PORTS-ARE-A-HEURISTIC** (LOW, correctness) — `Process::ports`
  (`rsx-cli/src/plan.rs`) lists every bare `host:port` in a process's env: the
  addresses it sends to as well as the ones it binds, and not a port a daemon
  binds from its own default (marketdata binds 9510 without naming it; me-pengu
  names it as its destination). A port check on a partial start therefore
  misfires, so `cluster up` checks the plan's ports on a cold start only and a
  partial start relies on the daemon's failed bind exiting it during startup.
  **Fix:** name the ports a daemon binds in the plan (env key by key) and check
  those. Found in step 12 restarting `me-pengu`.
- **PG-HEALTH-RED-SPEC-VACUOUS** (LOW, test) — `health_drops_red_when_verify_fails`
  (`rsx-playground/tests/play_health_truthful.spec.ts`) reads
  `GET /api/verify`, which has no route, and returns when the response is not
  ok, so it never reaches its assertion. **Fix:** read `POST
  /api/verify/run-json`; `/x/health` is cached 1 s (`_cached_for`), so the
  assertion needs a wait or a fresh read. Found in the verify glue step; not
  fixed.
- **PG-FORMAT-NOTIONAL-SCALE** (LOW-MED, correctness) —
  `pages.format_notional` divides a ledger value by `USD_SCALE = 1e9`
  (`rsx-playground/pages.py`), but risk's notional unit is 10^10 per USD
  (`NOTIONAL_DECIMALS`, `rsx-risk/src/config.rs`), so the Risk page shows
  collateral, equity, IM, MM and uPnL ten times too large (the seeded 1e17 raw
  reads $100,000,000, not $10,000,000). `users_page` divides collateral by
  1e8, a third scale. **Fix:** one scale constant equal to
  `10^NOTIONAL_DECIMALS`. Measured:
  `tests/api_risk_test.py::test_known_defect_format_notional_uses_the_risk_unit`.
  Found in the step 18 live check.
- **PG-LIQUIDATION-PANEL-READS-SLIP-BPS** (LOW, correctness) —
  `render_risk_overview` and `render_liquidations_wal` (`rsx-playground/pages.py`)
  read `slip_bps`, the WAL record's field, but a row of risk's `liquidations`
  table (what `risk overview` returns) carries `slippage_bps`, so the Slip
  column of the Liquidation Queue always shows 0 bps for a Postgres row.
  **Fix:** read both names or have the source rows use one. Measured:
  `tests/api_risk_test.py::test_known_defect_liquidation_panel_shows_the_slippage_of_a_row`.

- **PG-STRESS-RUN-NOT-ADMIN-GATED** (LOW, security) — `POST /api/stress/run`
  (`rsx-playground/server.py`) starts a load run of real orders with no
  `_require_admin_request`, while `/api/stress/start` and every order route
  are gated (`rsx-playground/CLAUDE.md`: every order route is admin-gated). The
  `playground stress` command and the Playwright down-contract specs call it
  without a token. **Fix:** gate it and send the token from both callers.
  CONFIRMED by reading the route; not fixed (the callers change with it).

- **PG-STRESS-LOAD-LEAVES-CROSSING-ORDERS-UNFILLED** (LOW, to investigate) —
  a live `rsx-cli stress` load through the playground at 50 orders/s for 5 s
  on the minimal scenario (box load average about 4) reported 250 accepted,
  235 filled, and so `status` failed with "accepted crossing orders did not
  all fill". The maker's top levels hold 1,000,000 raw qty each; the cause is
  not established (book depth moving under the run, or IOC orders priced
  `--cross-bps` past a book the maker was re-quoting). **Fix:** rerun on a
  quiet box; if it repeats, log the unfilled orders' prices against the
  marketdata BBO. Seen once, 2026-10-27.

- **PG-GATE-SCENARIO-SWITCH-LEAVES-UNTRACKED-DAEMONS** (MED, flake) — in one
  `make gate` (Gate 4, box load average 11 to 18, 920 MB available) the
  `play_control.spec.ts:41` scenario switch ended with "restart failed: ports
  already in use: 8088, 8180, 9110, ...", every later order spec answered
  "gateway (gw-0) not available", and after the run `cluster ps` was empty
  while a gateway (started 17:05) and five more daemons (17:07) were still
  alive and holding the ports; they had to be signalled by PID. Seen once; the
  server log of the run was wiped by the gate. Inferred, not measured: the
  state files under `tmp/cluster/` were deleted while the daemons ran, so
  `down` found nothing to stop. **Fix:** on a second occurrence keep the
  server log and `tmp/cluster/` and find who removes the pid files. 18 Gate 4
  failures followed (420 of 448 passed).

- **PG-SPEC-LOG-PILLS-ROW-ROUNDTRIPS** (LOW, test) — `play_controls.spec.ts`
  "Log source pills isolate their source" reads the source column with one
  locator call per `<tr>` (`sources()`), about 400 CDP round trips for the 200
  rows a full log view holds. Measured on the loaded box (load average 17):
  11.6 ms per round trip, `sources()` 5.3 to 8.1 s against the 5 s
  `expect.poll` timeout; one `page.$$eval` over the same rows takes 70 ms. The
  spec failed alone on a cluster whose logs were full and passed 3 of 3 after
  `playground reset` (few rows). **Fix:** read the column in one `$$eval`; the
  assertions stay. CONFIRMED by measurement 2026-10-26; not fixed.

- **PG-SAFETY-GATEWAY-CRASH-SOCKET-HANGUP** (LOW, flake) — one `make gate`
  failed `play_safety.spec.ts:177` with `apiRequestContext.post: socket hang
  up` on `POST /api/processes/gw-0/start` in `afterEach`, which skipped the 7
  serial tests after it; the same 8 tests passed alone. The server log of the
  run was wiped by the gate. Inferred, not measured: the request reused a
  keep-alive connection idle for about 5 s, uvicorn's default
  `timeout_keep_alive`. **Fix:** confirm on a second occurrence (the server
  log must be kept), then raise the server's keep-alive above the spec's
  longest idle wait.
- **PG-MAKER-STATUS-FILE-NEVER-WRITTEN** (MED, correctness) — the Python maker
  wrote `tmp/maker-status.json`; the Go `rsx-maker` writes no file (`grep -n
  WriteFile rsx-maker/*.go` finds only a test), so `server._read_maker_stats`
  is always `{}`. The Maker page, `/api/maker/status` (`levels` 0 with the
  maker quoting five a side), `/x/maker-status`, the topology Maker card and
  `/api/status` show zero levels and orders and no mid prices;
  `docs/troubleshooting.md` still says to read the file. Found live in step 19:
  the book had five levels a side while `levels` read 0. **Fix:** needs a
  decision: the maker writes a status file (Go change), or the server derives
  levels from the book it already reads and drops the file.
- **PG-API-BBO-EMPTY-BOOK-IS-200-ZEROS** (LOW, correctness) — with an empty
  book `/api/bbo/{symbol_id}` answers 200 with all-zero fields and `source:
  "wal"` (the WAL's last BBO record is the zero one), so the 404 "no bbo for
  symbol" branch only fires for a symbol the WAL has never seen. **Fix:**
  answer 404 when both prices are 0. Found in step 19.

## rsx-cli

- **CLI-DUMP-AND-VERIFY-REPLAY-SKIP-THE-ARCHIVE** (LOW, correctness) —
  `wal ls`, `wal verify` and `wal dump --follow` list the recorder's
  `{stream_id}_{YYYY-MM-DD}.wal` files (`inspect::list_segments`), but
  `wal dump` and the replay and ledger scan of `verify` open `rsx_cast::WalReader`
  (`wal.rs` `open_reader`, `verify.rs`), which lists only the WAL writer's
  names, so on `tmp/wal/archive` they read zero records. **Fix:** read through
  `SegmentReader` over `inspect::list_segments` (rsx-cast is frozen). Found in
  step 14.

- **STRESS-REPLIES-CARRY-NO-CID** (LOW, protocol) — `N` replies (`U`, `F`,
  `E`) name the server order id, not the client order id, so `rsx-cli stress`
  binds the first frame naming a new oid to the oldest unanswered order of its
  connection (`stress_tally.rs`, `Tracker`). An `E` from the gateway's own
  validation can overtake orders still inside risk or matching, and then
  takes the wrong order's send time; a load mixing rejects with accepted
  orders can mis-attribute individual latencies (the counts stay right).
  Load and probe orders are all valid, and `reject` is closed-loop, so
  neither mode is affected. **Fix:** echo the cid in every reply (wire
  change; needs sign-off).
  CONFIRMED by reading the gateway frames.
- **SPEC-3-CLI-DESCRIBES-THREE-COMMANDS** (LOW, docs) — `specs/2/3-cli.md` is
  "shipped" and calls `rsx-cli` an offline WAL inspection tool with `wal dump`,
  `wal stats` and `wal file`; the CLI also runs `wal ls|verify|types`,
  `verify`, `md`, `token`, `order`, `stress`, `cluster`, `fault`, `config`,
  `account` and `risk`, and `specs/2/index.md` row 3 repeats the old title.
  **Fix:** refine the spec to the command tree in `rsx-cli/README.md`; spec
  text is the founder's to sign off. Found in step 22.
- **CLI-MAIN-HOLDS-THE-WHOLE-CLAP-TREE** (LOW, structure) — `rsx-cli/src/main.rs`
  is 1206 lines: the clap tree (about 750) and the `run_<group>` dispatch. The
  plan's step 1 target was under 200 lines, and the root rule is a thin
  `main.rs`. It holds no command logic, so the crate `CLAUDE.md` keeps "thin"
  to mean flags and dispatch only. **Fix (proposal, needs sign-off):** move the
  clap types to a `cli.rs` and the `run_` functions to `dispatch.rs`, leaving
  `main` under 60 lines. Found in step 22.
- **DOCS-TILES-AND-PINNING-LINK-BROKEN** (LOW, docs) — `rsx-matching`,
  `rsx-marketdata`, `rsx-recorder`, `rsx-mark` and `rsx-risk`
  `ARCHITECTURE.md` link `../docs/concepts/tiles-and-pinning.md`; the file is
  `docs/concepts/02-tiles-and-pinning.md`. **Fix:** repoint the five links
  (`rsx-cli` is done). Found in step 22.

## rsx-matching

Test pins for the entries below are written with each fix, not before:
`.ship/53-COMPONENT-COMPLETION/matching.md` § 5 names the step and test file
per entry.

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
  gateway or marketdata. `specs/2/47-validation-edge-cases.md` § 3.4
  says REJECT, while `specs/2/18-messages.md` § CANCEL states today's silent
  miss; `OrderFailureReason`
  has no such value (`rsx-book/src/event.rs:17-22`). needs sign-off.
  **Options:** (a) add a `NotFound` reason (wire + book) and emit ORDER_FAILED
  through the duplicate-reject path, which needs a ruling on what Risk
  (frozen-margin release) and the Gateway (`route_order_failed`,
  `route.rs:201-216`) do with an ORDER_FAILED for an order they may already
  have closed (invariant #2, exactly one completion); (b) keep the ME silent
  and correct 47 § 3.4 to say the gateway answers cancel misses from its
  pending map (18 § CANCEL already states the silent miss). **Default if
  nothing is decided:** (b), the current behaviour,
  pinned by the plan step 8 test.
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
  (`rsx-messages/src/frames.rs:167-183`, `:307`),
  `specs/2/47-validation-edge-cases.md` § 1.8 assigns the check to
  the Gateway, and the
  repo's trust-boundary rule makes the Gateway the one owner of wire parsing.
  The decode is total (no panic, no UB), so the § 1.4 "May panic or UB" row
  does not apply to the ME. The § 1.4 snippet that returns `INVALID_TIF` from
  the matcher is the stale part (doc fix, plan step 11).

- **TERM-FAILED-REASON-NOT-DECODED** — now glass requirement GW-22,
  `rsx-glass/specs/12-contract-gateway.md`.
- **TERM-SELF-TRADE-FILL-FOLD** — now glass requirement GW-20,
  `rsx-glass/specs/12-contract-gateway.md`.
- **PLAYGROUND-E2E-TESTS-RAW-UNITS-TRIP-OVERFLOW-GUARD** (LOW, test) —
  `api_e2e_test.py::test_api_orders_test_with_form_data`,
  `::test_order_flow_test_then_recent` and
  `::test_test_order_appears_in_recent_orders` post `price=50000 qty=100000`
  to `/api/orders/test`; the route answers "notional too large: price x qty
  overflows i64 ... enter human units", so the asserted words never appear. Fail the same at
  `5f7e33df`; `api_e2e_test.py` is not in Gate 3.
  **Fix:** post human-unit values (price 0.05, qty 10) in those three tests.

Found while checking `specs/2/28-risk.md`, `13-liquidator.md`,
`42-testing-risk.md` and `38-testing-liquidator.md` against the crate.
Each needs a design beyond what the spec states, so it is recorded as a
proposal rather than built.

- **RISK-FUNDING-CONFIG-NOT-PER-SYMBOL** (MED, spec gap) —
  `symbol_config_schedule` carries `funding_interval_sec`,
  `funding_rate_min_bps` and `funding_rate_max_bps` per symbol and
  28-risk.md §5 says "Interval: 8h, configurable per symbol", but the crate
  settles every symbol on one global `FundingConfig` (28800 s, ±100 bps) and
  `load_symbol_configs` does not read those columns. **Fix (proposal, needs
  sign-off):** per-symbol `FundingConfig` inside `SymbolRiskConfig` and
  asymmetric min/max clamping in `calculate_rate`. The persisted
  last-settled interval is already per symbol (`funding_intervals`), but it
  stores an `interval_id` in units of the one global interval length, so a
  per-symbol length also needs the length stored beside the id and a
  conversion when it changes.
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
- **RISK-LIQUIDATION-NOTICE-NOT-SENT** (LOW, spec gap) — the gateway forwards
  a `Q` frame for `RECORD_LIQUIDATION` (`rsx-gateway/src/cast_recv.rs:65-67`,
  `route.rs:170-184`; 37 G14, 49 `Q`), but `rsx-risk` never sends that record:
  its `LiquidationRecord` is the Postgres persist struct
  (`rsx-risk/src/persist.rs:80`, built at `insurance.rs:67`) and
  `RECORD_LIQUIDATION` appears nowhere in `rsx-risk/src` (the other uses are
  the gateway decoder and `rsx-cli`). A liquidated user never receives `Q`.
  **Fix (proposal, needs sign-off):** risk emits one
  `rsx_messages::LiquidationRecord` per round through `forward_to_gw`
  (`rsx-risk/src/rings.rs:42`).
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

- **GW-U-CANCEL-FROM-DONE-HAS-NO-REASON** (LOW, wire) — a status-2 `U` sent
  for ORDER_DONE(CANCELLED) carries reason 0, which reads as USER_CANCEL; a
  client can only tell it from an ORDER_CANCELLED `U` by heuristics (rsx-cli
  checks non-zero stamps). `OrderDoneRecord` has no reason field. Proposal:
  forward the ORDER_CANCELLED reason only, or add the reason to the done
  record (wire change; needs sign-off).

## rsx-gateway

Plan and step numbers: `.ship/53-COMPONENT-COMPLETION/gateway.md` § 5.
**fix-now** = the spec already decides it. **proposal** = needs sign-off before
it is built. **owner decision** = the owner owes a call. A fix-now entry is
pinned by the connection-handler harness of plan step 1 (no test calls
`handle_connection` today) and lands with its test. Line numbers are at
`e57914e1`.

### fix-now

- **GW-CID-DUPLICATE-ANSWER-READS-AS-FAILED** (HIGH, correctness) — a repeated
  cid is answered `U[<original oid>,3,…,3]`; a client that lost the first `U`
  (socket drop) reads its live order as failed, never learns the oid, and
  folds later fills as unknown. Design: `specs/2/61-cid-idempotency.md`
  (draft; answer as `E[1011/1012]`). Built after founder decisions D1–D4.
- **GW-TIMED-OUT-RESTING-ORDER-UNCANCELLABLE** (MED, correctness) — the 10 s
  sweep removes a submitted order and answers TIMEOUT; a late ORDER_INSERTED
  does not restore it, so cancel answers 1005 while the order rests. Spec 61.
- **GW-SHUTDOWN-NO-DRAIN-DOUBLES-RETRIES** (MED) — shutdown exits without
  draining `pending`; clients resend into a fresh gateway with an empty cid
  map. Spec 61 (drain + store epoch).
- **GW-BENCH-NO-P99** (LOW, perf) — the repo rule is p50 and p99 in every
  latency table, gated. `rsx-gateway/benches/gw_tail_bench.rs` reports p50 and
  p99 from one run for the order path (`order_accept`) and the route path
  (`route_inserted`, `route_done`, `route_fill`) into `target/tail/rsx-gateway.json`,
  and `bench-baseline.json` holds its eight `gw_tail/*` keys. Those keys are
  twice one run on a shared 2-core box at load average 10 (CPU pressure 40-80%),
  a placeholder that cannot fail on noise; the route rows there read ~4 ms p50
  against 21 us for `order_accept`, with the stamp the reactor puts on the
  frame already ~3.9 ms after the send and the bench thread's `schedstat` at
  11.9 s on the run queue to 0.5 s on CPU (inference: the busy-spinning
  `gw-cast-recv` thread waits for time slices), so they measure the scheduler. What is
  still open: (1) re-seed the `gw_tail/*` keys from a quiet-box run (RSX is
  measured once everything is done); (2) the Criterion gateway benches
  (`gateway_bench`, `ws_parse_bench`, `ws_encode_bench`, `jwt_validate_bench`)
  report means and have no key in `bench-baseline.json`, so `scripts/bench-gate.sh`
  prints NEW and never fails them; (3) `ws_order_latency` prints p50/p99 but
  needs a live cluster and is a `[[bench]]` target (`Cargo.toml`), so
  `cargo bench --workspace` runs it although its header says hand-run
  (`ws_order_latency.rs`); mark it `bench = false`; (4) the dated report
  `reports/YYYYMMDD_gateway-tail.md` and the crate README's pointer to it (the
  README names only `ws_order_latency` today). Plan step 11. No test — perf.

### proposals (need sign-off)

- **GW-CANCEL-NO-RATELIMIT** (MED, resource/DoS) — the `C` arm
  (`handler.rs:241-277`) calls none of `ip_limiter_for`, the per-user
  `RateLimiter` or `circuit.allow()`, unlike `N` (`admit_order`,
  `handler.rs:293-332`): a
  client can flood `C` frames to risk and the ME at no cost, even while
  throttled on `N`. 11 § Rate Limits limits "orders" and § Overload / Circuit
  Breaker gates "new orders" on the circuit; whether a cancel spends a token
  is unstated.
  **Options:** (a) cancels spend the per-user and per-IP tokens and the
  circuit never gates a cancel; (b) cancels stay unthrottled. **Default if
  nothing is decided:** (a) is the recommendation; until signed off a cancel
  stays unthrottled.
- **GW-CANCEL-DOUBLE-TERMINAL** (MED, correctness) — a user cancel makes the
  ME emit ORDER_CANCELLED and then ORDER_DONE(CANCELLED)
  (`rsx-matching/src/cancel.rs:74, 82`), risk forwards both
  (`rsx-risk/src/cast_loop.rs:257, 267`) and the gateway turns each into a
  status-2 `U` (`route.rs:156-168`, `:130-154`), against 18 § Completion
  Guarantee "exactly
  one completion". `stream_ordering_test::exactly_one_completion_per_order`
  covers fill + done only. A post-only cancel emits ORDER_CANCELLED alone
  (`rsx-book/src/matching.rs:103-111`), so the gateway cannot simply drop one.
  **Options:** (a) the ME stops emitting ORDER_CANCELLED before ORDER_DONE on a
  user cancel; (b) the gateway sends a status-2 `U` for ORDER_CANCELLED only
  when its reason is not USER_CANCEL. **Default if nothing is decided:** (a),
  decided in the matching follow-up.
- **GW-RISK-READINESS-GATE** (MED, correctness) — 20 § Startup Ordering says
  reject with OVERLOADED until the risk stream is up and CaughtUp; GUARANTEES
  § 5.1 says a partition is detected in 1 s and every pending order fails
  `NETWORK_ERROR` (29 § Network Errors records what the gateway does
  instead). The gateway has no liveness signal: `/ready` reports
  pending saturation only (`metrics.rs:115`), and `CastReceiver` consumes risk
  heartbeats internally and exposes only `try_recv_with`,
  `reset_after_replay`, `expected_seq` (`rsx-cast/src/cast.rs`). **Options:**
  (a) an rsx-cast liveness API (rsx-cast is frozen; needs sign-off);
  (b) a gateway-level probe record on the existing stream. **Default if
  nothing is decided:** no gate; `/ready` reports pending saturation only.
- **GW-RISK-STREAM-NOT-SERVED** (MED, correctness) — nothing serves the
  risk -> gateway stream: `rsx-risk` embeds no `ReplicationService` (`grep
  ReplicationService rsx-risk/src` -> nothing; `rsx-risk/src/rings.rs:37-38`
  even says the gateway never replays from risk). Without
  `RSX_RISK_REPLICATION_ADDR` the gap is skipped (`cast_recv.rs:154-159`); with
  it set, the connect fails, `replay_gap` warns and skips the gap
  (`cast_recv.rs:123-128`) and the gateway keeps running. **Options:** (a) risk
  embeds a `ReplicationService` over its gateway-stream WAL; the gateway half
  is built (`replay_gap` pushes each replayed record on the live ring,
  `tests/handler_replay_test.rs`); (b) drop the gateway drain and the
  variable, count skipped gaps, and say in GUARANTEES that a client can miss
  a notification after a risk -> gateway gap. **Default if nothing is
  decided:** (a). A drain that fails after pushing records resumes at the gap
  end (a RECONNECT has no gap: the last delivered seq), so a partial RECONNECT
  drain can deliver the records it already pushed a second time; `drain_replay`
  returns no tip on error. Read from the code, not run.
- **GW-MULTI-GATEWAY** (MED, design) — risk sends every response to one
  `RSX_GW_CAST_ADDR` (`rsx-risk/src/config.rs:376`, `cast_loop.rs:116`), but
  the playground `stress` and `M3S` presets start two gateways
  (`rsx-playground/runtime.py:64-65`, each with its own cast address); orders
  on the second gateway get no response (inference from the code, not run).
  20 § Scaling Strategy and 9-deploy describe several gateways. Related
  by-design
  ruling: GW-SINGLE-SHARD-NO-ROUTING. **Options:** (a) per-gateway senders in
  risk and a user -> gateway map; (b) one gateway per risk, the two-gateway
  presets removed and the specs say so. **Default if nothing is decided:**
  unchanged (one gateway peer per risk).
- **GW-REST-READ-ENDPOINTS** (LOW, design) — 26 specifies `/v1/account`,
  `positions`, `orders`, `fills`, `funding` with JWT and 5/s user, 50/s IP
  limits; the gateway serves only `/health` and `/v1/symbols`
  (`rest.rs:39-86`) and no REST limiter, and the playground serves only
  `/v1/symbols` too. 26:5-7 defers the reads. **Options:** (a) the gateway
  serves them over risk state; (b) they stay deferred. **Default if nothing
  is decided:** (b).
- **GW-FILL-FEE-ALWAYS-ZERO** (LOW, correctness) — `F` always carries `fee: 0`
  (`route.rs:81`); `FillRecord` has no fee field and risk computes it. 37 G23
  lists it. **Options:** (a) add the fee to the fill record on risk's gateway
  stream; (b) drop the `F` fee field from the spec. **Default if nothing is
  decided:** unchanged (always 0).
- **GW-SHARDED-REACTORS** (LOW, design) — 11 § Sharded reactors (proposed)
  specifies N pinned reactors with `SO_REUSEPORT`; 11 § Runtime Model states
  the build: one monoio reactor (`main.rs:176-180`) plus the recv thread.
  **Options:** (a) build the shard model; (b) delete the proposed section from
  11. **Default if nothing is decided:** (b).
- **GW-SHUTDOWN-NO-DRAIN** (LOW, ops) — shutdown does one `tick`, joins the
  recv thread and drops the runtime (`main.rs:228-238`): no close frames and
  `live` is never cleared (`main.rs:182` is the only store). 9-deploy
  § Rolling Upgrades (a `[STUB]`) wants stop accepting, drain, close. **Fix (proposal):** stop accepting,
  flip `/ready`, send close frames, then exit.
- **GW-STAMP-LATENCY-LIVE** (LOW, feature, spec 59) — `F` and the done `U`
  carry all five stamps (`route.rs:75-87, 141-152`); the inserted, cancelled
  and failed `U` send zeros (`update`, `route.rs:34-49`, called at `:120, :160,
  :189`).
  `OrderInsertedRecord` and `OrderCancelledRecord` are full 64-byte records
  with no spare field (`rsx-messages/src/lib.rs:173, 206`);
  `OrderFailedRecord` has 23 spare bytes (`:369`), not enough for five u64
  stamps. Clients show the "internal" leg as pending (BK-01,
  `rsx-glass/specs/04-screen-book.md`). **Options:** (a) widen those records
  (wire change);
  (b) stamp only what fits and say so in 59. **Default if nothing is
  decided:** unchanged (zeros).
- **GW-SERVER-FRAME-PARSERS-UNUSED** (LOW, duplication) — `parse` decodes the
  server-to-client frames `U`, `F`, `E` and `Q` (`parse_order_update` ..
  `parse_liquidation`, `rsx-messages/src/frames.rs:218-321`, about 105 lines) only so the
  handler can answer a client that sent one with `E[1004,"unsupported"]`
  (`handler.rs:284`); the gateway never accepts them. Tests and benches use
  them to read the gateway's own output (`rsx-messages/tests/frames_test.rs`,
  `stream_ordering_test.rs`, `order_lifecycle_test.rs`, `benches/gateway_bench.rs`,
  `benches/ws_parse_bench.rs`). **Options:** (a) delete the four parsers; a
  well-formed `U`/`F`/`E`/`Q` from a client then answers `E[1002,"unknown
  type: U"]` instead of `E[1004]` (or `parse` returns an unsupported error the
  handler maps to 1004, which also turns a malformed server frame from 1002 to
  1004), and the tests decode through a `tests/common` helper; (b) keep them.
  **Default if nothing is decided:** (b); it changes which E code a client
  sees, so it waits for sign-off.
- **ME-GW-DIRECT-SPEC-GAPS** and **DRAIN-REPLAY-4X-DUP** are open
  proposals kept below under their original date headings.

### owner decisions

- **GW-CID-MAP-LOST-ON-RESTART** (MED, design, owner decision) — the
  `(user_id, cid)` -> order id map (`client_ids.rs`) is in memory, so a gateway
  restart empties it. The ME does not make up for it: its dedup key is
  `(user_id, order_id_hi, order_id_lo)` (`rsx-matching/src/dedup.rs:10, 48`) and
  the retry after a restart mints a fresh order id, so the ME accepts it as a
  second order. A client retries exactly when the gateway died under it, so the
  window the map closes reopens for that case. No test — design.
  **Options:** (a) accept and document: a retry across a gateway restart is a
  new order and the client reconciles by fills and position; (b) persist the
  bindings (an append-only file under `RSX_GW_WAL_DIR`, replayed at boot and
  bounded by the 1 hour window); (c) derive the order id from `(user_id, cid)`
  so the ME dedup catches it, which also reuses an id for a later order with
  the same cid once the ME window ends while the first still rests (cancel by
  id then names two orders). **Default if nothing is decided:** (a).
- **GW-CID-RESERVED-AFTER-FAILURE** (LOW, design, owner decision) — a cid stays
  bound for the full hour after its order failed (risk `ORDER_FAILED`) or timed
  out in the gateway sweep (`route.rs:204`), so the same cid sent again after
  the failure is answered as a duplicate of the failed order and never reaches
  risk. The spec retry rule ("retry with the same cid if no update arrives",
  49 § ACK and retry) reads as resending a timed-out order, which this
  blocks. No test —
  design. **Options:** (a) keep the binding: a failed order's cid is spent and
  the client sends a new one; (b) drop the binding when the pending entry is
  removed by `ORDER_FAILED` or the timeout sweep, so the resend is a new order
  (after a timeout the first copy may still reach the ME, as 29 already says).
  **Default if nothing is decided:** (a).
- **CID-CLIENTS-REUSE-CIDS-ACROSS-RESTARTS** (MED, design) — the gateway binds a
  cid to its order for 1 hour per user, so a client whose cids restart gets
  its new orders answered as duplicates of the old ones. The Go clients on
  `rsx-core/wire` (the terminal, `Cid(counter)` from 1 per process,
  left-padded to 20 digits, `rsx-core/wire/order.go:16`; glass requirement
  GW-12, `rsx-glass/specs/12-contract-gateway.md`) still restart their
  counter every process. The generators (makers, stress client, `rsx-cli stress`,
  `ws_order_latency`, demo scripts, playground harness) mint a random session
  plus a base36 counter. No test — `rsx-core/wire` has no session source.
  **Fix:** a per-process session component in every `Cid`, as
  `rsx-maker/maker.go` `nextCID` does.

### measurement

- **GATEWAY-LATENCY-QUIET-BOX-REMEASURE** (MED, methodology) — the
  risk -> gateway recv runs on its own thread, SPSC ring and doorbell
  (`cast_recv.rs`), and the handler wakes on `select!{readable, egress.wait()}`
  (`handler.rs:126-138`), so neither the shared-reactor starvation nor the
  egress poll remains. The effect on `match_done_to_gateway` is not
  quantified: the 8-sample before/after medians (411.6 -> 539.3 us) overlap
  inside host noise (load average 8 -> 20 on an 8-core box with unrelated
  tenants; the untouched legs stayed tight), see
  `reports/20260723_gateway-egress.md`. The 11.5 ms single-connection WS p50
  that motivated the split is in `reports/20260530_gateway-ws-rest-latency.md`.
  **Fix:** re-measure with `ws_order_latency` on a quiet box with the recv
  thread pinned (`RSX_GW_CAST_CORE_ID`), p50 and p99 from the same run
  (GW-BENCH-NO-P99). No test — needs a quiet box.

### rulings (not defects)

- **GW-U-REASON-SLOT-BY-STATUS** (ruled, spec 18) — the `U` frame's reason
  slot (index 4) is one byte with two vocabularies, chosen by the status in
  index 1: status 3 carries a `FailureReason` (18 "FailureReason constants"),
  status 2 from an ORDER_CANCELLED record carries the `CANCEL_REASON_*` code
  the ME wrote (18 "CancelReason constants"; `route_order_cancelled`), every
  other status carries 0. A status-2 `U` from an ORDER_DONE(CANCELLED) record
  carries 0, which reads as `CANCEL_REASON_USER_CANCEL`:
  `OrderDoneRecord` has no reason field. Spec 49 § Gateway → client frames
  and spec 18 "CancelReason constants" document the slot. Consumers read the
  slot only under status 3 (`rsx-core/wire/order.go`,
  `rsx-playground/server.py`).
- **GW-REJECTS-PRICE-ZERO** (ruled not a defect) — the gateway answers a price
  0 order with `E[1008,"price not tick aligned"]`
  (`convert.rs:2-4`, `handler.rs:318-324`): 47 §§ 1.1-1.2 reject px <= 0 and
  no frame has a market type. The playground sends a marketable limit for
  "market" (`rsx-playground/server.py:5871-5912`).

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
  per tailing consumer). The recorder uses `run()` live-tail and
  drops a record at or below the last seq it wrote, so its archive keeps
  each seq once while the server still re-reads the file. Verified by code
  read; not reproduced live (playground stack was down). **Fix
  (proposal, needs sign-off):** skip records with `seq <= last_seq` in
  the live-tail loop (server-side filter restores spec §10.12 semantics
  without touching the reader); the recorder's seq guard keeps the
  archive clean but does not stop the re-streaming.
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

- **RISK-DEMOTED-FINAL-FLUSH-AFTER-LEASE-LOSS** (MED-HIGH, correctness) —
  on lease loss `run_main` calls `stop_persist_worker`
  (`rsx-risk/src/main.rs:965-966`), and the worker's shutdown path
  performs one FINAL FLUSH of pending events
  (`rsx-risk/src/persist.rs:398-410`) — after the advisory lock is
  already gone. A promoted new main may have flushed newer state in the
  gap (lease poll interval ~1s of pending events). The tips, positions,
  accounts and insurance upserts are monotonic, so the stale flush no-ops
  there; the other tables it writes (frozen orders, funding, liquidation
  events) carry no guard, and a duplicate funding interval fails the
  transaction carrying the rest. This is the one window where "at most one
  main" (invariant #10) does not imply "at most one writer".
  **Fix (needs sign-off — lifecycle semantics):** drop pending events on
  demote (replay recovers them).
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

## Status — 2026-08-03 — playground maker restart and typing

Record-only per triage; not fixed.

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
  response, but the order path treats those as HUMAN units
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

## Status — 2026-07-23 — live-cluster latency methodology notes

Caveats for comparing live per-leg stamps (`server_latency_ns`: `internal`,
`gateway_to_risk`, `risk_to_me`, `matching`, `match_done_to_gateway`) with
bench numbers. The gateway findings are under `## rsx-gateway`.

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
  (methodology fix, not a code bug) — the cluster runs `target/debug`
  binaries unless asked otherwise. Comparing live per-leg numbers (debug,
  ~5-20x slower on CPU-bound Rust) against bench-match-rt's release-mode
  figure is apples-to-oranges. The measurement-only opt-in
  `RSX_PLAYGROUND_RELEASE=1` (env, read by `rsx-playground/cluster_cli.py`,
  which passes `-r` to `rsx-cli cluster plan|up|restart`; default off) drives
  `perf-load`-style runs against `target/release` binaries.

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
  `wal_records.fills()` returns newest-first (`wal_feed.py`, `newest`), and
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
- **RISK-CMP-SEQ-DOMAIN-CONFLATION (diagnosis note for
  RISK-CMP-DESYNC-ORDER-EVENT-STARVE below)** — in the wedged state risk
  loops `replay drained … new_tip=13321` → `cmp sender reset detected:
  seq=11956 expected=13322` forever: the replay tip (WAL record seq) runs
  AHEAD of the live cmp stream's seq counter, so after every replay the
  next live packet looks like a sender reset and re-FAULTs. The two seq
  domains (replayed record seq vs live cmp seq) appear conflated in the
  receiver's reset_after_replay. Once diverged the boot never recovers —
  matches the "order events die minutes after boot" symptom.

## Status — 2026-07-12 — live demo trio

Record-only per triage; reproduces with a plain WS client on the live
`make demo` trio.

- **RISK-CMP-DESYNC-ORDER-EVENT-STARVE** (HIGH, correctness) — minutes after a
  fresh `reset + make demo` boot, user orders stop producing completion events:
  a client's `sent BUY … [IOC]` gets no Accepted/Done/Rejected/Fill back
  (first ~1 min: normal ~12 ms RTT; then multi-second RTTs; then nothing).
  `log/risk-0.log` shows `rsx_cast::cast: cmp sender reset detected: seq=N
  expected=M, re-sync` every ~2-3 s throughout — the risk↔ME cmp path is
  permanently desynced, so order round-trips are lost. Same window shows
  crossed/stale books at the client (md shadow diverges from `/api/book`).
  Likely the same family as `.ship/25-CMP-RELIABILITY-V2`. **Fix:** make the
  cmp re-sync actually recover in-flight completions (or fault loudly), and
  alert when `cmp sender reset` repeats at steady state.

## Status — 2026-07-11 — 3-token demo (display + maker polish)

The exchange core is fixed and correct — 3 tokens trade live, orders fill, MEs
FAULT=0 (the SEQ-1 per-ME fan-out re-sequencing + reset PG-wipe, in git). This
is a maker-tuning follow-up, not an exchange fault.

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
  playground's marketdata cache (`rsx-playground/server.py:8505-8510`), and the ME cannot rest a crossing order
  (`process_new_order` matches before `insert_resting`,
  `rsx-book/src/matching.rs:28-36`). The gateway-side miss is a different
  mechanism (error 1005 from the pending lookup, not this
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
  assistant (`specs/2/60-terminal-assistant.md`,
  `rsx-glass/specs/14-contract-support-assistant.md`). **Fix:** inventory
  named upstream code and add `NOTICE` entries with author, URL, license,
  copyright, and whether it was imported, adapted, or merely invoked; retain
  upstream headers and licenses. **Accept:** `NOTICE` exists, every named
  adapted/imported upstream has provenance, and the README links it.

## Status — 2026-07-11 — maker refine pass (deferred findings)

Review-pass findings verified but deliberately not applied (low severity /
speculative / risky-to-fix). Applied fixes are in git: fisheye inverse
`e7a0373`, ui stub/leak `3587538`, HL Close `30b7e86`, maker dial-timeout
`4a2829f`.

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

- **MD-L2-DELTA-MIXED-SLOT-QTY** (MED, correctness — marketdata) —
  `rsx-marketdata/src/shadow.rs` `derive_l2_delta` reports the slot's
  `total_qty`/`order_count`, which in a mixed compressed slot includes the
  other side and the other raw prices; the snapshot and BBO sum per side and
  raw price. Fix: sum the slot's order chain for `(side, price)` as
  `side_levels` does.

## Status — 2026-07-08 — live latency legs not stamped

Design settled → **`specs/2/59-latency-observability.md`** (planned). The
live internal/engine legs are delivered by **per-hop timestamps embedded in
the normal records** (not a bespoke delta frame), derived under assumed clock
sync (PTP in prod, single-box in dev); the client `net` leg stays
client-measured. Aggregate latency is Prometheus via the existing
`33-telemetry.md` Vector pipeline, not hand-rolled. The task below is an
increment of spec 59 (the gateway half, GW-STAMP-LATENCY-LIVE, is under `## rsx-gateway`):

- **ME-ENGINE-LATENCY-NOT-REPORTED** — now glass requirements LAT-03 and BK-01,
  `rsx-glass/specs/12-contract-gateway.md` and `04-screen-book.md`.

## Status — 2026-07-08 — ME→GW-direct spec is NOT ship-ready (codex adversarial audit)

- **ME-GW-DIRECT-SPEC-GAPS** (design blocker, proposal — needs sign-off; do NOT hand to a coding agent
  yet) — the async-output-split in `specs/2/28-risk.md` (ME sends fill straight
  to gateway; settle to Risk async) is `status: partial`/not-implemented, and an
  adversarial audit found it underspecified and unsafe as written.
  Frontmatter now reads `status: partial` (`28-risk.md:2`), matching the
  status block at `28-risk.md:199-205`. Verdict: **needs-spec-work-first**.
  Gaps that must be closed BEFORE any implementation:
  1. **No authoritative client sequence** once ME (per-symbol seq) and Risk
     (risk-gw seq) both feed the gateway — two clients/replay can disagree on
     order. Today Risk restamps one
     contiguous stream (`rsx-risk/src/rings.rs:42`, `forward_to_gw`).
  2. **Reject edge contradiction** — spec says the Risk→GW insufficient-margin
     reject "cannot be removed" (`28-risk.md:171`) but also says Risk drops
     `forward_to_gw`/response ring (`28-risk.md:203-205`); `forward_to_gw` carries
     BOTH fills and rejects (`rsx-risk/src/cast_loop.rs:233`, `:303`). Drop
     it and rejects vanish → client can't tell rejection from transport loss.
  3. **Gateway replay for the ME→GW stream is undefined** — no
     `RSX_ME_GW_REPLICATION_ADDR`, no per-symbol tip/replay-source/dedup; today
     gateway replays only Risk (`handle_replay`, `rsx-gateway/src/cast_recv.rs:134`,
     `drain_replay`, `replay.rs:23`). A dropped direct fill never reaches the client.
  4. **ME live-stream contents under-specified** → false FAULTED gaps: if the
     gateway gets only fill/DONE/FAILED, the WAL-seq holes at ORDER_ACCEPTED/BBO
     look like loss.
  5. **Duplicate fills** — no dedup key (`fill_id` / `(symbol,seq)`); during any
     mixed rollout the client double-counts (`rsx-gateway/src/route.rs:74`
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
     late Risk ORDER_FAILED emits a second terminal (`route_order_failed`, `route.rs:201-226`).
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
- **WS-CORE-DUPLICATED** (LOW, hardening, proposal) — `rsx-gateway/src/ws.rs`
  and `rsx-marketdata/src/ws.rs` each hold `WS_MAGIC`, the accept loop, the
  accept-key hash, the frame writers and the frame reader guards (mask
  required, FIN=1, 4 KiB cap). The crate graph has no shared home for
  monoio-based WS code (`rsx-types` has no runtime, `rsx-cast` is frozen), so
  only `WS_UPGRADE_DEADLINE` is shared. marketdata's `ws_handshake` still does
  one 4096-byte `read` and treats it as the whole request (a request split
  across segments is refused for a missing key; over 8 KiB is not answered
  `431`). **Fix:** hoist the domain-agnostic core into one crate both depend on
  and give marketdata the gateway's `read_http_request`. Needs sign-off: a new
  workspace crate.
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
- **GW/MD-README-ENV-DRIFT** (LOW, docs) — CONFIRMED at HEAD 2026-10-21.
  `rsx-gateway/README.md:14-36` names env the code does not read:
  `RSX_GW_LISTEN_ADDR` (code `RSX_GW_LISTEN`, `config.rs:81`),
  `RSX_GW_HEARTBEAT_INTERVAL_MS` / `_TIMEOUT_MS` (code
  `RSX_GW_HEARTBEAT_INTERVAL_S` / `RSX_GW_IDLE_TIMEOUT_S`, `config.rs:85-86`),
  `RSX_GW_CIRCUIT_*` (code `RSX_GW_CB_*`, `config.rs:90-91`), and omits
  `RSX_GW_RL_USER`, `RSX_GW_RL_IP`, `RSX_GW_HEALTH_ADDR`, `RSX_GW_CORE_ID`.
  `rsx-gateway/ARCHITECTURE.md:150` says the handler waits "readable (10ms
  timeout for poll)"; it parks on `select!{readable, egress.wait()}`
  (`handler.rs:126-138`), no timer. The marketdata README uses `RSX_MKT_*`
  (`rsx-marketdata/README.md:19-22`) where the code reads `RSX_MD_*`
  (`rsx-marketdata/src/config.rs:61-63`). **Fix:** the gateway env table from
  `config.rs` and `main.rs` (gateway plan step 13); the marketdata table in
  the marketdata pass. No test — docs.
- **DRAIN-REPLAY-4X-DUP** (LOW, duplication — do NOT act, proposal) —
  `drain_replay` exists in three crates: `rsx-gateway/src/replay.rs:23`,
  `rsx-marketdata/src/replay.rs:36` (the gateway's body apart from the log
  label) and `rsx-risk/src/replay.rs:64` (a different apply and tip pre-seed).
  Each builds a tokio runtime, so its natural home (rsx-cast) can't take it —
  rsx-cast is FROZEN and never gets a runtime dep. Extraction needs founder
  sign-off on a NEW shared crate. Record only. CONFIRMED at HEAD 2026-10-21.

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
  with an empty process table. (3) Fresh PG needs `seed_accounts` (runs at
  playground startup) + a risk restart to load them. (4) `/api/risk/users/N`
  mislabels an empty-positions
  result as "no postgres connection". Correct bring-up order: PG → playground
  server → cluster (start-all) → maker → deposits. Fix: a supervised start
  sequence (or make PG a compose service + the playground not lose the process
  table on restart).

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

**NEW 2026-07-04** (sonnet bug-hunt, all verified genuine — detail at end):
COMPRESSION-ZONE-TICK-UNIT (HIGH, latent), WAL-ROTATE-PREWRITE-MISLABEL (HIGH),
CLI-PTR-READ-UNALIGNED-UB (HIGH), CAST-SEND-RING-TOO-SMALL (MED),
MARK-PARSE-NEG-ZERO (LOW).

**NEW 2026-07-04** (T4 `.ship/33-TUI-SPEED-TESTS` fixture debugging —
detail at end): DEMO-TRADE-SUBMIT-ORDER-404 (MED).

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

## PLAYGROUND-DASHBOARD-STALE-PID-RESTART-RACE [OPEN]
**Severity:** LOW (dev ergonomics)
**Where:** `rsx-playground/playground` restart vs a dashboard started outside
the wrapper (no PID file).
**Symptom:** if a dashboard is launched directly (not via `./playground
start`), `./playground restart` can't see it (no PID file) and aborts on a
stale-PID race; subs had to `kill` by PID and relaunch via the wrapper.
**Fix (defer, record only):** have restart fall back to port-owner lookup
(`ss -ltnp` on 49171) when the PID file is missing/stale.

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

## ME-FLUSH-OVER-10MS-UNDER-LOAD [OPEN]
**Severity:** LOW (observed once, not reproduced)
**Where:** rsx-matching WAL flush; seen in the same run as marketdata seq gaps.
**Symptom:** me-pengu logs `flush took 10-14ms` against the 10 ms target while
marketdata logged `seq gap` warnings (dropped casts).
**Fix (defer):** measure flush latency on a quiet box with the perf gate.

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
