# WebSocket Wire Protocol (WS Overlay)

Gateway exposes a compact WebSocket protocol and translates messages to gRPC for the risk engine. The goal is minimal parsing cost and small payloads.

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

### A: Auth (optional fallback)

Primary auth is via WebSocket upgrade headers. This message is a fallback for clients that cannot set headers.

```
{A:[token, ts, nonce]}
```

Fields:
- `token`: JWT string
- `ts`: client timestamp (ms)
- `nonce`: client nonce

### N: New Order

```
{N:[sym, side, px, qty, cid, tif, ro]}
```

Fields:
- `sym`: symbol id (uint32)
- `side`: enum `Side`
- `px`: price in tick units (int64)
- `qty`: quantity in lot units (int64)
- `cid`: client order id (uint64)
- `tif`: enum `Time in Force`
- `ro`: reduce-only (0=normal, 1=reduce-only, optional,
  default 0)

### C: Cancel

```
{C:[cid_or_oid]}
```

Fields:
- `cid_or_oid`: client order id or server order id

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

### H: Heartbeat

```
{H:[ts]}
```

Server sends `{H:[ts]}` every 5s. Client must respond within
10s or server closes connection. Client may also initiate;
server echoes.

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

`u`: matching engine height (uint64, monotonic per symbol).
Gap detection: if `u` jumps > 1, re-subscribe for snapshot.
Server sends `B` snapshot on subscribe before any `D` deltas.

### Q: Liquidation Event (Private WS, see [LIQUIDATOR.md](LIQUIDATOR.md))

```
{Q:[sym, status, round, side, qty, price, slip_bps]}
// status: 0=started, 1=round_placed, 2=filled,
//         3=cancelled, 4=completed
```

Risk engine pushes to gateway SPSC ring. Gateway routes to user
by user_id. Fire-and-forget delivery.

## Notes

- Gateway multiplexes many users over a single gRPC stream to the risk engine.
- Risk engine multiplexes orders over a single gRPC stream to each matching engine.
- Backpressure is enforced at ingress. If the gateway buffer is full, it rejects new orders with OVERLOADED.
