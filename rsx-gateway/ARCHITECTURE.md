# rsx-gateway Architecture

WebSocket gateway process. Accepts client connections,
validates orders, routes to Risk via CMP/UDP, pushes
fills/updates back to clients. Same listening port also
serves REST.

Specs: `specs/2/11-gateway.md`, `specs/2/49-webproto.md`.

## Runtime Model

Single monoio (io_uring) reactor on one thread. All gateway
state lives in `Rc<RefCell<GatewayState>>`; no locks, no
cross-thread sharing. Each connection runs as a monoio task
spawned from the accept loop. The main loop polls the CMP/UDP
receiver, ticks the CMP sender, sweeps the pending tracker,
broadcasts heartbeats, and reaps idle connections.

tokio is NOT used on the gateway hot path. The reference
sibling project `trader/monoio-client/` proves the same
pattern for client-side WS.

## Module Layout

| File | Purpose |
|------|---------|
| `main.rs` | Binary: monoio runtime, CMP recv loop, sweeps, heartbeat |
| `lib.rs` | Re-exports |
| `config.rs` | `GatewayConfig` + `load_gateway_config` from env, JWT secret enforcement |
| `state.rs` | `GatewayState`, `ConnectionState`, bounded IP limiter, symbol configs |
| `handler.rs` | Per-connection: HTTP read, REST/WS branch, handshake, frame loop, validation, CMP forward |
| `ws.rs` | WS handshake, JWT extract, frame read/write, 4KB frame cap |
| `rest.rs` | REST endpoints (`/health`, `/v1/symbols`) on the same listener |
| `protocol.rs` | `WsFrame` enum + JSON serialize/parse |
| `jwt.rs` | HS256 validation, `Claims`, `validate_jwt_with_claims`, `JtiTracker` |
| `rate_limit.rs` | Microsecond-resolution token bucket |
| `circuit.rs` | `CircuitBreaker` (Closed/Open/HalfOpen) |
| `pending.rs` | `PendingOrders` with VecDeque + maps, stale sweep |
| `order_id.rs` | UUIDv7 generation + hex codec |
| `convert.rs` | Tick/lot alignment checks |
| `route.rs` | CMP -> WS fan-out (Fill, OrderInserted, OrderDone, OrderCancelled, OrderFailed, Liquidation) |

## REST + WS on One Port

`handler.rs::handle_connection` reads the initial HTTP request
once. `ws::is_ws_upgrade` decides:

- WS upgrade -> `ws_handshake_from_request` (JWT extract +
  101 Switching Protocols) -> per-frame loop
- Otherwise -> `rest::handle_rest` writes the response and
  closes the connection

`/health` and `/v1/symbols` are the only REST surfaces today.

## JWT Authentication

`jwt::validate_jwt_with_claims` enforces all of:

- HS256 signature against `RSX_GW_JWT_SECRET`
- `exp` (expiry)
- `nbf` (not-before, when present) -- `validate_nbf = true`
- `aud == "rsx-gateway"`
- `iss == "rsx-auth"`

Boot refuses to start if `RSX_GW_JWT_SECRET` is shorter than
`JWT_SECRET_MIN_LEN = 32` bytes (HS256 floor). Empty secret
also exits 2.

`Claims` carries an optional `jti`. `JtiTracker` (FIFO-bounded
HashSet) is implemented and unit-tested, but **currently dormant
on the wire path**: `ws::extract_user_id` calls `validate_jwt`
(user_id only), not `validate_jwt_with_claims`, so jti replay
protection is not active. Tracked at `ws.rs:108`
(`TODO(13-A16Z-FIXES T1.3)`). Wiring is pending a decision on
per-process vs shared (Redis) replay state.

## Rate Limiting

Two layers checked in sequence on every `NewOrder` frame:

| Layer | Scope | Default capacity |
|-------|-------|------------------|
| Per-IP | `peer.ip()` from `accept()` | `RSX_GW_RL_IP = 100` |
| Per-user | authenticated `user_id` | `RSX_GW_RL_USER = 10` |

Either exhausted -> WS error code 1006 `"rate limited"`, no
queueing.

The per-IP limiter map is **bounded** at
`IP_LIMITER_MAX = 10_000` entries. A parallel
`VecDeque<IpAddr>` records insertion order; on overflow the
oldest IP is evicted FIFO. This prevents memory exhaustion
from a rotating-IP adversary while preserving normal-case
fairness. Covered by `tests/state_test.rs::ip_limiter_map_is_bounded`.

`rate_limit::RateLimiter` uses microsecond-resolution token
accounting (capacity * 1_000_000 internal units) for fair
sub-second refill.

## Circuit Breaker

`state.rs::CircuitBreaker` is fail-CLOSED on overload. States:
`Closed -> Open` on `threshold` consecutive failures;
`Open -> HalfOpen` after `cooldown`; `HalfOpen` allows one
probe; success -> `Closed`, failure -> back to `Open`.

When `circuit.allow()` returns false, the gateway responds
WS error code 5 `"overloaded"` and refuses to forward the
order to Risk. Defaults: `threshold=10`, `cooldown=30s`.

## Connection Limits

Hard cap of **5 concurrent connections per `user_id`**
(`state.rs::add_connection`). Sixth attempt is rejected at
handshake time.

## Frame Size Cap

Both `ws::ws_read_frame` and `ws::ws_read_frame_buf` reject
WS payloads larger than **4096 bytes** with `InvalidData`. No
fragmentation support; orders and cancels comfortably fit.

## Message Flow

```
Client (WS JSON)
    |
    v
+-- handler.rs ----------------------------------+
|  1. Wait readable (10ms timeout for poll)      |
|  2. Read frame (ws::ws_read_frame_buf, 4KB)    |
|  3. UTF-8 + protocol::parse                    |
|     NewOrder:                                   |
|  4. Per-IP + per-user limiter                  |
|  5. Circuit breaker `allow()`                  |
|  6. Symbol bound + tick/lot alignment          |
|  7. UUIDv7 oid; pending.push                   |
|  8. CmpSender::send_raw(ORDER_REQUEST, &bytes) |
|     (binary forward path; alloc-free)          |
+-------------------------------------------------+
    |
    v [CMP/UDP -> Risk]
  Risk -> Matching -> Risk
    |
    v [CMP/UDP -> Gateway]
+-- main.rs CMP loop ----------------------------+
|  Decode FillRecord/OrderInsertedRecord/...     |
|  Dispatch to route.rs                          |
+-- route.rs ------------------------------------+
|  Serialize WsFrame JSON, push_to_user          |
+-------------------------------------------------+
    |
    v
+-- handler.rs ----------------------------------+
|  drain_outbound -> ws_write_text (JSON)        |
+-------------------------------------------------+
```

The binary forward path (`OrderRequest` and `CancelRequest`
written via `send_raw` from a stack struct) is **allocation-
free** on the hot path. The outbound JSON fan-out path
allocates per frame (per-connection `VecDeque<String>`);
acceptable for the WS-JSON protocol per spec.

## Pending Order Tracking

`PendingOrders` (capacity = `RSX_GW_MAX_PENDING`, default
10k) tracks every order sent to Risk until a terminal
`OrderDone`/`OrderCancelled`/`OrderFailed` arrives. Stale
entries past `RSX_GW_ORDER_TIMEOUT_MS` (default 10s) are
swept every 100ms by the main loop. Push failure when full
returns WS error 1003 `"pending queue full"`.

## Heartbeats

Server sends WS heartbeat every `RSX_GW_HEARTBEAT_INTERVAL_S`
(default 5s). Per-connection idle reaper closes connections
with no activity for `RSX_GW_IDLE_TIMEOUT_S` (default 10s).
Per-connection heartbeat timeout is also enforced inside
`handle_connection`.

## Backpressure & Failure Modes

| Condition | Response |
|-----------|----------|
| Per-IP or per-user limiter empty | WS error 1006 |
| Circuit breaker open | WS error 5 (overloaded) |
| Pending queue full | WS error 1003 |
| Symbol out of range | WS error 1007 |
| Tick / lot misaligned | WS error 1008 / 1009 |
| `client_order_id` > 20 chars | WS error 1010 |
| 6th conn for same user | Handshake rejected |
| WS frame > 4KB | Connection dropped |

No internal queueing -- always fail fast.

## Statelessness

Gateway holds no durable state. On crash:
- User sessions drop, in-flight orders may be lost
- Recovery: clients reconnect; Risk + WAL preserve
  authoritative state
- Fills are persisted by Risk/Recorder, never gated by
  Gateway

## Scaling

Horizontal by `user_id` hash. Load balancer routes sticky
sessions. Each gateway connects to its Risk shard via CMP/UDP.
No cross-instance coordination.

## Networking (monoio / io_uring)

Gateway uses monoio (io_uring) for all client-facing I/O:
- io_uring batches submissions in shared kernel/userspace
  rings -- fewer syscalls than epoll
- For 100K+ connections, epoll syscall overhead is too high
- Future: DPDK/AF_XDP swaps the I/O layer without touching
  the connection model
