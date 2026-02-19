# PLAN

## goal

Make every RSX playground page, API endpoint, and user flow
work end-to-end with RSX processes actually running. No
mocks. No placeholders. Every panel shows live data.

## what is done (do not redo)

- All 13 page routes return 200
- All 40 `/x/*` HTMX partials return 200
- All 30 `/api/*` endpoints return 200/204
- 806 Python unit + API tests pass
- 223 Playwright tests pass (HTML structure only — no live data)
- No absolute hrefs (krons.cx proxy compatible)
- `/api/stress/run` returns 502 when gateway down
- `/trade/` SPA loads, assets use `./` prefix
- Market maker code complete, auto-start wired

## what is NOT done (the real work)

These are the gaps. Each task below is a discrete deliverable.
Workers: read your assigned task only. The plan is context.
Do not attempt to deliver the whole plan. Report done when
your single task verifies clean.

## tasks

### Phase A — Build & process startup

- [ ] Build all RSX binaries via `cargo build` for packages:
  rsx-gateway, rsx-risk, rsx-matching, rsx-marketdata, rsx-mark.
  Verify binaries exist in `target/debug/`. Fix any compile
  errors. Document build command in `rsx-playground/README.md`.

- [ ] Start all 5 processes via `/api/processes/all/start`.
  Verify `/api/processes` shows all as `running` with PIDs.
  Check `log/*.log` for startup errors. Fix any port conflicts
  or config issues. Acceptance: all 5 green in control grid.

### Phase B — Market maker live

- [ ] With processes running, verify market maker auto-starts
  (3s delay after startup). Check `/api/maker/status` returns
  `running`. Check `/x/book?symbol_id=10` shows 5 bid + 5 ask
  levels placed by user_id=99. Fix maker if quotes don't appear.

- [ ] Verify maker status card on Control page shows "running
  (pid X)" with meaningful metrics (orders_placed > 0 after
  10s). Extend `/x/maker-status` HTML to include orders_placed,
  active_orders, mid_price fields from `maker.status()` dict.

### Phase C — Order flow live

- [ ] Submit a single limit order via `/api/orders/test`
  (symbol_id=10, side=buy, price=100, qty=1, user_id=1).
  Verify response contains `accepted` (not `queued`).
  Verify order appears in `/x/recent-orders` table.
  Fix gateway routing or order serialization if rejected.

- [ ] Submit batch of 10 orders via `/api/orders/batch`.
  Verify WAL timeline (`/x/wal-timeline`) shows new events.
  Verify book page shows updated depth after fills.
  Verify `/x/live-fills` shows at least one fill event.

- [ ] OID trace: after submitting an order, use the returned
  client_id to query `/x/order-trace?oid=<cid>`. Verify
  lifecycle shows: submitted → routed → (filled or open).
  Fix trace endpoint if it returns empty for valid orders.

### Phase D — Stress test with live gateway

- [ ] Run stress test via `/api/stress/run?rate=50&duration=10`.
  Verify: submitted > 0, accept_rate > 0.8, report saved to
  `tmp/stress-reports/stress-*.json`. Fix stress_client.py
  if orders don't reach gateway or responses don't parse.

- [ ] Verify stress reports table at `/x/stress-reports-list`
  shows new row after run. Verify clicking report ID at
  `/stress/<report_id>` renders full detail page with
  latency bars and accept rate progress bar.

### Phase E — Playwright with live system

- [ ] Write `rsx-playground/tests/play_stress.spec.ts` with
  ~20 tests covering: stress form renders, run button triggers
  request, result shows metrics, reports table updates,
  report detail page loads. Tests run against live server
  with processes up.

- [ ] Update `play_orders.spec.ts` test "submits valid order
  successfully" to assert `accepted` only (not `accepted|queued`).
  Update "batch order submission" to assert fills appear in
  recent-orders. All 20 order tests must pass with gateway up.

- [ ] Update `play_book.spec.ts` tests to assert real bid/ask
  levels appear (not just "no book data" placeholder). With
  maker running, book must show depth. Update "live fills
  shows placeholder" to assert actual fills after orders.

- [ ] Run full Playwright suite with processes running:
  `npx playwright test`. All 243+ tests must pass.
  Zero failures. Fix any test that still accepts
  placeholder-only content when live data is available.

## worker boundary rules

Each worker delivers exactly ONE task above. The plan is
provided as context — do not attempt to complete adjacent
tasks or fix things outside your assigned scope. If your task
depends on something broken upstream (e.g. processes won't
start), report that as a blocker and stop. Do not improvise.

## success criteria (full system)

- All 5 RSX processes start and stay running
- Market maker places quotes, book shows live levels
- Single order submits → `accepted` response → appears in WAL
- Stress test (50 ord/s × 10s) saves report with metrics
- All 243+ Playwright tests pass with processes running
- Zero 500 errors in server console during full walkthrough
- `/api/maker/status` shows orders_placed > 0 after 30s
