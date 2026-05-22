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

---

# Oracle pass (codex, 2026-05-21) -- 7 more lies

Adversarial second pass via `codex exec` against
`server.py` after the agent-browser audit landed.
Each claim below was verified by direct read.

## Finding 13: `/x/pulse` proc pill paints green on any running process

- Where: `server.py:2961-2963`
- Claim: `proc` pill goes green for a healthy estate.
- Truth: `"emerald-400" if running > 0 else "red-400"`.
  1/8 alive paints success; partial cluster death is
  color-coded healthy.
- Repro:
  1. `curl -sX POST localhost:49171/api/processes/me-pengu/kill >/dev/null`
  2. `curl -s localhost:49171/x/pulse | grep -o 'emerald-400[^>]*">[0-9]*/[0-9]*'`
- Severity: critical.
- Playwright: `play_health_truthful.spec.ts::pulse_proc_pill_not_green_on_partial_outage`

## Finding 14: Gateway "circuit breaker: closed" is a hardcoded string

- Where: `server.py:2127` -- `("circuit breaker", "closed")`
- Claim: Detail panel reports the gateway's breaker state.
- Truth: The row is a literal tuple. It never reads gateway
  state, breaker state, logs, or any counter. Stays "closed"
  while gateway is dead or refusing connections.
- Repro:
  1. `curl -sX POST localhost:49171/api/processes/gw-0/kill >/dev/null`
  2. `curl -s localhost:49171/x/topology/gateway | grep -o 'circuit breaker.*closed'`
- Severity: important.
- Playwright: `play_topology.spec.ts::gateway_circuit_breaker_not_hardcoded`

## Finding 15: `/x/topology/flow` rates come from Python in-process memory

- Where: `server.py:2370-2371`
- Claim: Node badges show live order/fill flow on the
  cluster.
- Truth: `client` rate is `len(recent_orders)`, `gateway`
  rate is `len(recent_fills)`, `marketdata` is
  `len(_book_snap) sym`. All three are FastAPI-process
  dictionaries that reset on dashboard restart, ignore
  traffic the dashboard did not witness, and can be moved
  by UI helpers that never touch the cluster.
- Repro:
  1. `curl -sX POST localhost:49171/api/processes/all/stop?confirm=yes >/dev/null`
  2. `curl -s localhost:49171/x/topology/flow | jq '.nodes[] | select(.key == "client") | .rate'`
     -- still shows non-zero immediately after.
- Severity: important.
- Playwright: `play_topology.spec.ts::flow_counters_not_from_dashboard_memory`

## Finding 16: Index price is synthesized from mark price

- Where: `server.py:5697` -- `index_px = int(mid * 1.0001) if mid else 0`
- Claim: `/api/risk/funding` returns each symbol's mark,
  index, premium, and funding rate.
- Truth: `index_px` is fabricated as `mark * 1.0001`,
  `premium_bps` is derived from that fake index, and
  `rate_bps` is just `(mid - index) / index`. Placeholder
  math dressed up as market structure -- no external index
  source is queried.
- Repro:
  1. `curl -s localhost:49171/api/risk/funding | jq '.funding[0] | {mark_px,index_px}'`
  2. `# index_px / mark_px == 1.0001 exactly`
- Severity: important.
- Playwright: `play_risk.spec.ts::funding_uses_real_index_source_not_formula_stub`

## Finding 17: Reconciliation "Mark vs Index" is "book has bid and ask"

- Where: `server.py:3354-3367`
- Claim: A PASS on the reconciliation panel means mark
  pricing agrees with index pricing.
- Truth: The check sets PASS if a symbol merely has
  `bid > 0 and ask > 0` in `_book_snap`. No index is loaded
  anywhere in the function. "Reconciliation" is just "book
  not empty."
- Repro:
  1. `curl -sX POST localhost:49171/api/processes/mark/kill >/dev/null`
  2. `curl -s localhost:49171/x/reconciliation | grep -o 'valid BBO mid'`
     -- still PASS with mark dead.
- Severity: important.
- Playwright: `play_guarantees.spec.ts::reconciliation_mark_vs_index_loads_real_index`

## Finding 18: Reconciliation "Shadow vs ME" compares two WAL views

- Where: `server.py:3328-3352`
- Claim: Shadow book agrees with the matching engine.
- Truth: It compares `_book_snap` (which is parsed *from
  the WAL*) against `parse_wal_bbo` (also from the WAL).
  Both are downstream views. ME can be down or divergent
  and this check can still PASS on stale copies of itself.
- Repro:
  1. `curl -sX POST localhost:49171/api/processes/me-pengu/kill >/dev/null`
  2. `curl -s localhost:49171/x/reconciliation | grep -o 'symbols match'`
     -- still PASS with ME dead.
- Severity: important.
- Playwright: `play_guarantees.spec.ts::reconciliation_shadow_vs_me_queries_engine_truth`

## Finding 19: `/x/stale-orders` skips orders with string timestamps

- Where: `server.py:3419-3423`
- Claim: `0 stale orders` means there are no non-terminal
  orders stuck.
- Truth: The detector requires `isinstance(o.get("ts"),
  (int, float))`. Orders submitted via UI batch helpers
  write `ts` as `"%H:%M:%S"` strings, so they age forever
  and never become stale. The badge is always 0.
- Repro:
  1. `curl -sX POST localhost:49171/api/orders/batch -d '{}' -H 'content-type: application/json'`
  2. `sleep 65; curl -s localhost:49171/x/stale-orders`
- Severity: important.
- Playwright: `play_safety.spec.ts::stale_orders_counts_string_timestamp_orders`

---

# Soak pass (2026-05-22) -- gateway churn investigation

## Finding 20: gw-0 "crash every 2-3 min under WS churn" is a SUPERVISOR footgun, not a gateway defect

- Reported symptom (prior agent): gw-0 restarts every ~2-3 min
  under sustained WS connection churn; uptime resets observed
  2m42s -> 2m11s -> 35s while the watcher respawned it; maker
  (user 99) opening ~2 conns/sec.
- The gateway code is correct -- ruled out every code-bug hypothesis:
  - **No panic / no fatal.** `log/gw-0.log` contains **zero**
    `panic`/`FATAL`/`fatal:`/`aborting` lines across its entire
    history (8h+ of runtime, 14k+ connections). The panic hook
    (`rsx-types` `install_panic_handler`) prints `fatal: ...` and
    `process::exit(1)`; that string never appears. The only
    `expect()`s in `rsx-gateway/src` are startup fail-fast
    (`main.rs`) or guarded invariants (`protocol.rs:150` after
    `obj.len()==1`, `ws.rs` `try_into` after `read_exact`,
    `state.rs:318` after insert). The accept/handshake path
    returns `io::Result` and `warn!`s on every error.
  - **No resource leak.** After 8h and ~14k connections, gw-0
    held **8 fds (4 sockets), 8 MB RSS**. `connections` is
    removed on close/timeout; `user_limiters` is keyed by a
    bounded user set; `ip_limiters` is FIFO-capped at
    `IP_LIMITER_MAX` (10 000). Nothing grows unbounded.
  - **WS churn alone does not kill it.** An isolated gateway
    on a private port (no risk/ME behind it) survived a 32-thread
    flood -- 19 599 connections, mixed unauth + abrupt-close --
    with zero panic and a stable pid. So handshake/accept churn
    is handled correctly.
- Actual root cause (evidence-backed): the "restarts" are
  external SIGKILLs of the estate, with two contributing sources:
  1. **A concurrent harness re-running the cluster.** During the
     soak, `gw-0/me-pengu/marketdata/risk-0` restarted in lockstep
     (all uptime == 6-10 s together). The killer was a
     `uv run pytest tests/` process whose parent chain is a
     *separate* `claude -c` session -- its fault-injection /
     restart tests cycle the shared live cluster.
  2. **`start_all` SIGKILLs by loose name match.** Every
     `start_all` ran `pkill -9 -f rsx-gateway` (and peers) to
     "clear stale binaries." `-9 -f rsx-gateway` matches *any*
     command line containing the substring -- including an
     isolated test gateway on another port, log tails, an editor,
     or an agent session that merely names the binary -- and
     SIGKILL bypasses the F1 graceful WAL drain. So any stray
     `start_all` (e.g. from a parallel session or the readiness
     auto-heal) kills the running estate with no log trace, which
     is exactly the silent "restart with no panic" symptom.
- Fix (minimal, supervisor-side): `server.py start_all` now
  matches the full build path `target/debug/rsx-<bin>` instead
  of the bare binary name, and sends **SIGTERM first** (so F1's
  drain runs), escalating to SIGKILL only for survivors. This
  stops `start_all` from collaterally killing unrelated
  processes and preserves graceful shutdown. No `rsx-gateway`
  code change -- the gateway is correct.
- Proof: warm cluster restarted clean; with no fault-injector
  attached, gw-0 ran 8h8m at a constant pid (8 fds, 8 MB). The
  isolated-gateway flood (19 599 conns) left the process alive.
  The lockstep estate restarts disappeared once the concurrent
  pytest harness was identified as their source.
- Regression test:
  `play_readiness.spec.ts::@long gateway churn stability (F20)`
  drives ~90 s of churn through the gateway WS path and asserts
  gw-0's pid never changes and uptime is monotonic. The existing
  `system_stays_green_for_5m` soak covers the whole estate.
- Caveat / regression note: this test (and the cluster) is only
  valid in isolation. If a second session runs `pytest tests/`
  or `start_all` against the same machine, it WILL restart the
  estate -- that is environmental, not a gateway bug. Run the
  soak with no concurrent harness attached.
- Severity: not a gateway bug; supervisor hardening shipped.

---

# Oracle pass 2 (codex, 2026-05-22) -- 8 perf/telemetry lies

Second adversarial codex pass, targeting the latency,
"hardware", cached-status, and rendering surfaces the first
pass did not examine. Each verified by direct read.

## Finding 21: `/x/core-affinity` invents core numbers from list index

- Where: `server.py:3172` -> `pages.py:2660-2679`
- Claim: panel shows real CPU pinning / core placement.
- Truth: `for i, p in enumerate(processes): ... Core {i}`.
  The "pinned core" is just the row index. No affinity mask,
  cpuset, or `sched_getaffinity` is ever read. Pure fiction --
  and damning on an exchange whose whole pitch (TILES.md,
  `core_affinity`) is pinned hot threads.
- Repro:
  1. Pin two processes to the same physical CPU (or none).
  2. `curl -s localhost:49171/x/core-affinity` -- still one
     fake ascending core per row in dashboard order.
- Severity: critical.
- Playwright: `play_topology.spec.ts::core_affinity_backed_by_real_cpu_affinity`

## Finding 22: `/api/latency-probe` accepts ANY fill frame as the probe result

- Where: `server.py:5994` (the `if "F" in frame:` branch in
  `_run_latency_probe`).
- Claim: measures the probe order's real GW->ME->GW fill
  round-trip and returns its `cid`.
- Truth: it sends a probe order with a unique `cid`, then
  returns on the FIRST frame containing an `"F"` key WITHOUT
  checking the fill's cid matches the probe. It then echoes
  the probe's own `cid` in the response, so the result LOOKS
  matched. On the shared user-1 WS with the maker running, an
  unrelated fill arriving first is timed and reported as
  exchange latency. This is the headline latency number.
- Repro:
  1. Keep the maker filling on user 1.
  2. `curl -sX POST 'localhost:49171/api/latency-probe?symbol_id=10'`
     can return on a fill that is not the probe's order.
- Severity: critical -- the <50us story rests on this probe.
- Playwright: `play_latency.spec.ts::probe_matches_fill_to_its_own_cid`

## Finding 23: `/x/latency-regression` labels gateway-response time as engine round-trip

- Where: `server.py:3451` + `/api/latency` `server.py:5885`;
  fed by `order_latencies` populated at `server.py:~4056`.
- Claim: widget shows `GW->ME->GW p99` vs the 50us baseline.
- Truth: `order_latencies` is "time until first non-heartbeat
  gateway response or timeout" for ordinary order submits --
  including rejects and resting orders that never match. Not
  an engine round-trip, despite the label.
- Repro:
  1. Submit only resting/rejected orders.
  2. `/x/latency-regression` shows those gateway-response
     timings as `GW->ME->GW p99`.
- Severity: important.
- Playwright: `play_latency.spec.ts::regression_label_matches_what_is_measured`

## Finding 24: `/x/invariant-status` paints green on an empty cache

- Where: `server.py:3165` -> `pages.py:2580`. Renders the
  cached global `verify_results`.
- Claim: badge reflects live invariant health.
- Truth: an empty/never-run cache renders green "All passing".
  After a dashboard restart, before anyone runs
  `/api/verify/run`, the badge claims all invariants pass
  though zero checks have executed.
- Repro:
  1. Restart dashboard.
  2. `curl -s localhost:49171/x/invariant-status` -- green
     "All passing" with an empty cache.
- Severity: important.
- Playwright: `play_guarantees.spec.ts::invariant_status_not_green_when_cache_empty`

## Finding 25: `/x/ring-pressure` fabricates SPSC ring occupancy from WAL lag

- Where: `server.py:3159` -> `pages.py:2544-2562`.
- Claim: bars represent real ring occupancy / backpressure.
- Truth: `pct = min(100, int(lag_mb * 10))` (or `files * 5`).
  No ring depth, capacity, or enqueue/dequeue counter is read
  anywhere. The SPSC rings are an intra-process Rust concept
  (rtrb) the dashboard has no visibility into; this is a WAL-
  lag heuristic wearing a ring-pressure costume. The docstring
  even admits "Derive ring fill % from WAL stream lag."
- Repro:
  1. Let a WAL stream accumulate lag.
  2. `/x/ring-pressure` "pressure" jumps with no ring metric read.
- Severity: important.
- Playwright: `play_topology.spec.ts::ring_pressure_reads_real_telemetry_or_is_labeled_derived`

## Finding 26: `/x/key-metrics` Msgs/sec is a lifetime dashboard average

- Where: `server.py:3088` (`mps = len(recent_orders) / elapsed`,
  `elapsed = time.time() - SERVER_START`).
- Claim: `Msgs/sec` is current throughput.
- Truth: numerator is an in-memory list capped at recent
  orders; denominator is dashboard uptime. The rate decays
  toward zero the longer the dashboard runs and never reflects
  the live cluster message rate. (Distinct from F2: this is
  the throughput tile, not the health score.)
- Repro:
  1. Burst orders, then idle several minutes (no restart).
  2. `Msgs/sec` keeps falling though nothing changed.
- Severity: important.
- Playwright: `play_health_truthful.spec.ts::msgs_sec_uses_recent_window_not_uptime`

## Finding 27: `/api/maker/status` serves stale file stats after the maker dies

- Where: `server.py:6306`, always reads `tmp/maker-status.json`
  via `_read_maker_stats()` regardless of `running`.
- Claim: `levels` / `errors` describe the live maker subprocess.
- Truth: when the maker stops or crashes without deleting the
  file, the endpoint returns `running: false` alongside the
  dead process's old `levels`/`errors` -- stale state
  masquerading as current.
- Repro:
  1. Let maker write status, then kill it without removing
     `tmp/maker-status.json`.
  2. `curl -s localhost:49171/api/maker/status` -- old stats persist.
- Severity: important.
- Playwright: `play_topology.spec.ts::maker_status_clears_stats_when_not_running`

## Finding 28: `/api/stress/reports` silently swallows corrupt reports

- Where: `server.py:5223`, `except Exception: continue`.
- Claim: lists all stress-test reports.
- Truth: any malformed/partial `stress-*.json` is silently
  dropped, so failed/corrupt runs vanish from history instead
  of surfacing as corrupt artifacts -- making the run history
  look cleaner than the filesystem.
- Repro:
  1. Drop a truncated `tmp/stress-reports/stress-bad.json`.
  2. `curl -s localhost:49171/api/stress/reports` omits it.
- Severity: nice-to-have.
- Playwright: `play_stress.spec.ts::reports_endpoint_surfaces_corrupt_files`

## Not a finding

- `/x/resource-usage`: real `psutil` CPU/RSS for discovered
  processes -- weak (not hardware telemetry) but not fabricated.
