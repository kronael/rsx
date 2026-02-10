# Gateway Architecture

WebSocket ingress, CMP bridge to Risk/ME pipeline.

## Data Flow

```
Users (WS)                Gateway                   Risk (CMP)
+--------+          +------------------+          +----------+
| WS ord |--------->| WS Handler       |--------->| OrderReq |
| WS cxl |          |   |              |          +----------+
+--------+          |   v              |
    ^               | Protocol Parse   |          +----------+
    |               |   |              |<---------| Fills    |
    |               |   v              |          | Done     |
    |               | Validate         |          | Cancel   |
    |               |  tick/lot align  |          | Failed   |
    |               |  symbol bounds   |          +----------+
    |               |   |              |
    |               |   v              |
    |               | Rate Limit       |
    |               |  per-user token  |
    |               |  per-IP token    |
    |               |   |              |
    |               |   v              |
    |               | Circuit Breaker  |
    |               |   |              |
    |               |   v              |
    |               | CMP Sender       |
    +<--------------| WS Writer        |
                    +------------------+
```

## Auth

- JWT (HS256) from Authorization header
- X-User-Id fallback for dev/test
- Validated during WS handshake

## Rate Limiting

Token bucket per dimension:
- Per-user: order submission rate
- Per-IP: connection/request rate

## Circuit Breaker

- Tracks success/failure of order submissions
- Opens on high failure rate
- Returns "overloaded" error when open

## Protocol

Compact JSON over WebSocket. Single-letter keys:
- `N`: NewOrder, `C`: Cancel, `U`: OrderUpdate
- `F`: Fill, `E`: Error, `H`: Heartbeat
- `Q`: Liquidation, `S`: Subscribe, `X`: Unsubscribe

See [specs/v1/WEBPROTO.md](../specs/v1/WEBPROTO.md).

## Pending Order Tracking

- UUIDv7 order IDs (16 bytes)
- Tracked by oid and client_order_id (20 chars)
- 5min dedup window

## Status Code Mapping

| Code | Meaning |
|------|---------|
| 0 | Filled |
| 1 | Resting |
| 2 | Cancelled |
| 3 | Failed |

## Specs

- [specs/v1/WEBPROTO.md](../specs/v1/WEBPROTO.md)
- [specs/v1/GATEWAY.md](../specs/v1/GATEWAY.md)
- [specs/v1/RPC.md](../specs/v1/RPC.md)
- [specs/v1/MESSAGES.md](../specs/v1/MESSAGES.md)
