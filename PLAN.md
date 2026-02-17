# PLAN

## goal
Debug and fix the RSX playground so every page, API endpoint, and user
flow works end-to-end, with all 223 Playwright tests passing.

## approach
Work phase-by-phase: first fix server startup and all static/HTMX routes
(no 500s), then fix process management and order submission so data flows
through the whole pipeline. Finally verify the React trade UI, stress
testing, and proxy-relative URLs all work correctly before running the
full Playwright suite.

## tasks
- [ ] Audit server.py and pages.py: map all routes and HTMX partials to
      handlers, identify 500 sources (bad imports, wrong paths, unhandled
      exceptions)
- [ ] Fix server startup: ensure imports resolve, WAL scan paths match
      tmp/wal/pengu/10/ and tmp/wal/mark/100/, healthz returns valid JSON
- [ ] Fix all page routes (/, /overview, /topology, /book, /risk, /wal,
      /logs, /control, /faults, /verify, /orders, /stress, /docs, /trade/)
      to return HTTP 200
- [ ] Fix all /x/* HTMX partial endpoints to return 200; add graceful
      empty-state HTML where data is absent
- [ ] Fix WAL partials (wal-status, wal-detail, wal-files, wal-lag,
      wal-rotation, wal-timeline) to read WAL files correctly
- [ ] Fix process management API: correct binary paths, env vars, PID file
      handling, stale-PID cleanup for start/stop/restart/kill
- [ ] Fix order submission (/api/orders/test, /api/orders/batch) and
      verify orders appear in /x/recent-orders
- [ ] Fix stress test to return 502 + error message when gateway is
      unreachable (not silent zero counts)
- [ ] Fix risk page partials (position-heatmap, margin-ladder, funding,
      risk-user, liquidations) to read WAL fill records correctly
- [ ] Fix market maker API (/api/maker/start|stop|status) to launch
      market_maker.py subprocess correctly
- [ ] Fix verify endpoint (/api/verify/run, /x/verify,
      /x/invariant-status) to run all 10 invariant checks
- [ ] Fix fault injection endpoints (/x/faults-grid,
      /x/current-scenario, kill/restart via API)
- [ ] Remove all absolute href="/..." from server.py and pages.py;
      replace with relative paths
- [ ] Build rsx-webui dist with correct vite base so dist/index.html
      asset refs use "./" prefix; verify /trade/ serves the SPA
- [ ] Fix REST and WS proxy (/v1/*, /ws/private, /ws/public) to forward
      correctly to gateway on :8080 and marketdata on :8081
- [ ] Run Python API tests (tests/api_*_test.py) and fix failures
- [ ] Run Playwright suite (npx playwright test) in rsx-playground/tests/
      and rsx-webui/tests/; fix failures until 223/223 pass
