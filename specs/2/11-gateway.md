---
status: shipped
---

# Gateway Service

Gateway adapts external clients to internal casting. It owns
sessions, auth, rate limits, and ingress backpressure. The
wire protocol is defined in WEBPROTO.md.

## Responsibilities

- WebSocket ingress/egress (the authenticated order channel)
- Auth and session tracking
- Rate limiting and overload rejection
- Basic field validation
- casting/UDP forwarding to Risk and responses back to clients

## Protocol

- WebSocket frame formats: WEBPROTO.md
- Error codes and reject reasons: MESSAGES.md

## Backpressure

- A full in-flight set (`RSX_GW_MAX_PENDING`, `Submitted` orders only), an open circuit breaker or the
  per-instance rate limit rejects a new order with `E[5,"overloaded"]`. A
  failed casting send to risk answers `E[8,"network error"]`. Each code names
  one condition class (`49-webproto.md` § Error frame codes).
- Gateway does not block on internal congestion.

## Runtime Model

One monoio (io_uring) reactor thread runs the accept loop, every connection
handler and the housekeeping loop (heartbeats, idle reap, pending sweep,
casting sender tick). All gateway state is one `Rc<RefCell<GatewayState>>`: no
locks, no cross-thread sharing (`rsx-gateway/src/main.rs`).

A second thread, `gw-cast-recv`, busy-spins the casting receive from Risk,
decodes each record and pushes it on an SPSC ring of 8192 entries. It rings a
doorbell socket; the reactor parks on that readiness (or a 1 ms housekeeping
timer), so the reactor never polls and the receive never waits behind a
connection (`rsx-gateway/src/cast_recv.rs`). `RSX_GW_CORE_ID` pins the reactor
and `RSX_GW_CAST_CORE_ID` pins the receive thread; an unset id leaves that
thread unpinned.

The reactor routes each record by `user_id`: `push_to_user` queues the
serialized frame on every open connection of that user and wakes its handler
(`rsx-gateway/src/state.rs`). A connection's outbound queue is bounded by
`RSX_GW_MAX_OUTBOUND`; a queue that would overflow closes the connection,
because an order update can be neither dropped nor coalesced. An update for a
user with no open connection is not stored.

**Why io_uring here, and the reactor's latency contract** (readiness vs
completion, the syscall budget, and the road to fully zero-copy RX/TX via
AF_XDP) — see `rsx-gateway/notes/io-model.md`.

### Sharded reactors (proposed, not built)

**Targets: 100k concurrent WS connections, µs-scale egress latency, no
cross-connection head-of-line blocking.** One reactor cannot meet the
connection target: one thread cycling 100k sockets is ~10 ms per lap, so a
connection waits up to ~10 ms for its turn. This is a scheduling limit, not a
tuning one (BUGS `GW-SHARDED-REACTORS`).

N pinned reactor threads, one per core. `SO_REUSEPORT` load-balances incoming
connections across the N listeners; a connection is pinned to one reactor for
its lifetime (a socket can't be written from another reactor). Each reactor
owns ~100k/N connections and its own `GatewayState` shard. The receive thread
would route by order id through a read-mostly `oid → (shard, connection)`
index into a per-connection SPSC outbox ring and wake the owning reactor,
whose only egress work is the WS write.

Latency floor (kernel-bypass): io_uring SQPOLL, registered fds and buffers,
multishot recv on the WS side; AF_XDP or DPDK on the casting UDP side;
`SO_BUSY_POLL`.

## Connection Lifecycle

1. Client opens WebSocket with `Authorization: Bearer <JWT>`
   in upgrade headers.
2. Server validates JWT. Invalid/missing token -> HTTP 401,
   no WebSocket handshake.
3. On success, server sends initial `{H:[ts]}` heartbeat.
4. Server sends `{H:[ts]}` every `RSX_GW_HEARTBEAT_INTERVAL_S` (5s). A
   connection with no inbound frame of any kind for `RSX_GW_IDLE_TIMEOUT_S`
   (10s) is closed; a client echoes `H` to stay.
5. Client sends orders (`N`, `C`) and receives updates
   (`U`, `F`, `E`, `Q`).
6. Disconnect: client closes WS, or server closes on
   heartbeat timeout / fatal protocol violation.
   Shutdown (SIGTERM, SIGINT): the gateway stops accepting, turns `/ready`
   false and answers every new order `E[5]`, while risk's answers keep
   reaching clients. When no `Submitted` order is left, or after
   `RSX_GW_ORDER_TIMEOUT_MS` plus one sweep interval (the sweep answers what
   is left), it sends close 1001 on every connection and exits within one
   more second.
7. Reconnection: client re-opens, with the same JWT until `exp`. No session
   resumption: the gateway cannot list open orders (the WS queries and the
   REST reads are post-MVP). Orders in flight when a connection closes
   continue; their updates reach any open connection of that user.

## TLS

TLS terminates at a reverse proxy in front of the gateway (`9-deploy.md`).
The gateway listens on plain WS and holds no certificate.

## JWT

- Algorithm: HS256. `RSX_GW_JWT_SECRET` must be set; the
  gateway refuses to start if the secret is empty or
  shorter than 32 bytes.
- Required claims: `exp` (expiry), `aud == "rsx-gateway"`,
  `iss == "rsx-auth"`. The `user_id` claim carries the user id;
  `sub` is read as the user id when `user_id` is absent.
- Optional claims: `nbf` (enforced when present).
- A token is reusable until `exp`: the gateway keeps no
  per-token state and ignores a `jti` claim.

## Rate Limits

Per RPC.md, token buckets; each new order or cancel spends one token:
- 10 requests/sec per user (`RSX_GW_RL_USER`)
- 100 requests/sec per IP (`RSX_GW_RL_IP`)
- 1000 requests/sec per gateway instance, all users and addresses
  (`RSX_GW_RL_INSTANCE`)

Per-user or per-IP exhausted -> `E[1006,"rate limited"]`; 1006 names this
condition only. Per-instance exhausted -> `E[5,"overloaded"]`.

The per-IP limiter map is bounded (default 10,000 entries)
and evicts the oldest IP (FIFO) when full. This caps memory
under source-IP rotation while preserving rate-limit state
for any single misbehaving IP long enough to be effective.

## Overload / Circuit Breaker

The gateway runs a fail-closed circuit breaker on the order
path. A failure is a casting send of an order to risk that
errors. After `RSX_GW_CB_THRESHOLD` (10) consecutive failures it
opens and rejects new orders with `E[5,"overloaded"]` until
`RSX_GW_CB_COOLDOWN_MS` (30s) elapses, then probes via a single
half-open attempt. Success closes it; failure re-opens it. Cancels
do not consult it.

## Limits

- Max WebSocket frame: 4 KB (text frames only, no binary)
- Max concurrent WebSocket connections per user: 5; the 6th is closed after
  the 101 with no error frame
- Subscriptions belong to marketdata (`16-marketdata.md`); the gateway has none

## Config

- Env-only. See rsx-gateway config module.

## REST API

Any HTTP request without `Sec-WebSocket-Key` is REST, on the same port as the
WebSocket (`RSX_GW_LISTEN`, default `0.0.0.0:8080`). Served today
(`rsx-gateway/src/rest.rs`):

- `GET /health` — status, version, uptime
- `GET /v1/symbols` — the gateway's tick and lot per symbol

Neither reads a JWT and neither is rate limited. Another method is 400, another
path 404, with the body `{"E":[code,"msg"]}`. The account, position, order,
fill and funding reads and the REST rate limits are post-MVP (`26-rest.md`).
A WebSocket upgrade is accepted on any path. Order placement stays on the
WebSocket.

## Notes

Gateway contains no risk logic and no matching logic. It is
purely an adaptation layer between external clients and
internal casting links.

- A cancel, by `cid` or by `oid`, looks up the sender's own pending entry:
  the `CancelRequest` needs the order's symbol, which the `C` frame lacks.
  An order that is not in the sender's pending set cannot be cancelled.

## Post-MVP

The following are deferred beyond v1:

- WS query messages: O, P, A, FL, FN, M
  (open orders, positions, account, fill history,
  funding history, metadata)
- Market data routing through gateway (v1 uses
  separate rsx-marketdata service directly)
- Separate public WS endpoint (no-auth market data)
