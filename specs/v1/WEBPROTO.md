# WebSocket Wire Protocol (WS Overlay)

Gateway exposes a compact WebSocket protocol and translates
messages to CMP/WAL wire format for the risk engine. The
goal is minimal parsing cost and small payloads.

## Frame Shape

Each message is a JSON object with a single key. The key is the 1-letter message type and the value is a positional array payload.

Example:

```
{N:[sym, side, px, qty, cid, tif]}
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

### Failure Reason
- 0 = INVALID_TICK_SIZE
- 1 = INVALID_LOT_SIZE
- 2 = SYMBOL_NOT_FOUND
- 3 = DUPLICATE_ORDER_ID
- 4 = INSUFFICIENT_MARGIN
- 5 = OVERLOADED
- 6 = INTERNAL_ERROR
- 7 = REDUCE_ONLY_VIOLATION

### Authentication

Auth is via WebSocket upgrade headers only (JWT in
`Authorization` header). No in-band auth frame. Clients must
use the WS API. Connections without valid auth in upgrade
headers are rejected with HTTP 401 before WebSocket handshake
completes.

### N: New Order

```
{N:[sym, side, px, qty, cid, tif, ro]}
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
{S:[sym, channels]}     // subscribe (channels: 1=bbo, 2=depth)
{X:[sym, channels]}     // unsubscribe
{X:[0, 0]}              // unsubscribe all
```

**Server -> Client:**

```
{BBO:[sym, bp, bq, bc, ap, aq, ac, ts, u]}      // BBO update
{B:[sym, [[p,q,c], ...], [[p,q,c], ...], ts, u]} // L2 snapshot
{D:[sym, side, p, q, c, ts, u]}                 // L2 delta
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
| BBO.ts | BboRecord.timestamp_ns |
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

## Notes

- Gateway multiplexes many users over a single CMP/UDP link to
  the risk engine.
- Risk engine multiplexes orders over a single CMP/UDP link to
  each matching engine.
- Backpressure is enforced at ingress. If the gateway buffer
  is full, it rejects new orders with OVERLOADED.
