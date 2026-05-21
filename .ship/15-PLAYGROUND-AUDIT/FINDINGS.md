# Playground UI Audit -- Findings

Audit performed 2026-05-21 via agent-browser against the running
playground at http://localhost:49171/. Cluster was up at audit
start (verified `make latency-publish` had captured a p50 ~ 11.7
ms). During the audit, the matching engine (and downstream
marketdata + maker) crashed and restarted repeatedly with no
operator action -- see Finding 1.

Tabs visited (15): Walkthrough, Overview, Topology, Latency, Book,
Risk, WAL, Logs, Control, Maker, Faults, Verify, Orders, Stress,
Docs, Trade. The Infra and API tabs called out in the audit brief
do not exist in the current UI (the docs are stale).

---

## Finding 1: Matching engine is in a restart loop (`AddrInUse`/`flush stalled`); downstream collapse follows

- Where: `me-pengu` process, surfaced on Overview, Topology,
  Control, Verify (`GW->ME->GW p99 6 320 410 us`).
- What I did: Just observed. The audit performed no
  start/stop/restart actions. Within ~5 min of poking around,
  the maker self-aborted (`quote circuit breaker: 10 consecutive
  errors; aborting maker`), and gw-0 / risk-0 / mark / marketdata
  / me-pengu all flipped to `stopped`.
- Expected: With no faults injected, the cluster should stay
  green for the length of the audit.
- Actual: `log/me-pengu.log` shows repeated
  `panicked at rsx-matching/src/main.rs:319:6: failed to bind
  CMP receiver: Os { code: 98, kind: AddrInUse, message:
  "Address already in use" }` followed by `wal append failed
  (order-accepted) -- violates 6-consistency.md invariant 7
  (WAL persistence) and breaks dedup on replay: Custom { kind:
  WouldBlock, error: "flush stalled, backpressure" }`.
  Successive restarts panic on the CMP port held by the
  panicking parent's still-open socket. Each restart leaves
  the WAL active file truncated (see Finding 4) and clears the
  book. Marketdata separately panics on `memory allocation of
  45000784944 bytes failed` (~45 GB) when ME's stream comes
  back.
- Console / network errors: none in the browser; this is all
  server-side. The UI hides it under "100 GREEN".
- Suggested Playwright test: extend `play_readiness.spec.ts`
  with a `system_stays_green_for_5m` scenario: poll
  `/api/processes` and `/x/health` every 5 s; fail if any RSX
  process restarts or `health` drops below 100 with zero
  injected faults. Also add an assertion that
  `log/me-pengu.log` contains no `AddrInUse` panics during a
  warm-cluster window.
- Severity: critical -- every other finding in this report is
  partly a downstream symptom of this.

## Finding 2: Health score stays "100 GREEN" while the system is visibly broken

- Where: `/overview`, `/x/health`, `/x/key-metrics`,
  `/x/pulse`.
- What I did: Loaded `/overview` while the cluster was
  obviously degraded (latency p99 ~ 6 s, ME restarting, fills
  failing). Hit `/x/pulse`, `/x/key-metrics`, `/x/health`
  directly.
- Expected: Health score reflects observable issues (process
  restarts, latency regression vs baseline, errors in logs).
- Actual: `/x/health` returns `100 GREEN`. `/x/key-metrics`
  returns `Errors 0` and `Msgs/sec 0` even though maker is
  placing ~0.35 orders/s (`orders_placed=426` over 20 m
  uptime) and gw-0 log contains repeated `WARN handshake
  failed: missing or invalid auth` lines. `/x/pulse` agrees
  with `Errors 0`.
- Console / network errors: none.
- Suggested Playwright test: new spec
  `play_health_truthful.spec.ts` -- inject one fault (kill ME
  via `/api/processes/me-pengu/kill`), assert health drops
  below 100 within 10 s and recovers to 100 within 60 s. Also
  assert `Msgs/sec > 0` while the maker is running.
- Severity: critical -- the headline dashboard is a lie under
  current code.

## Finding 3: WAL UI reports 0.0 B for all streams while disk and Verify disagree

- Where: `/wal`, `/x/wal-status`, vs `/verify` and
  `/tmp/wal/*/*.wal`.
- What I did: Loaded `/wal`; the "PER-PROCESS WAL STATE" and
  "LAG DASHBOARD" tables show all four streams (archive, mark,
  pengu, sol) as `0.0B`. Same second, `/verify` showed
  `WAL stream pengu has files 1 files, 3.1KB` and a few
  seconds later `6.0KB`. `ls -la tmp/wal/pengu/10/10_active.wal`
  showed 0 bytes -- because each ME restart truncates the
  active file and loses unflushed records.
- Expected: The WAL page should show non-zero size whenever
  records have been appended since last rotation, and should
  agree with `/verify`'s reading of the same files within a
  refresh interval.
- Actual: Three different views (WAL page, Verify page, disk)
  disagree. The WAL page appears to read sizes after a flush
  cycle that never completes because ME crashes first.
- Suggested Playwright test: extend `play_wal.spec.ts` with a
  `wal_size_agrees_with_verify` case: poll `/x/wal-status` and
  `/api/verify/run` simultaneously; assert sizes match within
  +-10 % across N polls when no fault is injected.
- Severity: important.

## Finding 4: Verify check "Fills precede ORDER_DONE" reports "no trades yet" while fills are visibly happening

- Where: `/verify`, invariant row "Fills precede ORDER_DONE
  (per order)".
- What I did: Clicked "Run All Checks". The row went PASS with
  detail `0 fills; system running, no trades yet`. At the same
  time `/api/maker/status` returned `orders_placed: 426`, the
  Book page showed `LIVE FILLS PENGU buy 0.050125 10 seq 434`,
  and `/x/topology/gateway` reported `fills (session) 135`.
- Expected: With fills observable in three other places, this
  check should either count them and PASS truthfully, or
  fail/skip with an honest message.
- Actual: It silently SKIPs via a hard-coded "no trades yet"
  message and is marked PASS. False-green.
- Suggested Playwright test: extend
  `play_guarantees.spec.ts` so that after the maker has been
  running >= 30 s, `/api/verify/run` must report
  `fills_observed > 0` and the per-invariant message must not
  contain "no trades yet".
- Severity: important -- Verify is the spec-correctness
  surface and is currently lying.

## Finding 5: Topology shows Gateway "stopped" + `pid: -` while `/api/processes` reports it running

- Where: `/topology`, click "Gateway --" detail
  (`/x/topology/gateway`).
- What I did: Loaded `/topology`; selected detail returned
  `<span>stopped</span> <span>pid: -</span>` even though
  `/api/processes` returned `gw-0 running pid=4026036
  uptime=53s` the same second. Same partial reported session
  counters `orders 137, fills 135, WAL tip sym10=6929` --
  proving the process IS reachable. So the partial reads
  liveness from one source and counters from another, and they
  disagree.
- Expected: Both readings come from the same process-state
  oracle.
- Actual: Two readers, one (counters) live, one (status pill)
  reading a stale or wrong file.
- Suggested Playwright test: new test in
  `play_topology.spec.ts`:
  `topology_status_pill_agrees_with_api_processes` -- for each
  component, fetch `/x/topology/{name}` and `/api/processes`;
  assert the pill state matches the API state.
- Severity: important -- defeats the topology screen's whole
  purpose as a status overview.

## Finding 6: Topology "Mark" detail is empty when the mark process is running

- Where: `/x/topology/mark`.
- What I did: `curl /x/topology/mark`.
- Expected: Mark detail panel showing current mark prices,
  funding state, sample rate.
- Actual: Reduces to `Mark running pid 3799293 uptime 9m59s
  mark data requires mark process` -- i.e. an error message
  embedded in the panel saying the mark process is required,
  even though the process is right there. Same partial for
  other components is informative (e.g. Maker: orders placed,
  active orders, spread bps). Looks like a forgotten TODO/stub.
- Suggested Playwright test: add to `play_topology.spec.ts`:
  `topology_mark_detail_shows_real_data` -- assert response
  contains at least one numeric mark price when `mark` is
  running.
- Severity: nice-to-have.

## Finding 7: Logs "gateway" quick-filter returns "no log lines" while "errors only" surfaces gw-0 WARN lines

- Where: `/logs`, top-row filter buttons.
- What I did: Clicked "gateway" -> empty list. Clicked
  "errors only" -> 5+ lines of
  `[gw-0] WARN rsx_gateway::handler: write error conn 27xx:
  Broken pipe`.
- Expected: "gateway" matches the same `[gw-0]`-prefixed lines,
  so it should be a superset of what "errors only" shows.
- Actual: "gateway" filter probably matches the literal
  process name `gateway` rather than `gw-0`. The label in the
  UI is the one used in `make` and docs; the log prefix is the
  shard-numbered one.
- Suggested Playwright test: extend `play_logs.spec.ts` with
  `filter_label_to_log_prefix_consistency`: each quick filter
  must produce >= 1 line if the corresponding process is
  emitting logs.
- Severity: important.

## Finding 8: Risk system-wide metrics show `--` for OI/notional with 135 fills in session

- Where: `/risk`, "SYSTEM-WIDE RISK METRICS" card.
- What I did: Loaded `/risk` after the maker had been running
  for > 5 m and `/x/topology/gateway` reported `fills
  (session) 135`. The card shows `TOTAL OI --`, `LONG NOTIONAL
  --`, `SHORT NOTIONAL --`, `ACCOUNTS W/ POSITIONS 0`. Yet the
  Maker tab at the same time reports `Positions 2` (Overview
  agrees with `Positions 2`).
- Expected: With >= 1 fill on the maker's account, the OI and
  notional numbers should be populated and "accounts w/
  positions" should be >= 1.
- Actual: System-wide aggregates appear unwired; per-user
  detail (`user 1` and below) does render correct collateral
  but every user is "no open positions" -- contradicting
  overview's `Positions 2`. Multiple aggregation sources
  disagree.
- Suggested Playwright test: extend `play_risk.spec.ts` with
  `risk_system_metrics_populated_when_fills_exist`: after
  >= 10 maker fills, assert `TOTAL OI != --` and
  `ACCOUNTS W/ POSITIONS > 0`.
- Severity: important.

## Finding 9: ME -> Mktdata CMP counter stuck at 0 while orderbook + fills propagate

- Where: `/topology` -> "CMP CONNECTIONS" table; same data at
  `/x/cmp-flows`.
- What I did: Counter for `ME -> Mktdata` shows `Sent 0 /
  Recv 0 / NAK 0 / Drop 0` after 2172 `Gateway -> Risk`
  packets in the same window. Marketdata IS receiving updates
  (the ladder updates and Book tab shows live fills), so
  either the counter is broken or marketdata is reading from a
  different source than ME's CMP publisher.
- Suggested Playwright test: extend `play_topology.spec.ts`
  with `cmp_counters_track_marketdata_progress`: after N maker
  orders, `ME -> Mktdata.sent` must be >= N (or >= N fills, if
  it's per-fill).
- Severity: important -- the CMP flow visualization is the
  primary architecture demo on the topology page; a stuck zero
  there undermines the story.

## Finding 10: Stuck price level rendered as `50000` (`data-px="50000000000"`) after an ME restart

- Where: Landing-page orderbook
  (`/x/book?symbol_id=10`), first ask row.
- What I did: Loaded `/` and observed ask rows. The first row
  had `data-px="50000000000"` (50 * 10^9) and rendered as
  `50000`; the next five asks had `data-px="50100..50300"`
  rendering correctly as `0.0501..0.0503`. Polled twice 5 s
  apart -- the stale row persisted while quantities on the
  other rows updated.
- Expected: All asks share the same price scale; an order at
  raw 50 G would render as 50 000 (impossible for PENGU), so
  the level must have been written wrong.
- Actual: Probably created during a state where the maker
  shipped a price in different units than the configured tick
  (a leftover of one ME restart). Side effect: BOOK STATS
  still shows the correct best ask (`0.050125`) because it
  sorts by raw price, but the ladder is misleading.
- Suggested Playwright test: extend `play_book.spec.ts` with
  `ask_prices_monotonic_and_in_tick_band`: assert all ask
  `data-px` values are within `[best_ask_raw,
  best_ask_raw * 1.10]` and each is a multiple of `tick_size`.
- Severity: important -- first visible artifact on the
  landing page, undermines first impression.

## Finding 11: Stress page "Scenarios" panel renders only "no data"

- Where: `/stress`, `/x/stress-scenarios`.
- What I did: Loaded `/stress`. The top scenarios card showed
  "Loading scenarios..." then settled on "no data". Direct
  `curl /x/stress-scenarios` returns the literal `<span
  class="text-slate-500 text-xs">no data</span>`.
- Expected: List of named stress profiles (`stress-low /
  stress-high / stress-ultra` -- they're listed in the Control
  scenario selector).
- Actual: Endpoint returns the placeholder. Possibly a
  forgotten wiring; the run-form below works.
- Suggested Playwright test: extend `play_stress.spec.ts`
  with `stress_scenarios_panel_lists_named_profiles`: assert
  at least three named scenarios are present.
- Severity: nice-to-have.

## Finding 12: Trade UI shows "disconnected" + "Authentication failed -- check credentials"

- Where: `/trade/`.
- What I did: Opened. All quote fields show `--`, status pill
  says `disconnected`, a red toast appears with
  "Authentication failed -- check credentials".
- Expected: Either an obvious GitHub sign-in prompt (button
  exists) or -- if the spec is "logged-out user sees live
  mark/index without auth" -- show those numbers. Either way,
  no scary "Authentication failed" toast on first paint before
  the user tried to sign in.
- Actual: The toast looks like a hard error, while it's really
  just "you're not logged in yet".
- Suggested Playwright test: new `play_trade.spec.ts` cases
  `trade_initial_paint_has_no_error_toasts`: on first load
  without auth, assert no element with role=alert containing
  "failed"; and
  `trade_market_feed_connects_when_md_running`: with
  marketdata running, status pill must reach `connected`
  within 10 s.
- Severity: important -- Trade UI is a public-facing demo.

---

## Things that work cleanly

- Walkthrough page renders, anchor links scroll to all 9
  sections.
- Orders tab: 1x / 5x / 20x / 100x buttons all submit; trace
  works on a known oid; "order ... not found in session"
  shown for unknown oids.
- Book tab: ladder renders, symbol switch via combobox works
  for PENGU/SOL/BTC/ETH (though see Finding 10 for ladder
  scaling under stress and below for unfiltered side-panels).
- Faults tab: Kill/Stop buttons present per-process; clear
  recovery-notes guidance about iptables / hex-editing for
  the network / WAL paths the UI doesn't yet implement.
- Latency tab: presents three correctly-labelled tiers (e2e
  probe, gateway round-trip, microbenches) and surfaces the
  regression vs baseline clearly. Honest disclaimer on the
  "GW round-trip" card is the best-written piece of copy in
  the playground.
- Docs sub-pages (api / index / scenarios / tabs /
  troubleshooting) all render.
- `/api/risk/users/{n}/freeze` + `/unfreeze` + `/deposit`
  endpoints all returned 200 with sensible JSON.

## Minor cosmetic issues (no finding written)

- Book tab: BOOK STATS / TRADE AGGREGATION panels don't
  filter by the selected symbol -- switching to BTC still
  shows PENGU rows in those panels.
- Overview key-metrics "Active Orders 152" vs Orders page
  shows the last 50 -- fine, just worth a tooltip clarifying
  scope.
- Docs nav advertises Infra and API tabs that don't exist as
  pages in the current UI; either restore them or remove from
  the docs.
- `/x/topology/detail?component=gateway` (a guess at the
  endpoint shape) returns "unknown component: detail" --
  honest 400-equivalent, but the param shape is undocumented.
