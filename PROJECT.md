# RSX Playground — Full System Debug & Verification

## Goal

Make every page, API endpoint, and user flow in the RSX playground work end-to-end. Fix all 500s, blank pages, missing data, and broken buttons. All 223 Playwright tests must pass.

## Stack

- **Server**: Python (uv), `rsx-playground/server.py` + `pages.py` + `stress_client.py`
- **UI**: HTMX partials + React SPA (`rsx-webui/dist/`) at `/trade/`
- **Exchange**: Rust binaries (`target/debug/rsx-{matching,mark,risk,gateway,marketdata}`)
- **Transport**: CMP/UDP (hot path), WAL/TCP (cold path), WebSocket (client-facing)
- **DB**: Postgres at `postgres://rsx:folium@10.0.2.1:5432/rsx_dev` (optional)
- **Tests**: Playwright (`rsx-playground/tests/`)

## IO Surfaces

| Surface | Address |
|---------|---------|
| Playground server | `http://localhost:49171` |
| Gateway WS + REST | `ws://localhost:8080`, `http://localhost:8080` |
| Marketdata WS | `ws://localhost:8081` |
| REST proxy | `/v1/*` → `http://localhost:8080/v1/*` |
| WS proxy (private) | `/ws/private` → `ws://localhost:8080` |
| WS proxy (public) | `/ws/public` → `ws://localhost:8081` |
| Trade SPA | `http://localhost:49171/trade/` |
| Reverse proxy | `https://krons.cx/rsx-play/` (nginx prefix strip) |

## Pages & Endpoints

**Pages (must return 200):** `/`, `/overview`, `/topology`, `/book`, `/risk`, `/wal`, `/logs`, `/control`, `/faults`, `/verify`, `/orders`, `/stress`, `/trade/`

**HTMX partials (`/x/*`):** `processes`, `health`, `key-metrics`, `ring-pressure`, `core-affinity`, `cmp-flows`, `control-grid`, `resource-usage`, `faults-grid`, `wal-status`, `wal-detail`, `wal-files`, `wal-lag`, `wal-rotation`, `wal-timeline`, `logs`, `logs-tail`, `error-agg`, `auth-failures`, `book-stats`, `live-fills`, `trade-agg`, `position-heatmap`, `margin-ladder`, `funding`, `risk-latency`, `reconciliation`, `latency-regression`, `order-trace`, `stale-orders`, `recent-orders`, `current-scenario`, `invariant-status`, `verify`, `stress-reports-list`

**REST APIs:** `/api/processes`, `/api/processes/all/start`, `/api/processes/all/stop`, `/api/processes/{name}/restart`, `/api/processes/{name}/kill`, `/api/build`, `/api/orders/test`, `/api/orders/batch`, `/api/stress/run`, `/api/verify/run`, `/api/maker/start`, `/api/maker/stop`, `/api/maker/status`, `/api/users/create`, `/healthz`

## Phases

1. Server startup + all pages/partials return 200
2. Build Rust binaries + start 5 RSX processes
3. Gateway REST proxy works (`/v1/symbols`, `/v1/account`)
4. Order submission → appears in recent orders → WAL grows
5. WebSocket proxy (public + private) functional
6. Trade SPA loads, connects WS, shows BBO/book
7. Risk page shows fill-derived data (heatmap, margin, funding)
8. Stress test returns error when gateway down (not silent 0)
9. Verify/invariant checks return 10 results
10. Fault injection: kill/restart process, stop all
11. Playwright 223/223 green
12. No absolute hrefs in server.py/pages.py; Trade SPA assets use `./`; works at `/rsx-play/` prefix

## Constraints

- Fix bugs in `server.py`, `pages.py`, `stress_client.py` only — do not rewrite
- Postgres is optional; features must degrade gracefully without it
- WAL files at `tmp/wal/pengu/10/` and `tmp/wal/mark/100/`
- Log files in `log/*.log`
- PID files in `tmp/pids/`
- All URLs must be relative (proxy-safe)
- Debug builds only (`cargo build`, not `--release`)

## Success Criteria

- [ ] All 13 pages return HTTP 200
- [ ] All HTMX partials return HTTP 200 (no 500s)
- [ ] WAL status shows `mark` and `pengu` streams
- [ ] 5 RSX processes start/stop/restart via API
- [ ] Test order submits → appears in `/x/recent-orders`
- [ ] `/x/wal-timeline` shows events after order submission
- [ ] `/api/stress/run` returns 502 + error message when gateway down
- [ ] Trade SPA loads and shows correct empty state without gateway
- [ ] Trade SPA shows BBO/book with gateway running
- [ ] Market maker starts, places orders, book shows bid/ask levels
- [ ] Playwright 223/223 pass
- [ ] Zero 500 errors in server console during full run
- [ ] `grep 'href="/'` returns 0 matches in server.py and pages.py
- [ ] Trade SPA asset paths start with `./` not `/`
- [ ] `curl https://krons.cx/rsx-play/healthz` returns 200
