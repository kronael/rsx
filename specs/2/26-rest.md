---
status: partial
---

# REST API

> **Status: v2 — deferred.** Only `/health` and `/v1/symbols`
> are implemented. All other endpoints are post-MVP targets.

Gateway serves REST alongside WebSocket on the same port
(8080). WS upgrades on `/ws`, REST on `/v1/*` and `/health`.

REST is read-only. Order placement and cancellation stay on
WebSocket for latency.

Responses use the same compact array format as WebSocket
(see WEBPROTO.md). REST routes map naturally to WS query
frames -- same data, HTTP transport.

## Endpoints

### GET /health

No auth required.

```json
{"status":"ok","version":"0.1.0","uptime_sec":3600,
 "ts":1700000000000000000}
```

### GET /v1/symbols

No auth required. Same as WS `{M:[]}`.
Symbol status: `"active"` (trading), `"paused"` (halted),
`"pending"` (not yet live).

```
{M:[[sym, tick, lot, name], ...]}
```

### GET /v1/account

JWT required. Same as WS `{A:[]}`.

```
{A:[collateral, equity, unrealized_pnl,
    initial_margin, maint_margin, available]}
```

### GET /v1/positions

JWT required. Same as WS `{P:[]}`.

```
{P:[[sym, side, qty, entry_px, mark_px,
     unrealized_pnl, liq_px], ...]}
```

`liq_px` is an estimate assuming this is the user's only
position (ignores portfolio effects).

### GET /v1/orders

JWT required. Same as WS `{O:[]}`.

```
{O:[[oid, cid, sym, side, px, qty, filled,
     status, tif, ro, po, ts], ...]}
```

Returns open orders only.

### GET /v1/fills?symbol=&limit=&before=

JWT required. Query params:
- `symbol` (uint32, optional) -- filter by symbol_id
- `limit` (uint32, optional, default 50, max 500)
- `before` (uint64, optional) -- cursor, nanosecond ts

```
{FL:[[oid, sym, px, qty, side, fee, is_maker, ts], ...]}
```

Sorted descending by `ts`. Use `before` for pagination.
`fee` is in quote currency. `is_maker`: 1=maker, 0=taker.

### GET /v1/funding?symbol=&limit=&before=

JWT required. Query params:
- `symbol` (uint32, optional) -- filter by symbol_id
- `limit` (uint32, optional, default 50, max 500)
- `before` (uint64, optional) -- cursor, nanosecond ts

```
{FN:[[sym, amount, rate_bps, ts], ...]}
```

Sorted descending by `ts`. Use `before` for pagination.

## Authentication

`Authorization: Bearer <JWT>` header. Same JWT as WebSocket.
Missing or invalid token returns HTTP 401:

```json
{E:[401, "unauthorized"]}
```

## Errors

Same `{E:[code, msg]}` format as WebSocket. HTTP status
codes map to error codes:

| HTTP Status | Meaning |
|-------------|---------|
| 200 | Success |
| 400 | Bad request (invalid params) |
| 401 | Unauthorized (missing/invalid JWT) |
| 404 | Not found |
| 429 | Rate limited |
| 500 | Internal error |

## Rate Limits

- 5 requests/sec per user (JWT)
- 50 requests/sec per IP
- Exceeded -> HTTP 429 with `Retry-After` header (seconds)

REST rate limits are separate from WebSocket order rate
limits.

## CORS

Not needed. Frontend and API are same origin
(`rsx.krons.cx`). No cross-origin headers required.

## Notes

- Same wire format as WebSocket (compact arrays, integer
  enums). See WEBPROTO.md for field definitions and enum
  mappings.
- No POST/PUT/DELETE endpoints. Writes go through WebSocket.
- Gateway reads state from risk engine cache. Not on hot
  path.
