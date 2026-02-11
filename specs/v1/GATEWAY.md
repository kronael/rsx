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
   resumption -- client must re-query open orders.

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
- Max concurrent connections per user: 5

## Config

- Env-only. See rsx-gateway config module.

## Notes

Gateway contains no risk logic and no matching logic. It is
purely an adaptation layer between external clients and
internal CMP links.

- Cancel by `cid` requires gateway to keep a pending map
  of client_order_id -> order_id. Cancels by order_id are
  stateless.
