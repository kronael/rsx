---
status: shipped
---

# TESTING-MARKETDATA.md — Market Data Service Tests

Source specs: [MARKETDATA.md](MARKETDATA.md),
[NETWORK.md](NETWORK.md) §MARKETDATA,
[CONSISTENCY.md](CONSISTENCY.md) §1

Binary: `rsx-marketdata`

## Table of Contents

- [Requirements Checklist](#requirements-checklist)
- [Unit Tests](#unit-tests)
- [Benchmarks](#benchmarks)
- [Integration Points](#integration-points)

---

## Requirements Checklist

| # | Requirement | Source |
|---|-------------|--------|
| MD1 | Shadow orderbook per symbol (shared rsx-book) | NETWORK.md §MARKETDATA |
| MD2 | Derive BBO from shadow book | MARKETDATA.md |
| MD3 | Derive L2 depth (snapshot + delta) from events | MARKETDATA.md |
| MD4 | WS public feed (BBO/L2/trades) | WEBPROTO.md |
| MD5 | Subscribe by symbol_id list + depth | WEBPROTO.md §market data |
| MD6 | L2Snapshot sent on initial subscribe | WEBPROTO.md §market data |
| MD7 | Deltas after snapshot, seq monotonic per symbol | WEBPROTO.md §market data |
| MD8 | Backpressure: drop deltas, resend snapshot | MARKETDATA.md §notes |
| MD9 | Seq gap -> client re-subscribes for snapshot | WEBPROTO.md §market data |
| MD10 | Public endpoint, no auth | MARKETDATA.md |
| MD11 | Single-threaded, dedicated core, busy-spin | NETWORK.md §MARKETDATA |
| MD12 | monoio (io_uring) for WS I/O (no Tokio) | NETWORK.md §MARKETDATA |
| MD13 | CMP/UDP input from matching engine | NETWORK.md §MARKETDATA |
| MD14 | Recovery via DXS replay from ME WAL | DXS.md §8 |
| MD15 | WS JSON: BBO, B (snapshot), D (delta), S, X | WEBPROTO.md |
| MD16 | Event routing: Fill + OrderInserted + Cancelled | CONSISTENCY.md §1 |
| MD17 | WS schema mirrors JSON (B/D/BBO) | WEBPROTO.md |
| MD18 | BBO includes order count per side (bid_count, ask_count) | MARKETDATA.md §messages |
| MD19 | Snapshot consistency: point-in-time best effort | MARKETDATA.md §transport |
| MD20 | OrderDone NOT routed to market data | CONSISTENCY.md §1 table |
| MD21 | MktData derives own BBO from shadow book (not ME BBO) | CONSISTENCY.md §1 |
| MD22 | WS seq gap: u jumps >1 triggers re-subscribe | WEBPROTO.md §market data |
| MD23 | `u` field is WS alias for `seq` | WEBPROTO.md §market data |
| MD24 | Server sends B snapshot on subscribe before D deltas | WEBPROTO.md §market data |
| MD25 | Trades derived from fill events | NETWORK.md §MARKETDATA |
| MD26 | Subscribe depth parameter: 10, 25, 50 | MARKETDATA.md §subscribe |

---

## Unit Tests

See `rsx-marketdata/tests/` — shadow_book_test.rs, bbo_test.rs,
snapshot_test.rs, delta_test.rs, subscription_test.rs,
backpressure_test.rs, trade_test.rs, event_routing_test.rs,
ws_frame_test.rs.

---

## Benchmarks

See `rsx-marketdata/benches/` for Criterion benchmarks.

Derived from system-level E2E latency target (<50us same machine,
TESTING.md §6) and throughput requirements (10K orders/sec normal,
100K burst):

| Operation | Target |
|-----------|--------|
| Shadow book insert/fill | <500ns |
| BBO derivation | <100ns |
| L2 snapshot (10 levels) | <1us |
| L2 delta generation | <200ns |
| Event processing throughput | >100K events/sec |
| 100-client broadcast per event | <100us |

---

## Integration Points

- Imports `rsx-book` crate for shadow orderbook (NETWORK.md §MARKETDATA)
- Receives Fill, OrderInserted, OrderCancelled via CMP/UDP
  from matching engine (CONSISTENCY.md §1)
- Connects as DXS consumer for ME WAL replay on startup (DXS.md §8)
- Serves WS marketdata feed to external clients (MARKETDATA.md §service)
- Serves public WS endpoint with BBO/B/D frames (WEBPROTO.md §market data)
- System-level: market data streaming in smoke tests (TESTING.md §5 smoke)
- Load tests: 100 clients, Zipf symbol distribution (TESTING.md §6)
- Does NOT receive OrderDone or BBO events from ME (CONSISTENCY.md §1 event routing table)
- Derives own BBO from shadow book, not from ME BBO event (CONSISTENCY.md §1)
- WS `u` field maps to `seq` for gap detection (WEBPROTO.md §market data)
