# PLAN

## goal

Fix every broken endpoint, page, and user flow in the RSX playground so all
223 Playwright tests pass and every screen shows meaningful content.

## approach

Work phase by phase: fix Python server (server.py/pages.py/stress_client.py)
so all 13 page routes and ~35 /x/* HTMX partials return 200; verify WAL data
visibility and RSX process lifecycle; validate order flow, stress testing, risk
pages, and the React Trade UI. Fix bugs as encountered with curl verification
after each fix, then run the full Playwright suite.

## tasks

- [ ] Start playground server, verify /healthz returns 200; fix any
      import/startup errors in server.py (missing deps, bad imports).
- [ ] Curl all 13 page routes (/, /overview, /topology, /book, /risk,
      /wal, /logs, /control, /faults, /verify, /orders, /stress, /trade/)
      — all must return 200. Fix any 404/500.
- [ ] Curl every HTMX partial /x/* endpoint (~35 endpoints). Fix any
      500 errors (bad template calls, missing functions, import errors
      in pages.py).
- [ ] Verify WAL partials (/x/wal-status, /x/wal-detail, /x/wal-files,
      /x/wal-lag) show stream names (mark, pengu). Confirm
      tmp/wal/pengu/10/ and tmp/wal/mark/100/ structures exist.
- [ ] Run cargo build --workspace (debug). Verify all 5 binaries exist
      in target/debug/ (rsx-matching, rsx-mark, rsx-risk, rsx-gateway,
      rsx-marketdata).
- [ ] Start all processes via POST /api/processes/all/start. Verify 5
      processes show "running" with PIDs. Check log/*.log for startup errors.
- [ ] Test gateway REST proxy: curl /v1/symbols through playground proxy.
      Verify gateway health responds at :8080.
- [ ] Submit test order via POST /api/orders/test. Verify order appears
      in /x/recent-orders and /x/wal-timeline shows events after order.
- [ ] Submit batch orders via POST /api/orders/batch. Verify multiple
      orders appear in /x/recent-orders partial.
- [ ] Verify /x/book, /x/book-stats, /x/live-fills show data after
      orders submitted. Check /x/position-heatmap and /x/margin-ladder.
- [ ] Fix stress test (/api/stress/run): must return 502 + error message
      when gateway is down, not silent empty 200.
- [ ] Verify Trade SPA at /trade/ loads (200). Confirm rsx-webui/dist/
      has index.html + assets/. Fix vite base path so asset refs use
      ./ prefix not /.
- [ ] Test market maker API (POST /api/maker/start, GET /api/maker/status,
      POST /api/maker/stop). Verify maker places orders and /x/book
      shows bid/ask levels.
- [ ] Test fault injection: kill me-pengu via POST /api/processes/me-pengu/kill,
      confirm "stopped"; then restart and confirm "running".
      Stop all via POST /api/processes/all/stop.
- [ ] Run POST /api/verify/run and check /x/verify shows 10 invariant
      checks with pass/fail/skip states.
- [ ] Audit href= and src= in server.py, pages.py, rsx-webui/dist/index.html.
      Fix any absolute URLs (/assets/*, /api/*) to relative paths for proxy compat.
- [ ] Run full Playwright suite: cd rsx-playground/tests && npx playwright test.
      Fix failures until 223/223 green.
