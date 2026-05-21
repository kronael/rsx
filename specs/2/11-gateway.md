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

## Runtime Model

- Single-threaded monoio (io_uring) reactor. `GatewayState`
  lives behind `Rc<RefCell<...>>`; no locks, no cross-thread
  sharing. One connection = one spawned task.
- CMP/UDP send (to risk) and receive (responses) run on the
  same reactor as WS handlers.

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

## JWT

- Algorithm: HS256. `RSX_GW_JWT_SECRET` must be set; the
  gateway refuses to start if the secret is empty or
  shorter than 32 bytes.
- Required claims: `exp` (expiry), `aud == "rsx-gateway"`,
  `iss == "rsx-auth"`. `sub` carries the user id (also
  accepted as a `user_id` claim).
- Optional claims: `nbf` (enforced when present), `jti`.
- `jti` replay protection: a bounded in-process `JtiTracker`
  exists but is not wired through the WS handshake in v1.
  Short-lived `exp` is the v1 mitigation for replay; a
  shared (Redis) tracker is the planned multi-replica
  hardening.

## Rate Limits

Per RPC.md:
- 10 orders/sec per user (token bucket, configurable)
- 100 orders/sec per IP
- 1000 orders/sec per gateway instance (total)

Exceeded -> `{E:[1006, "rate limited"]}` or
`ORDER_FAILED(RATE_LIMIT)` depending on stage.

The per-IP limiter map is bounded (default 10,000 entries)
and evicts the oldest IP (FIFO) when full. This caps memory
under source-IP rotation while preserving rate-limit state
for any single misbehaving IP long enough to be effective.

## Overload / Circuit Breaker

The gateway runs a fail-closed circuit breaker on the order
path. After `circuit_threshold` consecutive failures it
opens and rejects new orders with `OVERLOADED` until a
cooldown elapses, then probes via a single half-open
attempt. Success closes it; failure re-opens it.

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
