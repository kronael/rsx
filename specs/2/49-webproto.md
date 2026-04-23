---
status: shipped
---

# WebSocket Wire Protocol (WS Overlay)

Gateway exposes a compact WebSocket protocol and translates
messages to CMP/WAL wire format for the risk engine. The
goal is minimal parsing cost and small payloads.

## Table of Contents

- [Frame Shape](#frame-shape)
- [Types](#types)
- [Enums](#enums)
- [N: New Order](#n-new-order)
- [C: Cancel](#c-cancel)
- [U: Order Update / Ack](#u-order-update--ack)
- [F: Fill](#f-fill)
- [E: Error](#e-error)
- [H: Heartbeat](#h-heartbeat)
- [Market Data Messages](#market-data-messages-public-ws-see-marketdatamd)
- [Q: Liquidation Event](#q-liquidation-event-private-ws-see-liquidatormd)
- [T: Trade](#t-trade-public-ws)
- [M: Metadata Query](#m-metadata-query-public-ws)
- [Reconnection](#reconnection)
- [O: Open Orders Query](#o-open-orders-query-private-ws)
- [P: Positions Query](#p-positions-query-private-ws)
- [A: Account Summary Query](#a-account-summary-query-private-ws)
- [FL: Fill History Query](#fl-fill-history-query-private-ws)
- [FN: Funding History Query](#fn-funding-history-query-private-ws)
- [Notes](#notes)

---

## Frame Shape

Each message is a JSON object with a single key. The key
is the 1-letter message type and the value is a positional
array payload.

Example:

```
{N:[sym, side, px, qty, cid, tif, ro, po]}
```

Rules:
- Exactly one key per frame.
- Key is a single ASCII letter.
- Value is a JSON array with fixed positional fields for that type.
- No permessage-deflate or other compression.

ACK semantics:
- There is no "order accepted" ACK.
- The first response is an order update/fill from the matching engine path.
- Orders may become stale; clients should cancel and forget if no update arrives within their timeout.

## Types

| Name | Wire type | Description |
|------|-----------|-------------|
| symbol_id | uint32 | Symbol identifier |
| price | int64 | Price in tick units (tick_size from metadata) |
| qty | int64 | Quantity in lot units (lot_size from metadata) |
| oid | string(32) | Server order id, UUIDv7 as 32-char hex |
| cid | string(20) | Client order id, zero-padded 20 chars |
| ts | uint64 | Nanosecond timestamp (Unix epoch) |
| fee | int64 | Fee in tick units (negative = rebate) |
| side | uint8 | Enum `Side` |
| tif | uint8 | Enum `Time in Force` |
| seq | uint64 | Monotonic event sequence per symbol |

## Enums

### Side
- 0 = BUY
- 1 = SELL

### Time in Force (tif)
- 0 = GTC
- 1 = IOC
- 2 = FOK

### Order Status
- 0 = FILLED
- 1 = RESTING
- 2 = CANCELLED
- 3 = FAILED

OrderDone.final_status mapping (CMP -> WS):
- 0 -> FILLED
- 1 -> RESTING (unexpected for done, but forwarded)
- 2 -> CANCELLED

### Failure Reason
- 0 = INVALID_TICK_SIZE
- 1 = INVALID_LOT_SIZE
- 2 = SYMBOL_NOT_FOUND
- 3 = DUPLICATE_ORDER_ID
- 4 = INSUFFICIENT_MARGIN
- 5 = OVERLOADED
- 6 = INTERNAL_ERROR
- 7 = REDUCE_ONLY_VIOLATION
- 8 = POST_ONLY_REJECT
- 9 = RATE_LIMIT
- 10 = TIMEOUT
- 11 = USER_IN_LIQUIDATION
- 12 = WRONG_SHARD

Risk reject mapping (CMP -> WS):
- InsufficientMargin -> INSUFFICIENT_MARGIN
- UserInLiquidation -> USER_IN_LIQUIDATION
- NotInShard -> WRONG_SHARD

### Authentication

Auth is via WebSocket upgrade headers only (JWT in
`Authorization` header). No in-band auth frame. Clients must
use the WS API. Connections without valid auth in upgrade
headers are rejected with HTTP 401 before WebSocket handshake
completes.

### Reconnection

On reconnect, client opens a fresh WebSocket with a new JWT.
There is no session resumption and no replay of missed
messages. To restore state after reconnect:

1. Use the REST endpoints (`GET /v1/orders`, `GET /v1/positions`,
   `GET /v1/account`) to query current state. On the playground
   these are served by rsx-playground at `/v1/orders` etc.
   The `{O:[]}`, `{P:[]}`, `{A:[]}` WS queries are Post-MVP
   (not implemented on the gateway WS).
2. Re-subscribe to market data channels on the public WS
   (separate MARKETDATA service, see MARKETDATA.md).

### O: Open Orders Query (Private WS)

> **Post-MVP: not implemented in v1.**

Client request:
```
{O:[]}
```

Server response:
```
{O:[[oid, cid, sym, side, px, qty, filled,
     status, tif, ro, po, ts], ...]}
```

Fields per order:
- `oid`: server order id (string, 32-char hex)
- `cid`: client order id (string, 20 chars)
- `sym`: symbol id (uint32)
- `side`: enum `Side`
- `px`: price in tick units (int64)
- `qty`: total quantity in lot units (int64)
- `filled`: filled quantity in lot units (int64)
- `status`: enum `Order Status`
- `tif`: enum `Time in Force`
- `ro`: reduce-only (0 or 1)
- `po`: post-only (0 or 1)
- `ts`: created timestamp, nanoseconds (uint64)

Returns open orders only. Not on hot path -- gateway reads
from cached state or Postgres.

### P: Positions Query (Private WS)

> **Post-MVP: not implemented in v1.**

Client request:
```
{P:[]}
```

Server response:
```
{P:[[sym, side, qty, entry_px, mark_px,
     unrealized_pnl, liq_px], ...]}
```

Fields per position:
- `sym`: symbol id (uint32)
- `side`: enum `Side`
- `qty`: position size in lot units (int64)
- `entry_px`: average entry price in tick units (int64,
  computed as entry_cost / qty)
- `mark_px`: current mark price in tick units (int64)
- `unrealized_pnl`: unrealized PnL in tick units (int64)
- `liq_px`: estimated liquidation price in tick units (int64)

Returns non-zero positions only.

### A: Account Summary Query (Private WS)

> **Post-MVP: not implemented in v1.**

Client request:
```
{A:[]}
```

Server response:
```
{A:[collateral, equity, unrealized_pnl,
    initial_margin, maint_margin, available]}
```

Fields:
- `collateral`: deposited collateral in tick units (int64)
- `equity`: collateral + unrealized PnL (int64)
- `unrealized_pnl`: total unrealized PnL (int64)
- `initial_margin`: total initial margin (int64)
- `maint_margin`: total maintenance margin (int64)
- `available`: available balance for new orders (int64)

All query responses use tick units (raw i64). Clients apply
tick_size/lot_size for display.

### FL: Fill History Query (Private WS)

> **Post-MVP: not implemented in v1.**

Client request:
```
{FL:[sym, limit, before]}
```

Fields:
- `sym`: symbol id filter (uint32, 0 = all symbols)
- `limit`: max results (uint32, default 50, max 500)
- `before`: cursor timestamp in nanoseconds (uint64, 0 = latest)

Server response:
```
{FL:[[oid, sym, px, qty, side, fee, is_maker, ts], ...]}
```

Fields per fill:
- `oid`: server order id (string, 32-char hex)
- `sym`: symbol id (uint32)
- `px`: fill price in tick units (int64)
- `qty`: fill quantity in lot units (int64)
- `side`: enum `Side`
- `fee`: fee in tick units (int64, negative = rebate)
- `is_maker`: 1 = maker, 0 = taker
- `ts`: nanosecond timestamp (uint64)

Sorted descending by `ts`.

### FN: Funding History Query (Private WS)

> **Post-MVP: not implemented in v1.**

Client request:
```
{FN:[sym, limit, before]}
```

Fields:
- `sym`: symbol id filter (uint32, 0 = all symbols)
- `limit`: max results (uint32, default 50, max 500)
- `before`: cursor timestamp in nanoseconds (uint64, 0 = latest)

Server response:
```
{FN:[[sym, amount, rate_bps, ts], ...]}
```

Fields per entry:
- `sym`: symbol id (uint32)
- `amount`: funding amount in tick units (int64)
- `rate_bps`: funding rate in basis points (int32)
- `ts`: nanosecond timestamp (uint64)

Sorted descending by `ts`.

### N: New Order

```
{N:[sym, side, px, qty, cid, tif, ro, po]}
```

Fields:
- `sym`: symbol id (uint32)
- `side`: enum `Side`
- `px`: price in tick units (int64)
- `qty`: quantity in lot units (int64)
- `cid`: client order id (fixed 20-char string, zero-padded)
- `tif`: enum `Time in Force`
- `ro`: reduce-only (0=normal, 1=reduce-only, optional,
  default 0)
- `po`: post-only (0=normal, 1=post-only, optional,
  default 0)

### C: Cancel

```
{C:[cid_or_oid]}
```

Fields:
- `cid_or_oid`: client order id (20-char string) or server
  order id (UUIDv7, 32-char hex). Server distinguishes by
  length: 20 chars = cid, 32 chars = oid.

### U: Order Update / Ack

```
{U:[oid, status, filled, remaining, reason]}
```

Fields:
- `oid`: server order id (UUIDv7 bytes, or string if client cannot handle bytes)
- `status`: enum `Order Status`
- `filled`: filled qty (int64)
- `remaining`: remaining qty (int64)
- `reason`: enum `Failure Reason`

### F: Fill

```
{F:[taker_oid, maker_oid, px, qty, ts, fee]}
```

Fields:
- `taker_oid`: server order id
- `maker_oid`: server order id
- `px`: price in tick units (int64)
- `qty`: quantity in lot units (int64)
- `ts`: nanosecond timestamp
- `fee`: fee charged to this user (signed int64, negative =
  rebate)

### E: Error

```
{E:[code, msg]}
```

Fields:
- `code`: error code
- `msg`: human readable error

**Parse error handling:** On malformed frame (missing fields,
unknown message type, wrong value types, empty arrays), server
sends `{E:[code, msg]}` describing the parse failure. Server
does NOT close the connection on parse errors — the client can
continue sending valid frames. Connection is closed only on:
fatal protocol violations (binary frame, oversized message),
or auth failure.

### H: Heartbeat

```
{H:[ts]}
```

Server sends `{H:[ts]}` every 5s. Client must respond with
`{H:[ts]}` (echoing its own timestamp) within 10s or server
closes connection. Client may also initiate heartbeats;
server echoes. Simultaneous heartbeats from both sides are
harmless (no sequence number needed, each side tracks its
own timeout independently).

Fields:
- `ts`: client or server timestamp (ms)

### Market Data Messages (Public WS, see [MARKETDATA.md](MARKETDATA.md))

Separate public WS endpoint (no auth required). Same frame shape.

**Client -> Server:**

```
{S:[sym, channels]}     // subscribe (channels bitmask:
                        //   1=bbo, 2=depth, 4=trades)
{X:[sym, channels]}     // unsubscribe
{X:[0, 0]}              // unsubscribe all
```

**Server -> Client:**

```
{BBO:[sym, bp, bq, bc, ap, aq, ac, ts, u]}      // BBO update
{B:[sym, [[p,q,c], ...], [[p,q,c], ...], ts, u]} // L2 snapshot
{D:[sym, side, p, q, c, ts, u]}                 // L2 delta
{T:[sym, px, qty, side, ts, u]}                  // trade
```

**B snapshot format:** In the `B` frame, the first array is
bids `[[price, qty, count], ...]` sorted descending by price.
The second array is asks `[[price, qty, count], ...]` sorted
ascending by price.

**WS <-> WAL record field mapping:**

| WS Field | WAL Record Field (MARKETDATA.md) |
|----------|----------------------------------|
| BBO.sym | BboRecord.symbol_id |
| BBO.bp | BboRecord.bid_px |
| BBO.bq | BboRecord.bid_qty |
| BBO.bc | BboRecord.bid_count |
| BBO.ap | BboRecord.ask_px |
| BBO.aq | BboRecord.ask_qty |
| BBO.ac | BboRecord.ask_count |
| BBO.ts | BboRecord.ts_ns |
| BBO.u | BboRecord.seq |
| D.sym | L2Delta.symbol_id |
| D.side | L2Delta.side |
| D.p | L2Delta.price |
| D.q | L2Delta.qty |
| D.c | L2Delta.count |
| D.ts | L2Delta.timestamp_ns |
| D.u | L2Delta.seq |

`u`: matching engine height (uint64, monotonic per symbol).
Gap detection: if `u` jumps > 1, re-subscribe for snapshot.
`u` is the WS alias for `seq` used in WAL records.
Server sends `B` snapshot on subscribe before any `D` deltas.

### Q: Liquidation Event (Private WS, see [LIQUIDATOR.md](LIQUIDATOR.md))

```
{Q:[sym, status, round, side, qty, price, slip_bps]}
// status: 0=started, 1=round_placed, 2=filled,
//         3=cancelled, 4=completed
```

Risk engine sends to gateway over CMP/UDP. Gateway routes to user
by user_id. Fire-and-forget delivery.

### T: Trade (Public WS)

```
{T:[sym, px, qty, side, ts, u]}
```

Fields:
- `sym`: symbol id (uint32)
- `px`: trade price in tick units (int64)
- `qty`: trade quantity in lot units (int64)
- `side`: taker side, enum `Side`
- `ts`: nanosecond timestamp (uint64)
- `u`: matching engine sequence (uint64, monotonic
  per symbol)

Sent to clients subscribed to channel 4 (trades) for
that symbol. Each fill produces one trade message.

### M: Metadata Query (Public WS)

> **Post-MVP: not implemented in v1.**

Client request:
```
{M:[]}
```

Server response:
```
{M:[[sym, tick, lot, name], ...]}
```

Fields per symbol:
- `sym`: symbol id (uint32)
- `tick`: tick size as human-readable string (e.g. "0.01")
- `lot`: lot size as human-readable string (e.g. "0.001")
- `name`: symbol name (string, e.g. "BTC-USD")

Returns all active symbols. Clients should query on connect
to obtain tick/lot sizes for order formatting.

## Notes

- Gateway multiplexes many users over a single CMP/UDP link to
  the risk engine.
- Risk engine multiplexes orders over a single CMP/UDP link to
  each matching engine.
- Backpressure is enforced at ingress. If the gateway buffer
  is full, it rejects new orders with OVERLOADED.
