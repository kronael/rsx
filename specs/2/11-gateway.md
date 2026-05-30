---
status: shipped
---

# Gateway Service

Gateway adapts external clients to internal casting. It owns
sessions, auth, rate limits, and ingress backpressure. The
wire protocol is defined in WEBPROTO.md.

## Responsibilities

- WebSocket ingress/egress (public + native)
- Auth and session tracking
- Rate limiting and overload rejection
- Basic field validation
- casting/UDP forwarding to Risk and responses back to clients

## Protocol

- WebSocket frame formats: WEBPROTO.md
- Error codes and reject reasons: MESSAGES.md

## Backpressure

- If internal queues exceed limits, reject new orders with
  an OVERLOADED error.
- Gateway does not block on internal congestion.

## Runtime Model

**Targets: 100k concurrent WS connections, µs-scale egress latency, no
cross-connection head-of-line blocking.** (Not yet implemented — the
build is currently a single monoio reactor; that cannot meet this: one
thread cycling 100k sockets is ~10 ms per lap, so a connection waits up
to ~10 ms for its turn. This is a scheduling limit, not a tuning one.)

**Sharded reactors.** N pinned reactor threads, one per core. A reactor
is one event loop: it asks io_uring which of its sockets are ready and
runs each handler in turn. `SO_REUSEPORT` load-balances incoming
connections across the N listeners; a connection is **pinned to one
reactor for its lifetime** (read *and* write on that reactor's io_uring
— a socket can't be written from another reactor). Each reactor owns
~100k/N connections and its own `GatewayState` shard behind
`Rc<RefCell<…>>` — no locks, no cross-thread sharing within a shard. A
noisy connection loads only its own shard; the others are unaffected.

**Decoupled egress.** The casting receive from Risk runs on a
**dedicated pinned busy-spin thread**, off every reactor, draining
`recv_from` in µs. It decodes each response, correlates by `order_id`
to the owning connection via a read-mostly `oid → (shard, connection)`
index, pushes the response into that connection's **SPSC outbox ring**,
and wakes the owning reactor — whose only egress work is the WS write.
Egress latency is therefore independent of ingress load: the response
never waits in the kernel socket buffer for a reactor turn.

- `cid → order_id` pending map is ingress-owned, per shard; egress
  routes by `order_id` (every response carries it).
- Per-connection outboxes are bounded; stream updates **coalesce**
  (latest BBO wins) instead of queueing — a slow client never blocks a
  reactor.

**Latency floor (kernel-bypass).** io_uring **SQPOLL** (no syscall per
op) + registered fds/buffers + multishot recv on the WS side; AF_XDP or
DPDK on the internal casting UDP side to skip the kernel network stack;
`SO_BUSY_POLL` so the NIC is polled, not interrupt-driven.

This mirrors the Risk/ME tiles: a latency-critical path runs on its own
pinned, busy-spin core, and cross-tile handoff is a single SPSC ring.

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
internal casting links.

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
