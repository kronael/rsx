# CRITIQUE.md — RSX Playground

Comprehensive audit of functionality gaps, test quality,
and implementation issues.

## 1. Critical: No Authentication

No endpoint requires authentication. Anyone can:

- `POST /api/users/create` — create funded accounts
  (hardcoded 1T balance)
- `POST /api/users/{id}/deposit` — deposit to any user
- `POST /api/risk/liquidate` — liquidate any user
- `POST /api/processes/all/stop` — kill exchange
- `POST /api/maker/stop` — kill market maker
- HX-Request header bypasses confirm guards (server.py
  ~line 1102)

This is a playground so auth isn't blocking, but
destructive endpoints should at minimum require
`confirm=yes` consistently.

## 2. High: Race Conditions in Global State

All in-memory state is unprotected globals:

```
recent_orders: list[dict]      # concurrent append+trim
recent_fills: list[dict]       # concurrent append+trim
_user_balances: dict[int, int] # concurrent read+write
_liquidation_log: list[dict]   # unbounded growth
_user_frozen: set[int]         # concurrent add/remove
_book_snap: dict[int, dict]    # WS subscriber vs sim
```

`_trim_recent_orders()` after `append()` is not atomic.
Two concurrent requests can both trim, losing entries.
Fix: use `asyncio.Lock` or collections.deque(maxlen=N).

## 3. High: Blocking I/O in Async Context

- `subprocess.run(["cargo", "build", ...])` blocks the
  event loop for 30-60s during builds (server.py ~913)
- `Path.glob()` and `read_bytes()` in WAL parsing block
  on every 1-2s HTMX refresh
- `parse_wal_fills()` rescans all WAL files on every call
  to `/x/live-fills` (every 1s)

Fix: use `asyncio.create_subprocess_exec()` for builds,
`asyncio.to_thread()` for file I/O, add caching with
TTL for WAL parsing.

## 4. High: pg_query() Return Type Inconsistency

```python
async def pg_query(sql, *args):
    if pg_pool is None: return None       # NoneType
    try: return [dict(r) for r in rows]   # list[dict]
    except: return {"error": str(e)}      # dict
```

Callers expect list but may get None or error dict.
Triple-checking `if data and isinstance(data, list)`
everywhere is fragile. Fix: return (data, error) tuple
or raise exceptions.

## 5. High: Input Validation Gaps

`/api/orders/test` form parsing:
- symbol_id: no bounds check (999999 crashes lookup)
- price: can be negative (invalid WAL record)
- qty: can be 0 or negative
- order_type: no enum validation
- tif: accepts any string

Fix: validate at form parse, before sim or gateway.

## 6. High: Market Maker Resource Leaks

- `_cancel_all()` clears `active_cids` even if cancel
  request fails — on restart, old orders are orphaned
- Circuit breaker on marketdata never restarts — maker
  quotes stale prices forever after 8 WS failures
- Quote circuit breaker aborts maker entirely after 10
  failures instead of retrying with longer backoff
- WebSocket error mid-quote leaves partial order state

## 7. Medium: Performance Issues

- WAL parsing is O(N) per request, called every 1-2s
  via HTMX refresh. With 10k WAL records = 10k parses/s
- No caching layer for WAL data (add TTL cache)
- `scan_processes()` shells out to `ps` on every call
- `/x/position-heatmap` re-parses 500 fills every 2s

## 8. Test Quality Issues

### Vacuous Tests

- `play_logs.spec.ts:18-31` — asserts option count
  without verifying which options
- `play_overview.spec.ts:46` — asserts content length
  > 50 chars (any HTML passes)
- `play_control.spec.ts:22-38` — selectOption doesn't
  throw is the only assertion
- `play_orders.spec.ts:56` — regex `/order|queued|
  accepted/` matches any page with "order" in it

### Misleading Test Names

- "submits valid order successfully" — doesn't verify
  order reached matching engine
- "recent orders auto-refresh every 2s" — only checks
  hx-trigger attribute, never verifies data changes
- "user lookup shows postgres not connected" — regex
  too broad: `/no data|not connected|postgres|error/i`

### Missing Edge Cases

- No test for negative price, qty=0, invalid symbol_id
- No test for concurrent identical orders from same user
- No test for WAL file corruption handling
- No test for session expiry during active test
- No WebSocket endpoint tests at all

### Flaky Timing

- `play_orders.spec.ts` uses hard `waitForTimeout(2000)`
  instead of waiting for state change
- `play_maker.spec.ts` beforeEach polls with 15s timeout
  using exponential backoff that may not converge
- `play_risk.spec.ts:108-147` submits orders then
  immediately navigates — race condition

### Redundant Tests

- Process table auto-refresh tested in both
  play_control.spec.ts and play_overview.spec.ts
- Topology polling tests are identical patterns
  (verifyPolling on different selectors)
- Log filter tests overlap between "quick filters"
  and "smart search"

## 9. Untested Endpoints (15+)

- `/api/logs/clear`
- `/api/verify/run` (Playwright)
- `/api/mark/prices` (Playwright)
- `/api/risk/insurance`
- `/api/stress/reports/{id}`
- `/ws/private`, `/ws/public`
- `/v1/symbols`, `/v1/candles`, `/v1/funding`
- `/v1/positions`, `/v1/fills`
- `/api/wal/{stream}/status`
- `/api/orders/{cid}/cancel`

## 10. Pages.py UI Issues

- Cards show "loading..." forever when endpoint fails
  (no timeout fallback message)
- No semantic HTML — tables without proper structure
- No ARIA labels on form inputs or interactive elements
- No error state rendering for HTMX 500 responses
- Inconsistent: some cards use h2, some h3 for titles

## 11. Maker Price Calculation Edge Case

```python
half_spread = max(1, mid * spread_bps // 10000)
```

When `mid < 10000/spread_bps`, floor division yields 0,
capped to 1 raw unit. For low-price symbols (BONK,
PEPE) with small spread_bps, spread collapses to
minimum tick. Fix: use rounding not floor:
`(mid * spread_bps + 5000) // 10000`

## Summary

| Severity | Count | Key Areas |
|----------|-------|-----------|
| Critical | 6 | No auth on destructive endpoints |
| High | 9 | Race conditions, blocking I/O, validation |
| Medium | 15 | Circuit breaker, flaky tests, perf |
| Low | 10 | Vacuous tests, accessibility |
