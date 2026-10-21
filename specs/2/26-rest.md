---
status: partial
---

# REST API

The gateway's REST surface is read-only HTTP on the WebSocket port. Two
endpoints are built, `GET /health` and `GET /v1/symbols`; the account, position,
order, fill and funding reads below them are targets, not built.

Gateway serves REST alongside WebSocket on the same listener
(`RSX_GW_LISTEN`, 8080). A request that carries `Sec-WebSocket-Key` upgrades
on any path (clients use `/ws/private` and `/`); every other request is REST
(`rsx-gateway/src/ws.rs:60`, `rsx-gateway/src/handler.rs:68`).

REST is read-only. Order placement and cancellation stay on
WebSocket for latency.

## Built endpoints

Both answer `Content-Type: application/json` and close the connection.
Neither reads a JWT and neither is rate limited
(`rsx-gateway/src/rest.rs:73`).

### GET /health

```json
{"status":"ok","version":"0.2.0","uptime_sec":3600,"ts":1700000000000000000}
```

`version` is the gateway crate version, `uptime_sec` the seconds since the
gateway state was created, `ts` Unix nanoseconds
(`rsx-gateway/src/rest.rs:39`).

### GET /v1/symbols

One object per symbol id the gateway accepts, from its boot configuration
(`RSX_MAX_SYMBOLS`, `RSX_DEFAULT_TICK_SIZE`, `RSX_DEFAULT_LOT_SIZE`, the
per-symbol overrides):

```json
{"symbols":[{"id":0,"tick_size":1,"lot_size":1,
             "price_decimals":8,"qty_decimals":8}]}
```

The body has no symbol name and no status. `price_decimals` and `qty_decimals`
are 8 for every symbol (`rsx-gateway/src/rest.rs:50`,
`rsx-gateway/src/config.rs:52`). `rsx-maker` reads this shape
(`rsx-maker/maker.go:221`).

### Errors

A GET of another path answers HTTP 404 and any other method answers HTTP 400,
each with the WebSocket error shape:

```json
{"E":[404,"not found"]}
{"E":[400,"bad request"]}
```

A query string is ignored (`GET /health?x=1` is `/health`).

## Deferred endpoints (not built)

The gateway answers each of these with 404. The shapes are the target.
The rows use the compact-array notation of the order channel
(`49-webproto.md`).

### GET /v1/account

JWT required.

```
{"A":[collateral, equity, unrealized_pnl,
      initial_margin, maint_margin, available]}
```

### GET /v1/positions

JWT required.

```
{"P":[[sym, side, qty, entry_px, mark_px,
       unrealized_pnl, liq_px], ...]}
```

`liq_px` is an estimate assuming this is the user's only
position (ignores portfolio effects).

### GET /v1/orders

JWT required. Open orders only.

```
{"O":[[oid, cid, sym, side, px, qty, filled,
       status, tif, ro, po, ts], ...]}
```

### GET /v1/fills?symbol=&limit=&before=

JWT required. Query params:
- `symbol` (uint32, optional) -- filter by symbol_id
- `limit` (uint32, optional, default 50, max 500)
- `before` (uint64, optional) -- cursor, nanosecond ts

```
{"FL":[[oid, sym, px, qty, side, fee, is_maker, ts], ...]}
```

Sorted descending by `ts`. Use `before` for pagination.
`fee` is in quote currency. `is_maker`: 1=maker, 0=taker.

### GET /v1/funding?symbol=&limit=&before=

JWT required. Same query params as fills.

```
{"FN":[[sym, amount, rate_bps, ts], ...]}
```

Sorted descending by `ts`. Use `before` for pagination.

### Target: authentication, status codes and limits

- `Authorization: Bearer <JWT>`, the WebSocket JWT. Missing or invalid token
  returns HTTP 401 with `{"E":[401,"unauthorized"]}`.
- HTTP statuses: 200 success, 400 bad parameters, 401 unauthorized,
  404 not found, 429 rate limited, 500 internal error.
- 5 requests/sec per user and 50 requests/sec per IP, separate from the
  WebSocket order limits. Exceeded answers 429 with a `Retry-After` header.
- State comes from the risk engine's cache, off the hot path.

## CORS

Not needed. Frontend and API are same origin
(`rsx.krons.cx`). No cross-origin headers required.

## Notes

- Built responses are JSON objects. The deferred reads use the compact arrays
  and integer enums of the order channel.
- No POST/PUT/DELETE endpoints. Writes go through WebSocket.
