---
status: shipped
---

# TESTING-GATEWAY.md — Gateway Tests

Source specs: [NETWORK.md](NETWORK.md), [WEBPROTO.md](WEBPROTO.md),
[RPC.md](RPC.md), [MESSAGES.md](MESSAGES.md)

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
| G2 | CMP/WAL wire format for internal links | NETWORK.md |
| G3 | JWT auth via WS upgrade headers (A fallback) | WEBPROTO.md |
| G4 | UUIDv7 order ID generated at gateway | RPC.md §order-id |
| G5 | LIFO VecDeque pending order tracking | RPC.md §pending |
| G6 | Rate limiting: 10/s per user, 100/s per IP | RPC.md §rate-limit |
| G7 | Ingress backpressure: cap 10k, OVERLOADED | RPC.md §backpressure |
| G8 | Heartbeat: 5s interval, 10s timeout | WEBPROTO.md §H |
| G9 | Order timeout: 10s | RPC.md §timeout |
| G10 | No ACK on order -- first response is update/fill | WEBPROTO.md |
| G11 | Fill streaming: 0+ fills then ORDER_DONE/FAILED | MESSAGES.md §fills |
| G12 | Circuit breaker: 10 failures -> open -> half-open | RPC.md §circuit |
| G13 | Market data WS: S subscribe, X unsubscribe | WEBPROTO.md §S |
| G14 | Liquidation event Q frame to user WS | WEBPROTO.md §Q |
| G15 | Single CMP/UDP link to risk engine | NETWORK.md |
| G16 | Config cache synced via CONFIG_APPLIED | MESSAGES.md |
| G17 | Tick/lot pre-validation (fail fast) | ORDERBOOK.md §2.9 |
| G18 | Out-of-order response handling via order_id | RPC.md §pending |
| G19 | Stale order policy: 5 min, client cancels/forgets | RPC.md §timeout |
| G20 | Per-instance throughput cap: 1000 orders/s | RPC.md §rate-limit |
| G21 | Enum validation: Side (0-1), TIF (0-2), OrderStatus (0-3), FailureReason (0-12) | WEBPROTO.md §enums |
| G22 | Reduce-only (ro) field in N frame (optional, default 0) | WEBPROTO.md §N |
| G23 | Fill fee field: signed int64, negative=rebate | WEBPROTO.md §F |
| G24 | Error frame E: code + msg | WEBPROTO.md §E |
| G25 | No permessage-deflate compression | WEBPROTO.md §frame-shape |
| G26 | Horizontal scaling: user_id hash sharding | NETWORK.md §scaling |
| G27 | Dedup: 5-min window in ME, fresh UUIDv7 on retry | RPC.md §dedup |
| G28 | OrderDone/OrderFailed exactly one per order | MESSAGES.md §completion |
| G29 | Fills precede ORDER_DONE in stream | MESSAGES.md §fill-streaming |
| G30 | Fixed-point price/qty: int64, no float | MESSAGES.md §field-encodings |
| G31 | Exactly one key per WS frame | WEBPROTO.md §frame-shape |

---

## Unit Tests

See `rsx-gateway/tests/` — protocol_test.rs, convert_test.rs,
order_id_test.rs, pending_test.rs, rate_limit_test.rs,
circuit_test.rs, heartbeat_test.rs, prevalidation_test.rs.

---

## E2E Tests

See `rsx-gateway/tests/` — gateway_e2e_test.rs.

---

## Benchmarks

Targets from NETWORK.md:

| Path | Target |
|------|--------|
| External -> Gateway | ~1-10ms (after TLS) |
| Gateway -> Risk (UDS) | ~50-100us per message |
| Gateway -> Risk (TCP) | ~100-300us per message |
| WS frame parse (N/C/A) | <500ns |
| UUIDv7 generation | <50ns |
| Rate limit check | <50ns |

---

## Integration Points

- Single CMP/UDP link to risk engine (NETWORK.md)
- Receives fills/done/failed from risk via CMP/UDP
- Receives liquidation events from risk via CMP/UDP (WEBPROTO.md §Q)
- Forwards CONFIG_APPLIED to local config cache (MESSAGES.md §ConfigApplied)
- Public market data WS endpoint separate from trading WS (WEBPROTO.md §market data)
- System-level: full order lifecycle gateway -> risk -> ME (TESTING.md §2 e2e)
- Load tests: 10K concurrent users, 100K orders/sec burst (TESTING.md §6 load tests)
- Fixed-point price/qty conversion at gateway ingress (MESSAGES.md §field-encodings)
- Horizontal scaling via user_id hash sharding (NETWORK.md §gateway-scaling)
