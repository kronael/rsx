# rsx-gateway

WebSocket gateway binary. Client-facing order entry point.

## What It Does

Accepts WebSocket connections with JWT auth, validates and
rate-limits orders, routes to Risk via CMP/UDP, pushes
fills and order updates back to clients.

## Running

```
RSX_GW_LISTEN_ADDR=0.0.0.0:8080 \
RSX_GW_CMP_ADDR=127.0.0.1:8000 \
RSX_RISK_CMP_ADDR=127.0.0.1:9000 \
RSX_GW_WAL_DIR=./tmp/wal \
RSX_GW_JWT_SECRET=your-secret-here \
cargo run -p rsx-gateway
```

## Environment Variables

| Env Var | Purpose |
|---------|---------|
| `RSX_GW_LISTEN_ADDR` | WebSocket listen address |
| `RSX_GW_CMP_ADDR` | CMP bind address |
| `RSX_RISK_CMP_ADDR` | Risk CMP address |
| `RSX_GW_WAL_DIR` | WAL directory for CMP sender |
| `RSX_GW_JWT_SECRET` | HS256 JWT signing secret |
| `RSX_GW_MAX_PENDING` | Max pending orders |
| `RSX_GW_ORDER_TIMEOUT_MS` | Order timeout |
| `RSX_GW_HEARTBEAT_INTERVAL_MS` | Server heartbeat interval |
| `RSX_GW_HEARTBEAT_TIMEOUT_MS` | Client heartbeat timeout |
| `RSX_GW_CIRCUIT_THRESHOLD` | Circuit breaker failure count |
| `RSX_GW_CIRCUIT_COOLDOWN_MS` | Circuit breaker cooldown |

## Deployment

- Stateless -- no durable state, crash recovery is <1s
- Horizontal scaling by user_id hash with sticky sessions
- Each instance connects to one Risk shard via CMP/UDP
- Needs `RSX_GW_JWT_SECRET` set (shared with auth service)
- Uses monoio (io_uring) -- requires Linux kernel 5.1+

## Testing

```
cargo test -p rsx-gateway
```

12 test files: circuit, config, convert, JWT, JWT+WS e2e,
order ID, pending, protocol, rate limit, rate limit e2e,
state, types. See `specs/v1/TESTING-GATEWAY.md`.

## Dependencies

- `rsx-types` -- shared types, validate_order
- `rsx-dxs` -- CMP sender/receiver

## Gotchas

- Gateway is stateless. In-flight orders are lost on crash.
  Clients must reconnect and query order status.
- Rate limiting is per-instance. With multiple gateway
  instances, effective rate is multiplied.
- Circuit breaker trips on sustained Risk failures. All
  orders are rejected while open (not queued).
- monoio requires io_uring (Linux 5.1+). Will not run on
  macOS or older kernels.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) -- message flow, rate
  limiting, circuit breaker, backpressure, scaling
- `specs/v1/GATEWAY.md`, `specs/v1/WEBPROTO.md`
