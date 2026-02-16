# RSX Implementation Progress

**Last audit: 2026-02-16. Numeric safety refinement:
overflow guards on hot paths, saturating arithmetic,
bounds checks.**

---

## Executive Summary

**Status:** All v1 spec requirements met. 813 Rust tests
passing (zero failures). ~17k LOC source + ~18k LOC tests.
All hot-path unwraps eliminated. Wire protocol spec-compliant
(WEBPROTO.md, MESSAGES.md, RPC.md). CLI tools complete
(JSON + Parquet output). Gateway hardened (heartbeat 5s/10s,
frame size 4KB, binary frame rejection, per-user 5-conn
limit, liquidation routing). Playground: 223 Playwright
tests passing, real data endpoints (/x/book-stats, /x/live-fills,
/x/trade-agg, /x/book), WAL binary parser, fixed process env
inheritance.

**Test Quality:** All Rust tests non-flaky (unique temp dirs,
ephemeral ports, migration completeness checks, dedup
boundary fixes). Playground tests hardened (process cleanup,
polling replaces sleeps).

**Overall completion: 100%** (all v1 spec requirements met)

---

## Per-Crate Status

| Crate | Tests | % |
|-------|-------|---|
| rsx-types | 15 | 100 |
| rsx-book | 135 | 100 |
| rsx-matching | 56 | 100 |
| rsx-dxs | 116 | 100 |
| rsx-risk | 224 | 100 |
| rsx-gateway | 124 | 100 |
| rsx-marketdata | 90 | 100 |
| rsx-mark | 43 | 100 |
| rsx-recorder | 5 | 100 |
| rsx-cli | 5 | 100 |

### rsx-types (100%)
Price, Qty, Side, TimeInForce, SymbolConfig, validate_order,
panic handler, macros, time module (time_ns/time_ms/time/
perf_counter).

### rsx-book (100%)
Slab arena, CompressionMap (5 zones), PriceLevel, OrderSlot.
Matching: FIFO, smooshed tick, IOC/FOK, reduce-only, post-only.
Migration: lazy frontier, bounded by old_min/max_price.
Snapshot save/load with binary serialization (blocks during
migration). 9 snapshot tests. All spec requirements done.
Tests: migration completion assertions, zone boundary edge
cases, compression map coverage.

### rsx-matching (100%)
Main loop: recv OrderMessage, process, write WAL, send CMP.
Fanout to both Risk and Marketdata (separate CmpSenders).
BBO emission after best bid/ask changes (routed to Risk
only). Config polling every 10min with CONFIG_APPLIED
emission to WAL, Risk, and Marketdata. Order dedup with
5min pruning (DedupTracker, OrderAcceptedRecord in WAL,
OrderFailed on duplicate). Testcontainers Postgres tests
for config polling (9 tests). Tests: dedup boundary logic
corrected (submission after expiry window).

### rsx-dxs (100%)
WAL: write/read/rotate/GC (mtime-based), CRC32.
CMP: sender/receiver, flow control, heartbeat, NACK,
configurable via CmpConfig (env vars).
DxsReplayService: TCP replay, live_seq from payload, TLS.
DxsConsumer: tip tracking, reconnect backoff, TLS, unknown
record skip. WAL dump via rsxcli (rsx-cli crate).
Tests: unique temp dirs (no races), proper port allocation
(cmp/tls tests), extract_seq helper for archive tests.

### rsx-risk (100%)
Position tracking, margin calc, fees, funding, price feeds,
pre-trade checks, persistence, cold start, process_fill
(dedup, fees), process_order (margin, freeze),
process_order_done (release_margin), Risk -> ME forwarding,
liquidation engine, per-tick margin recalc, liquidation
order emission, insurance fund (accounting + persistence +
socialized loss), CONFIG_APPLIED handling, DXS consumer for
ME replay, lease renewal, backpressure enforcement, symbol
halt/resume on ORDER_FAILED in liquidation engine,
replication & failover (advisory lease, replica state,
promotion, tip sync).

### rsx-gateway (100%)
Per-connection handler: WS -> CMP. Order + cancel routing.
JWT auth (HS256) with X-User-Id fallback, rate limiting
(token bucket per-user/per-IP/per-instance), circuit breaker.
Heartbeat 5s interval / 10s timeout. Binary frame rejection.
Frame size limit 4KB. Per-user 5-conn limit. Liquidation
routing. Handles fill/done/cancelled from Risk, routes to
user WS. Pending order tracking by oid/cid. ORDER_FAILED
routing. Tick/lot validation at order entry. Status codes
aligned with WEBPROTO spec.

### rsx-marketdata (100%)
ShadowBook, L2/BBO/Trade serialization, SubscriptionManager.
CMP decode loop: handles insert/cancel/fill, updates shadow
book, broadcasts to WS clients. DXS replay bootstrap on
startup. Server heartbeat. Seq gap detection with automatic
L2 snapshot resend to depth subscribers.

### rsx-mark (100%)
All 10 spec sections implemented. SymbolMarkState (median
aggregation), sweep_stale, staleness filtering.
BinanceSource + CoinbaseSource (tokio-tungstenite WS).
SPSC rings, config loading, DxsReplay server, WAL writer.
Main loop: drain rings, sweep 1s, flush 10ms, busy-spin.
CMP sender to risk (RECORD_MARK_PRICE).

### rsx-recorder (100%)
WAL archival consumer. Daily file rotation. Buffered
writes (flush every 1000 records). DxsConsumer integration.
5 tests covering serialization, rotation, roundtrip.

### rsx-cli (100%)
WAL dump tool with JSON and Parquet output. Two commands:
`dump <file>` (single file) and `wal-dump <stream>` (WAL
directory). Parquet via Arrow (optional feature). 5 tests
covering format parsing and output generation.

---

## Phase Status

### Phase 1: CMP/Payload -- DONE
### Phase 1.5: Spec Compliance -- DONE
### Phase 2: Gateway Wiring -- DONE
### Phase 3: Event Forwarding -- DONE
### Phase 4: Mark Price -- DONE
### Phase 5: Liquidation -- DONE
### Phase 6: Gateway Hardening -- DONE (2026-02-12)
Heartbeat intervals (5s/10s), frame size limit (4KB),
binary frame rejection, per-user connection limit (5),
liquidation routing, spec post-MVP annotations.

---

## Spec Compliance

| Spec | Crate | % |
|------|-------|---|
| ORDERBOOK.md | rsx-book | 100 |
| MATCHING.md | rsx-matching | 100 |
| DXS.md | rsx-dxs | 100 |
| WAL.md | rsx-dxs | 100 |
| CMP.md | rsx-dxs | 100 |
| RISK.md | rsx-risk | 100 |
| LIQUIDATOR.md | rsx-risk | 100 |
| DATABASE.md | rsx-risk | 100 |
| ARCHIVE.md | rsx-recorder | 100 |
| MARK.md | rsx-mark | 100 |
| MARKETDATA.md | rsx-marketdata | 100 |
| WEBPROTO.md | rsx-gateway | 100 |
| GATEWAY.md | rsx-gateway | 100 |
| TILES.md | All | 95 |
| NETWORK.md | All | 95 |
| CONSISTENCY.md | All | 100 |

Post-MVP specs (not in v1 scope):
- REST.md, METADATA.md, DEPLOY.md, TELEMETRY.md
- WEBPROTO.md query frames (O/P/A/FL/FN/M/T)

---

## Summary

**Overall System: 100%** (all v1 spec requirements met)
**Rust Tests: 813** (zero failures, all non-flaky)
**Playground Tests: 223** Playwright (all passing)

**Last Updated:** 2026-02-16
