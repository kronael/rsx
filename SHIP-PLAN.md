# RSX Playground — Full System Debug & Verification Plan

## Goal

Make every page, every API endpoint, and every user flow in
the RSX playground actually work end-to-end. The playground
is at `http://localhost:49171`. It has 13 tabs (Overview,
Topology, Book, Risk, WAL, Logs, Control, Faults, Verify,
Orders, Stress, Docs, Trade). It manages RSX exchange
processes (gateway, risk, matching engine, marketdata, mark)
and lets you submit orders, view orderbooks, inspect WAL
files, inject faults, and run stress tests.

**Your job**: Start the server, hit every endpoint, start
RSX processes, submit orders, and verify data flows through
the whole pipeline. Fix any 500s, blank pages, missing data,
broken buttons, or incorrect responses you find. When done,
all 223 Playwright tests must pass and every screen must
show meaningful content (not just placeholders).

**How to work**: Go phase by phase. Use `curl` to test APIs.
Read server logs for errors. When something breaks, read the
relevant code in `server.py` / `pages.py` / `stress_client.py`,
fix it, then re-test. The code is in `/home/onvos/sandbox/rsx/`.
The playground code is in `rsx-playground/`. The trade UI is
a React SPA in `rsx-webui/` served at `/trade/`.

**Do not skip phases.** If processes won't start (phase 2),
debug why before moving to phase 4 (orders). Each phase
builds on the previous one.

## Deployment

- **Playground server**: `http://localhost:49171`
- **Start**: `cd rsx-playground && uv run server.py`
- **Gateway WS**: `ws://localhost:8080` (proxied at `/ws/private`)
- **Marketdata WS**: `ws://localhost:8081` (proxied at `/ws/public`)
- **REST proxy**: `/v1/*` -> `http://localhost:8080/v1/*`
- **Trade UI SPA**: `http://localhost:49171/trade/`
- **Postgres**: `postgres://rsx:folium@10.0.2.1:5432/rsx_dev`

### Reverse Proxy (krons.cx/rsx-play/)

All URLs are relative — works behind any prefix. Env vars:
```
GATEWAY_URL=ws://localhost:8080
MARKETDATA_WS=ws://localhost:8081
GATEWAY_HTTP=http://localhost:8080
```

Nginx example:
```nginx
location /rsx-play/ {
    proxy_pass http://127.0.0.1:49171/;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
}
```

### Market Maker

Python market maker in `rsx-playground/market_maker.py`.
Connects via gateway WS, places two-sided quotes.

```bash
# via API
curl -X POST http://localhost:49171/api/maker/start
curl http://localhost:49171/api/maker/status
curl -X POST http://localhost:49171/api/maker/stop
```

Or use the Control tab in the playground UI.

## Process Architecture (minimal scenario)

```
me-pengu    -> rsx-matching  (symbol_id=10)
mark        -> rsx-mark      (stream_id=100)
risk-0      -> rsx-risk      (shard=0)
gateway     -> rsx-gateway   (WS 8080)
marketdata  -> rsx-marketdata (WS 8081)
```

## Phase 1: Server Startup & Static Pages

### 1.1 Verify server starts cleanly
```bash
cd rsx-playground && uv run server.py &
sleep 2
curl -s http://localhost:49171/healthz | python3 -m json.tool
```
Expected: `{"processes": N, "postgres": true/false}`

### 1.2 Test every page route loads (no 500s)
```bash
for path in / /overview /topology /book /risk /wal /logs \
  /control /faults /verify /orders /stress /trade/; do
  code=$(curl -s -o /dev/null -w "%{http_code}" \
    "http://localhost:49171${path}")
  echo "$path -> $code"
done
```
All must return 200. If `/trade/` returns 404, check
`rsx-webui/dist/` exists with `index.html` + `assets/`.

### 1.3 Test HTMX partials load (no crashes)
```bash
for ep in processes health key-metrics ring-pressure \
  core-affinity cmp-flows control-grid resource-usage \
  faults-grid wal-status wal-detail wal-files wal-lag \
  wal-rotation wal-timeline logs logs-tail error-agg \
  auth-failures book-stats live-fills trade-agg \
  position-heatmap margin-ladder funding risk-latency \
  reconciliation latency-regression order-trace \
  stale-orders recent-orders current-scenario \
  invariant-status verify stress-reports-list; do
  code=$(curl -s -o /dev/null -w "%{http_code}" \
    "http://localhost:49171/x/${ep}")
  echo "/x/$ep -> $code"
done
```
All must return 200. Any 500 = server crash = fix.

### 1.4 WAL data visible (even without processes)
```bash
# WAL files exist at tmp/wal/pengu/10/ and tmp/wal/mark/100/
curl -s http://localhost:49171/x/wal-status
curl -s http://localhost:49171/x/wal-detail
curl -s http://localhost:49171/x/wal-files
curl -s http://localhost:49171/x/wal-lag
```
Must show stream names (mark, pengu), file counts, sizes.
If empty: `scan_wal_streams()` uses `rglob("*.wal")` —
verify `tmp/wal/pengu/10/10_active.wal` exists.

## Phase 2: Build & Start RSX Processes

### 2.1 Build binaries
```bash
curl -X POST http://localhost:49171/api/build
# Or: cargo build --workspace
```
Verify all binaries exist:
```bash
ls target/debug/rsx-{matching,mark,risk,gateway,marketdata}
```

### 2.2 Start all processes (minimal scenario)
```bash
curl -X POST "http://localhost:49171/api/processes/all/start"
sleep 5
curl -s http://localhost:49171/api/processes | python3 -m json.tool
```
Expected: 5 processes all "running" with PIDs.
Watch for:
- Gateway binds `:8080` (WS + HTTP)
- Marketdata binds `:8081` (WS)
- Risk binds CMP port
- ME-pengu binds CMP port
- Mark binds CMP port

### 2.3 Verify process logs appear
```bash
curl -s http://localhost:49171/x/logs-tail
# Or: ls log/*.log
```
Each process should have a log file in `log/`.

### 2.4 Verify process table in UI
Visit `http://localhost:49171/overview`.
All 5 processes should show green "running" with
CPU/mem/uptime. If any show "stopped" check
`log/<name>.log` for startup errors.

## Phase 3: Gateway + REST API

### 3.1 Gateway health
```bash
curl -s http://localhost:8080/health
# Through proxy:
curl -s http://localhost:49171/v1/symbols
```
Expected: `{"M": [[10, "1", "100000", "PENGU"]]}` or
similar metadata response.

### 3.2 Account endpoint
```bash
curl -s http://localhost:49171/v1/account
```
May return error if no user created yet. That's OK.

### 3.3 Create test user (if postgres available)
```bash
curl -X POST http://localhost:49171/api/users/create
```
Then retry account/positions/orders endpoints.

## Phase 4: Order Submission Flow

### 4.1 Submit test order via playground API
```bash
curl -X POST http://localhost:49171/api/orders/test \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "symbol_id=10&side=buy&order_type=limit&price=100&qty=1&tif=GTC&user_id=1"
```
Check response for success or meaningful error.

### 4.2 Submit batch orders
```bash
curl -X POST http://localhost:49171/api/orders/batch
```
Should add 10 orders to recent_orders list.

### 4.3 Verify orders appear
```bash
curl -s http://localhost:49171/x/recent-orders
```
Should show the submitted orders in HTML table.

### 4.4 Check WAL received events
```bash
curl -s http://localhost:49171/x/wal-timeline
```
After order submission, WAL should contain BBO and/or
FILL records. If empty, orders may not be reaching ME.

### 4.5 Check book shows data
```bash
curl -s "http://localhost:49171/x/book?symbol_id=10"
curl -s http://localhost:49171/x/book-stats
curl -s http://localhost:49171/x/live-fills
```

## Phase 5: WebSocket Connections

### 5.1 Public WS (marketdata)
```bash
# Test through playground proxy
# In browser console at http://localhost:49171/trade/:
# ws = new WebSocket("ws://localhost:49171/ws/public")
# ws.onmessage = (e) => console.log(e.data)
# ws.send('{"sub":{"sym":10,"ch":["BBO","DEPTH","TRADES"]}}')
```
If marketdata process running, should receive BBO updates.
If not running, WS should close with code 1013.

### 5.2 Private WS (gateway)
```bash
# Similar test with ws://localhost:49171/ws/private
# Should receive heartbeat responses
```

## Phase 6: Trade UI (React SPA)

### 6.1 Page loads
Visit `http://localhost:49171/trade/`.
Check browser DevTools:
- Network: `index-*.js` and `index-*.css` load (200)
- Console: no JS errors
- If 404 on assets: vite base path mismatch

### 6.2 With gateway running
- Symbol dropdown should show "PENGU" (fetched from `/v1/symbols`)
- Connection dots: green for public+private WS
- BBO should show bid/ask prices
- Orderbook should populate
- Submit an order: Buy Limit 100 @ 1.0

### 6.3 Without gateway
- Symbol shows "Loading..."
- Connection dots: red
- BBO shows "--" dashes
- Order submission fails gracefully (toast error)

## Phase 7: Risk Page

### 7.1 Position heatmap
```bash
curl -s http://localhost:49171/x/position-heatmap
```
With fills in WAL: shows net positions per symbol.
Without fills: "no fill data available".

### 7.2 Margin ladder
```bash
curl -s http://localhost:49171/x/margin-ladder
```
Shows recent fills with price/qty/notional.

### 7.3 Funding
```bash
curl -s http://localhost:49171/x/funding
```
Shows BBO-derived spread/rate per symbol.

### 7.4 User lookup (requires postgres)
```bash
curl -s "http://localhost:49171/x/risk-user?user_id=1"
```
With postgres: user balances/positions.
Without: "postgres not connected" message.

### 7.5 Liquidation queue
```bash
curl -s http://localhost:49171/x/liquidations
```

## Phase 8: Stress Testing

### 8.1 With gateway running
```bash
curl -X POST "http://localhost:49171/api/stress/run?rate=10&duration=5"
```
Expected: JSON with submitted > 0, actual_rate > 0.

### 8.2 Without gateway
```bash
curl -X POST "http://localhost:49171/api/stress/run?rate=10&duration=5"
```
Expected: 502 error with "gateway unreachable" message.
Must NOT return empty results with 0 submitted.

### 8.3 Via HTMX (browser)
Visit `/stress`, click "Run Stress Test".
With gateway: shows green "completed" with metrics.
Without gateway: shows red error message.

## Phase 9: Verify & Invariants

### 9.1 Run verification
```bash
curl -X POST http://localhost:49171/api/verify/run
```

### 9.2 Check verify results
```bash
curl -s http://localhost:49171/x/verify
```
Should show 10 invariant checks with pass/fail/skip.

## Phase 10: Fault Injection

### 10.1 Kill a process
```bash
curl -X POST http://localhost:49171/api/processes/me-pengu/kill
```
Process should show "stopped" in overview.

### 10.2 Restart
```bash
curl -X POST http://localhost:49171/api/processes/me-pengu/restart
```
Process should restart and show "running".

### 10.3 Stop all
```bash
curl -X POST http://localhost:49171/api/processes/all/stop
```
All processes should stop cleanly.

## Phase 11: Playwright Tests

```bash
cd rsx-playground/tests && npx playwright test
```
All 223 tests must pass. These test:
- All 11 dashboard pages render
- HTMX polling attributes configured
- Empty states show correct messages
- Trade UI components functional
- Navigation works
- Order form interactions

## Known Issues to Watch

1. **Stale PID files**: If processes crash, PID files in
   `tmp/pids/` may point to dead processes. `scan_processes()`
   handles this but verify.

2. **Postgres optional**: Many features degrade gracefully
   without postgres (risk user lookup, liquidations, user
   creation). Not a bug.

3. **WAL files empty**: Fresh start has 0-byte WAL files.
   After orders flow through ME, WAL should grow.

4. **Port conflicts**: If old processes linger on 8080/8081,
   new ones fail to bind. `start_all()` runs `fuser -k` to
   clean up.

5. **Browser cache**: After rebuilding webui dist, hard
   refresh (`Ctrl+Shift+R`) may be needed.

## Phase 12: Proxy Prefix Verification (krons.cx/rsx-play)

Verify all URLs work behind reverse proxy at `/rsx-play/`.

### 12.1 Local proxy simulation
```bash
# Start server
cd rsx-playground && uv run server.py &

# Test with curl, stripping prefix like nginx would
for path in / /overview /book /trade/ /docs; do
  code=$(curl -s -o /dev/null -w "%{http_code}" \
    "http://localhost:49171${path}")
  echo "$path -> $code"
done
```

### 12.2 Verify no absolute URLs remain
```bash
# Should return zero matches (all relative)
grep -n 'href="/' rsx-playground/server.py
grep -n 'href="/' rsx-playground/pages.py
```

### 12.3 Trade UI asset paths
```bash
# Assets must use relative paths (./assets/*)
grep -o 'src="[^"]*"' rsx-webui/dist/index.html
grep -o 'href="[^"]*"' rsx-webui/dist/index.html
```
All asset references should start with `./` not `/`.

### 12.4 Market maker populates books
```bash
curl -X POST http://localhost:49171/api/maker/start
sleep 5
curl -s http://localhost:49171/api/maker/status
# Should show running=true, orders_placed > 0
curl -s "http://localhost:49171/x/book?symbol_id=10"
# Should show bid/ask levels
curl -X POST http://localhost:49171/api/maker/stop
```

### 12.5 Deploy to krons.cx
```bash
# On krons.cx:
rsync -avz rsx-playground/ krons.cx:~/rsx-playground/
ssh krons.cx "cd rsx-playground && uv run server.py &"

# Verify through proxy
curl -s https://krons.cx/rsx-play/healthz
curl -s https://krons.cx/rsx-play/overview
curl -s https://krons.cx/rsx-play/trade/
```

## Success Criteria

- [ ] Server starts, all 11+ pages load (200)
- [ ] All HTMX endpoints return 200
- [ ] WAL status shows streams (mark, pengu)
- [ ] Processes start/stop/restart via API
- [ ] Orders submit and appear in recent orders
- [ ] WAL timeline shows events after orders
- [ ] Risk page shows fill-derived data
- [ ] Stress test returns error when gateway down
- [ ] Trade UI loads, shows correct empty state
- [ ] Trade UI populates with gateway running
- [ ] Playwright 223/223 green
- [ ] No 500 errors in server console
- [ ] All URLs relative (no absolute hrefs)
- [ ] Works behind /rsx-play/ proxy prefix
- [ ] Market maker starts and populates book
- [ ] Trade UI WS connects through proxy
