---
status: shipped
---

# TESTING-GATEWAY.md — Gateway Tests

Source specs: [20-network.md](20-network.md), [49-webproto.md](49-webproto.md),
[29-rpc.md](29-rpc.md), [18-messages.md](18-messages.md)

Binary: `rsx-gateway`

## Table of Contents

- [Requirements Checklist](#requirements-checklist)
- [Unit Tests](#unit-tests)
- [E2E Tests](#e2e-tests)
- [Benchmarks](#benchmarks)
- [Integration Points](#integration-points)

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| G1 | WS overlay: compact JSON, single-letter types | WEBPROTO.md |
| G2 | casting/WAL wire format for internal links | NETWORK.md |
| G3 | JWT auth via the WS upgrade header; a token is reusable until `exp`; no in-band auth frame | WEBPROTO.md |
| G4 | UUIDv7 order ID generated at gateway | RPC.md §order-id |
| G5 | Per-user VecDeque pending order tracking, newest-first search | RPC.md §pending |
| G6 | Rate limiting: 10/s per user, 100/s per IP | RPC.md §rate-limit |
| G7 | Ingress backpressure: in-flight cap 10k (resting orders excluded), `E[5,"overloaded"]` | RPC.md §backpressure |
| G8 | Heartbeat: 5s interval, 10s timeout | WEBPROTO.md §H |
| G9 | Order timeout: 10s without a first response, `U` status 3 reason TIMEOUT | RPC.md §timeout |
| G10 | No ACK on order -- first response is update/fill | WEBPROTO.md |
| G11 | Fill streaming: 0+ fills then ORDER_DONE/FAILED | MESSAGES.md §fills |
| G12 | Circuit breaker: 10 failures -> open -> half-open | RPC.md §circuit |
| G13 | Market data WS: S subscribe, X unsubscribe | WEBPROTO.md §S |
| G14 | Liquidation event Q frame to user WS | WEBPROTO.md §Q |
| G15 | Single casting/UDP link to risk engine | NETWORK.md |
| G16 | Config cache synced via CONFIG_APPLIED | MESSAGES.md |
| G17 | Tick/lot pre-validation (fail fast) | ORDERBOOK.md §2.9 |
| G18 | Out-of-order response handling via order_id | RPC.md §pending |
| G19 | A resting order is never swept; only a `Submitted` entry times out | RPC.md §timeout |
| G20 | Per-instance throughput cap: 1000 orders/s, `E[5,"overloaded"]` | RPC.md §rate-limit |
| G21 | Enum validation: Side (0-1), TIF (0-2), OrderStatus (0-3), FailureReason (0-14, `ORDER_TOO_SMALL` = 14) | WEBPROTO.md §enums |
| G22 | Reduce-only (ro) field in N frame (optional, default 0) | WEBPROTO.md §N |
| G23 | Fill fee field: signed int64, negative=rebate | WEBPROTO.md §F |
| G24 | Error frame E: code + msg | WEBPROTO.md §E |
| G25 | No permessage-deflate compression | WEBPROTO.md §frame-shape |
| G26 | Horizontal scaling: user_id hash sharding | NETWORK.md §scaling |
| G27 | Dedup: 1-hour window in the ME and in the gateway cid map; a repeated cid is answered `E[1011]` naming the first order and not forwarded; ORDER_FAILED releases the cid; a timed-out order stays cancellable (`rsx-gateway/tests/handler_cid_test.rs`, `handler_timeout_test.rs`) | RPC.md §dedup |
| G28 | OrderDone/OrderFailed exactly one per order | MESSAGES.md §completion |
| G29 | Fills precede ORDER_DONE in stream | MESSAGES.md §fill-streaming |
| G30 | Fixed-point price/qty: int64, no float | MESSAGES.md §field-encodings |
| G31 | Exactly one key per WS frame | WEBPROTO.md §frame-shape |

---

## Unit Tests

See `rsx-gateway/tests/` — records_test.rs, convert_test.rs,
order_id_test.rs, pending_test.rs, rate_limit_test.rs,
circuit_test.rs, state_test.rs, prevalidation_test.rs, jwt_test.rs,
stream_ordering_test.rs, and `rsx-gateway/src/` — client_ids_test.rs,
metrics_test.rs, egress_test.rs, cast_recv_test.rs.

---

## E2E Tests

See `rsx-gateway/tests/` — the `handler_*_test.rs` files (cancel, cid, metrics,
order, protocol, replay, route, session, timeout) drive the connection handler
over real sockets; `jwt_ws_e2e_test.rs`, `order_lifecycle_test.rs`,
`rate_limit_e2e_test.rs`, `rest_contract_test.rs` and `gateway_process_test.rs`
(the real binary).

---

## Benchmarks

Targets from NETWORK.md:

| Path | Target |
|------|--------|
| External -> Gateway | ~1-10ms (after TLS) |
| Gateway -> Risk (casting/UDP) | ~4 µs one-way, loopback (measured) |
| WS frame parse (N/C/A) | <500ns |
| UUIDv7 generation | <50ns |
| Rate limit check | <50ns |

---

## Integration Points

- Single casting/UDP link to risk engine (NETWORK.md)
- Receives fills/done/failed from risk via casting/UDP
- Receives liquidation events from risk via casting/UDP (WEBPROTO.md §Q)
- Forwards CONFIG_APPLIED to local config cache (MESSAGES.md §ConfigApplied)
- Public market data WS endpoint separate from trading WS (WEBPROTO.md §market data)
- System-level: full order lifecycle gateway -> risk -> ME (TESTING.md §2 e2e)
- Load tests: 10K concurrent users, 100K orders/sec burst (TESTING.md §6 load tests)
- Fixed-point price/qty conversion at gateway ingress (MESSAGES.md §field-encodings)
- Horizontal scaling via user_id hash sharding (NETWORK.md §gateway-scaling)
