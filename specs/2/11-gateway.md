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

**Current (shipped).** Single-threaded monoio (io_uring) reactor.
`GatewayState` lives behind `Rc<RefCell<...>>`; no locks, no
cross-thread sharing. One connection = one spawned task. casting/UDP
send (to risk) and receive (responses) run on the **same reactor** as
the WS accept loop and per-connection handlers.

**Limitation — egress is starved by ingress (measured 2026-05-30).**
The casting-receive is one task on this shared reactor: it drains the
socket, routes each response, then `sleep(0).await` yields. Under
WS-ingress load it loses the scheduling race. A single-order trace:
the response left Risk/ME by ~0.57 ms (`me_out`) but did not reach the
gateway receive (`gateway_cmp_recv`) until ~4.8 ms — a **~4.2 ms wait
in the kernel UDP socket buffer** for the recv task's next reactor turn
(gap ≈ **0.8 ms p50 / 10 ms p90** over 1934 probes). Route+write after
recv is ~40 µs, and Risk/ME are sub-ms — so the latency-critical egress
path being on the same reactor as unbounded WS ingress *is* the e2e
latency. (`me_out → gateway_cmp_recv` is the whole gap.)

**Target — decouple egress from the WS reactor (NOT YET IMPLEMENTED).**
Run the casting receive on a **dedicated pinned thread** (busy-spin
`recv_from`, like the Risk/ME tiles), off the WS reactor. It decodes
each response, correlates by `order_id` to the owning connection via a
read-mostly `oid → connection` index, and pushes the response into that
connection's **SPSC outbox ring**, waking the connection's WS task. The
WS reactor keeps ingress (accept → read → validate → casting-send to
Risk) and, per connection, drains the outbox → writes the WS frame.

- Egress latency becomes independent of ingress load: the casting
  socket is drained in µs, not after a multi-ms reactor turn.
- State model: relaxes "no cross-thread sharing" to ONE SPSC handoff
  per connection + a read-mostly `oid→connection` index. The
  `cid→order_id` pending map stays ingress-owned (egress routes by
  `order_id`, which every response carries).
- Optional further isolation: shard WS connections across N pinned
  reactor threads so one high-rate connection cannot starve others'
  writes.

This mirrors the Risk tile (busy-spin hot loop ↔ tokio PG sidecar via
SPSC) and the system rule that a latency-critical path runs on its own
pinned, busy-spin core.

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
