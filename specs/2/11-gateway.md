---
status: shipped
---

# Gateway Service

Gateway adapts external clients to internal CMP. It owns
sessions, auth, rate limits, and ingress backpressure. The
wire protocol is defined in WEBPROTO.md.

## Responsibilities

- WebSocket ingress/egress (public + native)
- Auth and session tracking
- Rate limiting and overload rejection
- Basic field validation
- CMP/UDP forwarding to Risk and responses back to clients

## Protocol

- WebSocket frame formats: WEBPROTO.md
- Error codes and reject reasons: MESSAGES.md

## Backpressure

- If internal queues exceed limits, reject new orders with
  an OVERLOADED error.
- Gateway does not block on internal congestion.

## Connection Lifecycle

1. Client opens WebSocket with `Authorization: Bearer <JWT>`
   in upgrade headers.
2. Server validates JWT. Invalid/missing token -> HTTP 401,
   no WebSocket handshake.
3. On success, server sends initial `{H:[ts]}` heartbeat.
4. Server sends `{H:[ts]}` every 5s. Client must respond
   within 10s or server closes connection.
5. Client sends orders (`N`, `C`) and receives updates
   (`U`, `F`, `E`, `Q`).
6. Disconnect: client closes WS, or server closes on
   heartbeat timeout / fatal protocol violation.
7. Reconnection: client re-opens with fresh JWT. No session
   resumption -- query open orders via `{O:[]}` on WS or
   `GET /v1/orders` (see WEBPROTO.md, REST.md).

## Rate Limits

Per RPC.md:
- 10 orders/sec per user (token bucket, configurable)
- 100 orders/sec per IP
- 1000 orders/sec per gateway instance (total)

Exceeded -> `{E:[1006, "rate limited"]}` or
`ORDER_FAILED(RATE_LIMIT)` depending on stage.

## Limits

- Max WebSocket frame: 4 KB (text frames only, no binary)
- Max subscriptions per connection: 64 symbols
- Max concurrent WebSocket connections per user: 5

## Config

- Env-only. See rsx-gateway config module.

## REST API

Gateway serves a read-only REST API on the same port (8080)
alongside WebSocket. See [REST.md](REST.md) for full endpoint
reference.

- REST: `/health`, `/v1/*` -- read-only queries
- WebSocket: `/ws` -- orders, cancels, live updates

Same JWT auth for both. REST is for simple reads (account,
positions, open orders, fills, funding, symbol metadata).
Order placement stays on WebSocket.

REST rate limits (5/sec per user, 50/sec per IP) are
tracked separately from WebSocket order rate limits.

## Notes

Gateway contains no risk logic and no matching logic. It is
purely an adaptation layer between external clients and
internal CMP links.

- Cancel by `cid` requires gateway to keep a pending map
  of client_order_id -> order_id. Cancels by order_id are
  stateless.

## Post-MVP

The following are deferred beyond v1:

- WS query messages: O, P, A, FL, FN, M
  (open orders, positions, account, fill history,
  funding history, metadata)
- Market data routing through gateway (v1 uses
  separate rsx-marketdata service directly)
- Separate public WS endpoint (no-auth market data)
