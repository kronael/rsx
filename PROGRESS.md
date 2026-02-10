# RSX Implementation Progress

**Last audit: 2026-02-10. Deep codebase scan, all crates compile.**

---

## Executive Summary

**Status:** Phases 1-3 complete. Full order pipeline wired:
Gateway → Risk → ME → Risk → Gateway, ME → Marketdata.
494 tests passing across 40 test files. 11,310 impl +
8,272 test = 19,582 total lines.

---

## Per-Crate Status

| Crate | Impl | Test | Tests | % |
|-------|------|------|-------|---|
| rsx-types | 209 | 148 | 15 | 100 |
| rsx-book | 1,401 | 1,274 | 75 | 90 |
| rsx-matching | 809 | 497 | 11 | 90 |
| rsx-dxs | 2,229 | 1,476 | 79 | 90 |
| rsx-risk | 1,963 | 2,267 | 123 | 90 |
| rsx-gateway | 2,242 | 1,211 | 94 | 85 |
| rsx-marketdata | 1,524 | 802 | 57 | 85 |
| rsx-mark | 792 | 597 | 40 | 75 |
| rsx-recorder | 141 | 0 | 0 | 100 |
| **Total** | **11,310** | **8,272** | **494** | - |

### rsx-types (100%)
Price, Qty, Side, TimeInForce, SymbolConfig, validate_order,
panic handler, macros, time module (time_ns/time_ms/time/
perf_counter).

### rsx-book (90%)
Slab arena, CompressionMap (5 zones), PriceLevel, OrderSlot.
Matching: FIFO, smooshed tick, IOC/FOK, reduce-only.
Migration: lazy frontier, bounded by old_min/max_price.
**Missing:** Snapshot save/load, post-only enforcement.

### rsx-matching (90%)
Main loop: recv OrderMessage, process, write WAL, send CMP.
Fanout to both Risk and Marketdata (separate CmpSenders).
Marketdata gets Fill/OrderInserted/OrderCancelled (no OrderDone
per MD20). OrderCancelled reason propagated.
**Missing:** BBO emission, CONFIG_APPLIED.

### rsx-dxs (90%)
WAL: write/read/rotate/GC (mtime-based), CRC32.
CMP: sender/receiver, flow control, heartbeat, NACK.
DxsReplayService: TCP replay, live_seq from payload.
DxsConsumer: tip tracking, reconnect backoff.
**Missing:** CMP stress tests (packet loss, reorder).

### rsx-risk (90%)
process_fill (dedup, fees), process_order (margin, freeze),
process_order_done (release_margin). Risk → ME forwarding via
accepted_producer + CMP send_raw. Receives all ME event types
(fill, done, cancelled, inserted) and forwards to Gateway.
OrderResponse has order_id_hi/lo.
**Missing:** Liquidation processing, mark price DXS consumer.

### rsx-gateway (85%)
Per-connection handler: WS → CMP. Order + cancel routing.
Auth (Bearer u32), rate limiting (token bucket), circuit
breaker. Heartbeat echo. Handles fill/done/cancelled from
Risk, routes to user WS. Pending order tracking by oid/cid.
**Missing:** Server-initiated heartbeats + timeout, JWT.

### rsx-marketdata (85%)
ShadowBook, L2/BBO/Trade serialization, SubscriptionManager.
CMP decode loop: handles insert/cancel/fill, updates shadow
book, broadcasts to WS clients.
**Missing:** SubscriptionManager wiring in main loop.

### rsx-mark (75%)
SymbolMarkState (median aggregation), sweep_stale.
BinanceSource + CoinbaseSource (tokio-tungstenite WS).
SPSC rings, config loading, main loop skeleton.
**Missing:** Source connector startup, DXS replay server.

### rsx-recorder (100%)
RecorderState, daily rotation, raw WAL append.

---

## Phase Status

### Phase 1: CMP/Payload — DONE
CmpRecord trait, seq injection, UB fix, header simplification.

### Phase 1.5: Spec Compliance — DONE
10 fixes: last_seq tracking, NAK off-by-one, fee persistence,
margin release, migration sell guard, GC mtime, cancel reason,
DxsConfig, reason range, monoio docs.

### Phase 2: Gateway Wiring — DONE
Handler, order/cancel routing, auth, rate limit, circuit
breaker, heartbeat echo, pending tracking.

### Phase 3: Event Forwarding — DONE
ME → Risk → Gateway (fill/done/cancelled/inserted).
ME → Marketdata (fill/inserted/cancelled, no done).
OrderResponse carries order_id. Risk relays all event types.
Consolidated time helpers in rsx_types::time.

### Phase 4: Mark Price — 75%
Source connectors written. Main loop skeleton.
Not yet: source startup, DXS consumer in risk.

### Phase 5: E2E Smoke Test — NOT STARTED

---

## Remaining Work

**Critical path to MVP:**
1. Marketdata WS broadcast loop (wire SubscriptionManager)
2. Risk mark price DXS consumer
3. Liquidation order generation

**Post-MVP:**
- Server-initiated heartbeats + client timeout
- JWT validation (replace Bearer u32)
- Post-only enforcement
- Snapshot save/load
- CMP stress tests

---

## Spec Compliance

| Spec | Crate | % |
|------|-------|---|
| ORDERBOOK.md | rsx-book | 90 |
| DXS.md | rsx-dxs | 90 |
| WAL.md | rsx-dxs | 90 |
| CMP.md | rsx-dxs | 85 |
| RISK.md | rsx-risk | 90 |
| DATABASE.md | rsx-risk | 90 |
| ARCHIVE.md | rsx-recorder | 100 |
| TILES.md | All | 70 |
| NETWORK.md | All | 70 |
| CONSISTENCY.md | All | 70 |
| MARK.md | rsx-mark | 40 |
| MARKETDATA.md | rsx-marketdata | 85 |
| WEBPROTO.md | rsx-gateway | 75 |
| RPC.md | rsx-gateway | 80 |
| MESSAGES.md | rsx-gateway | 75 |
| LIQUIDATOR.md | rsx-risk | 0 |
| METADATA.md | All | 0 |
| DEPLOY.md | - | 0 |

**Last Updated:** 2026-02-10
