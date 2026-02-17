# RSX Playground — Full System Debug & Verification

## Goal
Make every page, API endpoint, and user flow in the RSX playground work end-to-end. Fix all 500s, blank pages, broken buttons, and incorrect responses. All 223 Playwright tests must pass.

## Stack
- **Server**: Python (uv), `rsx-playground/server.py` + `pages.py`
- **Frontend**: HTMX partials + React SPA (`rsx-webui/`, Vite)
- **RSX processes**: Rust binaries (`rsx-matching`, `rsx-risk`, `rsx-gateway`, `rsx-marketdata`, `rsx-mark`)
- **Database**: Postgres at `postgres://rsx:folium@10.0.2.1:5432/rsx_dev` (optional)
- **WAL files**: `tmp/wal/pengu/10/` and `tmp/wal/mark/100/`

## IO Surfaces
| Surface | Address |
|---|---|
| Playground server | `http://localhost:49171` |
| Gateway WS/HTTP | `ws://localhost:8080`, `http://localhost:8080` |
| Marketdata WS | `ws://localhost:8081` |
| REST proxy | `/v1/*` → `http://localhost:8080/v1/*` |
| WS proxy (private) | `/ws/private` → `ws://localhost:8080` |
| WS proxy (public) | `/ws/public` → `ws://localhost:8081` |
| Trade UI SPA | `http://localhost:49171/trade/` |
| Reverse proxy | `https://krons.cx/rsx-play/` |

## Phases
1. **Server startup** — all 11+ pages return 200, all `/x/*` HTMX partials return 200
2. **Build & start RSX processes** — 5 processes running with PIDs via API
3. **Gateway + REST** — `/v1/symbols` returns metadata, account/positions work
4. **Order submission** — orders submit, appear in recent-orders, WAL grows
5. **WebSocket** — public+private WS proxy connects and streams data
6. **Trade UI** — React SPA loads, shows correct state with/without gateway
7. **Risk page** — position heatmap, margin ladder, funding show data from WAL
8. **Stress testing** — returns metrics with gateway, error without
9. **Verify & invariants** — 10 checks run and report pass/fail/skip
10. **Fault injection** — kill/restart/stop processes via API
11. **Playwright** — 223/223 tests pass
12. **Proxy prefix** — all URLs relative, works behind `/rsx-play/`

## Constraints
- Python server: `uv run server.py` from `rsx-playground/`
- Rust: debug builds only (`cargo build`, not `--release`)
- No absolute hrefs in server.py/pages.py/dist HTML
- Trade UI assets must use `./` relative paths
- Postgres is optional; features degrade gracefully without it
- Stress test must return 502 + error message when gateway is down (not silent zeros)

## Success Criteria
- [ ] Server starts, all 11+ page routes return HTTP 200
- [ ] All `/x/*` HTMX partial endpoints return HTTP 200
- [ ] WAL status shows streams: mark, pengu
- [ ] Processes start/stop/restart via `/api/processes/*`
- [ ] Orders submit and appear in `/x/recent-orders`
- [ ] WAL timeline shows events after order submission
- [ ] Risk page shows fill-derived data from WAL
- [ ] Stress test returns 502 when gateway unreachable
- [ ] Trade UI loads with correct empty state (no gateway)
- [ ] Trade UI populates BBO/orderbook when gateway running
- [ ] `npx playwright test`: 223/223 green
- [ ] Zero 500 errors in server console during full flow
- [ ] `grep -n 'href="/'` returns no matches in server.py/pages.py
- [ ] Trade UI dist assets use `./` prefix
- [ ] Market maker starts, places orders, populates book
- [ ] All flows work behind `/rsx-play/` reverse proxy prefix
