---
status: shipped
---

# TESTING-MATCHING.md — Matching Engine Tests

Source specs: [ORDERBOOK.md](ORDERBOOK.md),
[CONSISTENCY.md](CONSISTENCY.md), [RPC.md](RPC.md),
[MESSAGES.md](MESSAGES.md)

Binary: `rsx-matching` (one process per symbol or symbol group)

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
| M1 | Single-threaded per symbol, no locks | ORDERBOOK.md §0 |
| M2 | GTC limit orders only in v1 | ORDERBOOK.md §5 |
| M3 | Tick/lot validation before matching | ORDERBOOK.md §5 |
| M4 | Reduce-only enforcement before matching | ORDERBOOK.md §5 |
| M5 | UUIDv7 dedup via FxHashMap, 5min window | RPC.md, MESSAGES.md §7 |
| M6 | Event fan-out to risk/gateway/mktdata via CMP/UDP | CONSISTENCY.md §1 |
| M7 | CMP flow control via Status/Nak (no silent drop) | CONSISTENCY.md §3 |
| M8 | Fills precede ORDER_DONE | MESSAGES.md §fills |
| M9 | Exactly-one completion per order | MESSAGES.md §completion |
| M10 | Fill price = maker price | ORDERBOOK.md §5 |
| M11 | BBO emitted after best bid/ask change | CONSISTENCY.md §1 |
| M12 | WAL persistence via embedded WalWriter | ORDERBOOK.md §2.8 |
| M13 | Online snapshot + WAL replay recovery | ORDERBOOK.md §2.8 |
| M14 | DxsReplay server for downstream consumers | DXS.md §5 |
| M15 | Config polling every 10min, CONFIG_APPLIED | ORDERBOOK.md §2.9 |
| M16 | Position tracking per user (net_qty) | ORDERBOOK.md §6.5 |
| M17 | Deferred user reclamation (60s, net_qty==0 && order_count==0) | ORDERBOOK.md §6.5 |
| M18 | Fixed-point integer Price/Qty, never floating point | ORDERBOOK.md §1 |
| M19 | Compressed zone indexing (5 zones, bisection lookup) | ORDERBOOK.md §2.5 |
| M20 | Smooshed tick matching: scan within slot, check exact price | ORDERBOOK.md §2.6 |
| M21 | Incremental CoW recentering (no stop-the-world) | ORDERBOOK.md §2.7 |
| M22 | Slab allocator: O(1) alloc/free, free list, no shrink | ORDERBOOK.md §3 |
| M23 | Zero heap allocation on hot path | ORDERBOOK.md §7 |
| M24 | Event buffer: fixed array [Event; 10_000], no heap | ORDERBOOK.md §6 |
| M25 | Per-consumer CMP/UDP links (slow mktdata doesn't stall risk) | CONSISTENCY.md §3 |
| M26 | Total order within symbol (monotonic seq), no cross-symbol | CONSISTENCY.md §2 |
| M27 | ORDER_DONE is commit boundary for multi-fill sequences | CONSISTENCY.md §key invariants |
| M28 | Fills are final, no rollback | CONSISTENCY.md §4 |
| M29 | Snapshot + migration mutual exclusion | ORDERBOOK.md §2.8 |
| M30 | Best bid/ask tracking, scan on level exhaustion | ORDERBOOK.md §3 |
| M31 | Ingress backpressure: gateway rejects at 10k buffer cap | CONSISTENCY.md §3 |

---

## Unit Tests

See `rsx-matching/tests/` — covers order processing (tick/lot validation,
zero qty, negative price, duplicate ID, dedup window expiry), deduplication
(map lookup, cancelled-order retention, pruning after 5min, periodic
cleanup scan), and event fan-out (per-consumer routing for fill/BBO/inserted/
cancelled/done events, drain ordering).

See `rsx-matching/tests/` — covers reduce-only integration (close long/short,
clamp to position size, no-position and same-direction rejection, fill updates
position) and position tracking (taker/maker net_qty update, buy/sell direction,
user state lifecycle including 60s deferred reclaim and free-list reuse).

See `rsx-book/tests/` — covers compression map (all 5 zones, bisection 2-3
comparisons, bid/ask sides, recompute on recenter), slab allocator (sequential
alloc, free list, reuse, no-shrink, no cycles, 1M-op leak check), best bid/ask
tracking (insert/scan/exhaustion/empty), and event buffer (fixed array, no heap,
len reset per cycle, sequential slot emission, 10k max).

See `rsx-matching/tests/` — covers config application (event emission, version
monotonicity, effective_at, tick/lot update, 10min poll, metadata store source).

---

## E2E Tests

See `rsx-matching/tests/` — covers full order lifecycle (submit/fill/done,
rest/cancel/done, partial fill then fill, validation failure, 500-maker whale
order), correctness invariants (fills-before-done, exactly-one completion,
FIFO, no negative qty, coherent best bid/ask, slab no-leak, monotonic seq,
fills final, ORDER_DONE as commit boundary, zero heap during matching), WAL +
recovery (records written for all events, crash+snapshot recovery, book state
match, seq continuity, rotation under load), DxsReplay (historical records,
CaughtUp then live tail, concurrent consumers, disconnect no crash), and
fan-out under load (10k orders/s ring drain, per-ring backpressure, ring
independence, ingress rejection at 10k buffer).

See `rsx-book/tests/` — covers smooshed tick matching (exact price scan,
skip non-matching, time priority within slot, zone-4 coexistence, unsmoosh
on recenter) and recentering under load (50% price crash, orders during
migration, cancel during migration, migration completion, snapshot/migration
mutual exclusion).

---

## Benchmarks

Targets from TESTING.md §6:

| Metric | Target |
|--------|--------|
| Insert | 100-500ns (p50/p99/p99.9) |
| Match | 100-500ns |
| Cancel | 100-300ns |
| E2E latency (same machine, CMP/UDP) | <50us |
| Normal load | 10K orders/sec sustained 10min |
| Burst load | 100K orders/sec spike 10s |
| Recentering (lazy) | ~1-3us per level |
| Recentering (normal ops) | <1us overhead |
| Bisection lookup | <5ns |

See `rsx-matching/benches/` for Criterion benchmark implementations.

---

## Integration Points

- Config polling tests use Postgres via testcontainers.

- Imports `rsx-book` crate for orderbook data structures
  (ORDERBOOK.md §3)
- Embeds `rsx-dxs` WalWriter + DxsReplay server
  (ORDERBOOK.md §2.8, DXS.md §5)
- CMP/UDP fan-out to risk, gateway, mktdata
  (CONSISTENCY.md §1)
- Receives orders from risk engine via CMP/UDP
  (NETWORK.md, RISK.md §6)
- System-level: participates in full order lifecycle tests
  (TESTING.md §2 e2e, §3 integration)
- Load tests: BTC-PERP hotspot, Zipf distribution
  (TESTING.md §6 load tests)
- Hot spare ME receives mirrored event stream
  (CONSISTENCY.md §1)
- Config distribution from metadata store, polling every
  10min (ORDERBOOK.md §2.9)
- Crash recovery: snapshot + WAL replay restores book
  (ORDERBOOK.md §2.8, NETWORK.md §matching engine failure)
- Replica takeover via DxsConsumer tip + Postgres advisory
  locks (ORDERBOOK.md §2.8)
