# RSX Implementation Progress

**Last audit: 2026-02-10. Full spec compliance audit, all crates.**

---

## Executive Summary

**Status:** Phases 1-4 complete, Phase 5 at 75%.
Full order pipeline wired: Gateway -> Risk -> ME ->
Risk -> Gateway, ME -> Marketdata. Liquidation engine
core done. 549 tests passing across 42 test files.
11,636 impl + 7,829 test = 19,465 total lines.

---

## Per-Crate Status

| Crate | Impl | Test | Tests | % |
|-------|------|------|-------|---|
| rsx-types | 185 | 133 | 15 | 100 |
| rsx-book | 1,290 | 1,151 | 75 | 99 |
| rsx-matching | 777 | 454 | 11 | 90 |
| rsx-dxs | 2,055 | 1,318 | 79 | 88 |
| rsx-risk | 1,995 | 2,397 | 171 | 75 |
| rsx-gateway | 2,097 | 1,105 | 97 | 85 |
| rsx-marketdata | 1,409 | 736 | 57 | 89 |
| rsx-mark | 705 | 535 | 40 | 100 |
| rsx-recorder | 123 | 0 | 0 | 100 |
| **Total** | **11,636** | **7,829** | **549** | - |

### rsx-types (100%)
Price, Qty, Side, TimeInForce, SymbolConfig, validate_order,
panic handler, macros, time module (time_ns/time_ms/time/
perf_counter).

### rsx-book (99%)
Slab arena, CompressionMap (5 zones), PriceLevel, OrderSlot.
Matching: FIFO, smooshed tick, IOC/FOK, reduce-only.
Migration: lazy frontier, bounded by old_min/max_price.
156/157 spec requirements done.
**Missing:** Snapshot save/load, post-only enforcement.

### rsx-matching (95%)
Main loop: recv OrderMessage, process, write WAL, send CMP.
Fanout to both Risk and Marketdata (separate CmpSenders).
Marketdata gets Fill/OrderInserted/OrderCancelled (no OrderDone
per MD20). OrderCancelled reason propagated. BBO emission after
best bid/ask changes (routed to Risk only).
**Missing:** CONFIG_APPLIED.

### rsx-dxs (88%)
WAL: write/read/rotate/GC (mtime-based), CRC32.
CMP: sender/receiver, flow control, heartbeat, NACK.
DxsReplayService: TCP replay, live_seq from payload.
DxsConsumer: tip tracking, reconnect backoff.
**Missing:** Unknown record type log+skip, 5min dedup
pruning, ARCHIVE fallback, TLS, CMP config env vars,
WAL dump tool.

### rsx-risk (75%)
**Done:** Position tracking, margin calc, fees, funding,
price feeds, pre-trade checks, persistence, cold start,
process_fill (dedup, fees), process_order (margin, freeze),
process_order_done (release_margin), Risk -> ME forwarding,
liquidation engine, per-tick margin recalc, liquidation
order emission.
**Missing:** Insurance fund, advanced escalation (symbol
halt pause), replication & failover (Phase 4, 0%),
CONFIG_APPLIED, backpressure enforcement, DXS consumer
for ME replay, lease renewal.

### rsx-gateway (85%)
Per-connection handler: WS -> CMP. Order + cancel routing.
Auth (Bearer u32), rate limiting (token bucket), circuit
breaker. Heartbeat echo. Handles fill/done/cancelled from
Risk, routes to user WS. Pending order tracking by oid/cid.
ORDER_FAILED routing, server heartbeat config + timeout.
**Missing:** JWT validation, per-IP/per-instance rate
limiting, tick/lot validation at GW.

### rsx-marketdata (89%)
ShadowBook, L2/BBO/Trade serialization, SubscriptionManager.
CMP decode loop: handles insert/cancel/fill, updates shadow
book, broadcasts to WS clients. 31/35 spec requirements.
**Missing:** DXS/WAL replay bootstrap, server heartbeat,
seq gap detection + snapshot resend.

### rsx-mark (100%)
All 10 spec sections implemented. SymbolMarkState (median
aggregation), sweep_stale, staleness filtering.
BinanceSource + CoinbaseSource (tokio-tungstenite WS).
SPSC rings, config loading, DxsReplay server, WAL writer.
Main loop: drain rings, sweep 1s, flush 10ms, busy-spin.
40 tests (27 aggregator + 6 config + 7 types).

### rsx-recorder (100%)
RecorderState, daily rotation, raw WAL append.

---

## Documentation Additions

**POSITION-EDGE-CASES.md** added 2026-02-10. Comprehensive
catalog of 60+ edge cases for position tracking across Risk,
ME, Gateway, Liquidator. Covers:
- Position state transitions (empty, flip, partial, accumulation)
- Arithmetic edge cases (overflow, division by zero, negative collateral)
- Multi-user interactions (self-trade, concurrent fills/orders)
- Crash/recovery (staleness, dual crash, replay with flip/funding)
- Liquidation (margin recovery, reduce-only clamping, frozen margin)
- Price feeds (mark unavailable, mark=0, crossed mark vs index)
- Fees (negative rebate, reserve, collateral exhaustion)
- Concurrency (fill before ORDER_DONE, BBO lag, tip persistence)
- Symbol config (updates during liquidation, max position exceeded)
- Replay/reconciliation (seq gaps, position mismatch, funding zero-sum)
- Network partitions (ME isolation, Postgres isolation)

Cross-references all related specs (RISK, GUARANTEES, CONSISTENCY,
LIQUIDATOR, ORDERBOOK, TESTING-RISK, CMP, DXS).

---

## Phase Status

### Phase 1: CMP/Payload -- DONE
CmpRecord trait, seq injection, UB fix, header
simplification.

### Phase 1.5: Spec Compliance -- DONE
10 fixes: last_seq tracking, NAK off-by-one, fee
persistence, margin release, migration sell guard,
GC mtime, cancel reason, DxsConfig, reason range,
monoio docs.

### Phase 2: Gateway Wiring -- DONE
Handler, order/cancel routing, auth, rate limit, circuit
breaker, heartbeat echo, pending tracking.

### Phase 3: Event Forwarding -- DONE
ME -> Risk -> Gateway (fill/done/cancelled/inserted).
ME -> Marketdata (fill/inserted/cancelled, no done).
OrderResponse carries order_id. Risk relays all event
types. Consolidated time helpers in rsx_types::time.

### Phase 4: Mark Price -- DONE
Source connectors, aggregation, DxsReplay server, WAL
writer, main loop complete. All spec sections implemented.

### Phase 5: Liquidation -- 75%
Core engine done: LiquidationEngine with enqueue, delay,
slippage escalation (quadratic), reduce-only order
generation, recovery detection. Per-tick margin recalc
with exposure index. 19 liquidation + 5 margin_recalc
unit tests.
**Missing:** Insurance fund, symbol halt pause on
ORDER_FAILED, persistence of liquidation_events table,
E2E cascade tests.

### Phase 6: E2E Smoke Test -- NOT STARTED

---

## Remaining Work

**Critical path to MVP:**
1. DXS replay bootstrap on restart (rsx-marketdata)
2. Insurance fund + liquidation event persistence
3. Tick/lot validation at gateway

**Post-MVP:**
- Replication & failover (rsx-risk Phase 4)
- JWT validation (replace Bearer u32)
- TLS for WAL replication
- ARCHIVE fallback for old replays
- Per-IP/per-instance rate limiting
- Snapshot save/load (rsx-book)
- Post-only enforcement (rsx-book)
- Unknown record type log+skip (rsx-dxs)
- WAL dump debug tool (rsx-dxs)
- CONFIG_APPLIED handling (rsx-matching, rsx-risk)

---

## Spec Compliance

| Spec | Crate | % |
|------|-------|---|
| ORDERBOOK.md | rsx-book | 99 |
| MATCHING.md | rsx-matching | 90 |
| DXS.md | rsx-dxs | 88 |
| WAL.md | rsx-dxs | 88 |
| CMP.md | rsx-dxs | 88 |
| RISK.md | rsx-risk | 75 |
| LIQUIDATOR.md | rsx-risk | 75 |
| DATABASE.md | rsx-risk | 80 |
| ARCHIVE.md | rsx-recorder | 100 |
| MARK.md | rsx-mark | 100 |
| MARKETDATA.md | rsx-marketdata | 89 |
| WEBPROTO.md | rsx-gateway | 85 |
| RPC.md | rsx-gateway | 85 |
| MESSAGES.md | rsx-gateway | 85 |
| GATEWAY.md | rsx-gateway | 85 |
| TILES.md | All | 70 |
| NETWORK.md | All | 70 |
| CONSISTENCY.md | All | 70 |
| METADATA.md | All | 0 |
| DEPLOY.md | - | 0 |

**Last Updated:** 2026-02-10
