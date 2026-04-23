# rsx-gateway Architecture

WebSocket gateway process. Accepts client connections,
validates orders, routes to Risk via CMP/UDP, pushes
fills/updates back to clients.
See `specs/2/11-gateway.md`, `specs/2/49-webproto.md`.

## Module Layout

| File | Purpose |
|------|---------|
| `main.rs` | Binary: monoio runtime, CMP pump, WS accept, event routing |
| `protocol.rs` | `WsFrame` enum, JSON serialize/deserialize |
| `handler.rs` | Per-connection: auth, parse, validate, send |
| `ws.rs` | WebSocket accept loop, frame read/write on monoio |
| `state.rs` | `GatewayState` -- connections, broadcasts, stale detection |
| `pending.rs` | `PendingTracker` -- order timeout tracking |
| `rate_limit.rs` | `RateLimiter` -- token bucket per user |
| `circuit.rs` | `CircuitBreaker` -- failure threshold + cooldown |
| `jwt.rs` | `JwtValidator` -- HS256 JWT validation |
| `order_id.rs` | UUIDv7 generation and hex encoding |
| `convert.rs` | Price/qty conversion between human and raw |
| `config.rs` | `GatewayConfig` from env vars |
| `types.rs` | Internal message types |

## Key Types

- `GatewayState` -- connection registry, pending tracker,
  circuit breaker, symbol configs
- `WsFrame` -- JSON protocol: `PlaceOrder`, `CancelOrder`,
  `Fill`, `OrderUpdate`, `Heartbeat`, `Error`
- `PendingTracker` -- order ID -> timestamp map, stale sweep
- `RateLimiter` -- per-user token bucket
- `CircuitBreaker` -- failure count with cooldown
- `JwtValidator` -- HS256 JWT token validation

## Message Flow

```
Client (WS JSON)
    |
    v
+-- handler.rs ----------------------------------+
|  1. Parse WS frame (protocol.rs)               |
|  2. Authenticate (jwt.rs: HS256 + X-User-Id)   |
|  3. Rate limit check (rate_limit.rs)            |
|  4. Circuit breaker check (circuit.rs)          |
|  5. Validate tick/lot (rsx_types::validate_order)|
|  6. Assign UUIDv7 order_id (order_id.rs)        |
|  7. Add to pending map (pending.rs)             |
|  8. Convert to CMP record (convert.rs)          |
+-------------------------------------------------+
    |
    v [CMP/UDP]
  Risk Engine
    |
    v [CMP/UDP: Fill, OrderDone, OrderFailed]
+-- handler.rs ----------------------------------+
|  1. Match fill/done to pending order (by oid)  |
|  2. Pop from pending VecDeque                   |
|  3. Convert to WS JSON (convert.rs)             |
|  4. Send to client WebSocket                    |
+-------------------------------------------------+
```

## Rate Limiting

Token bucket algorithm, three independent limiters:

| Limiter | Scope | Default |
|---------|-------|---------|
| Per-user | user_id | 100 req/s |
| Per-IP | source IP | 200 req/s |
| Per-instance | global | 10k req/s |

When any bucket is exhausted, gateway rejects with
`FailureReason::RateLimit` (code 9). No queueing.

## Circuit Breaker

Tracks Risk engine health. States: Closed (normal) ->
Open (rejecting) -> HalfOpen (probe). Transition based on
consecutive failure count and timeout.

## Backpressure

Gateway rejects new orders with OVERLOADED when:
- Pending order count exceeds cap (10k)
- Risk CMP/UDP link is down (circuit breaker open)
- Rate limit exceeded

Gateway never blocks on internal congestion. Fails fast.

## Statelessness

Gateway holds no durable state. On crash:
- User sessions drop, in-flight orders lost
- Recovery time: <1s (users reconnect)
- No data loss for trading state (fills never touch gateway)

## Scaling

Horizontal by user_id hash. Load balancer routes sticky
sessions. Each gateway instance connects to its Risk shard
via CMP/UDP. No cross-instance coordination.

## Networking (monoio / io_uring)

Gateway uses monoio (io_uring) for client-facing WebSocket I/O:
- io_uring batches submissions in shared kernel/userspace
  rings -- fewer syscalls than epoll
- For 100K+ connections, epoll syscall overhead is too high
- tokio (epoll) used only for auxiliary tasks
- Future: userspace networking (DPDK, AF_XDP) swaps I/O layer
