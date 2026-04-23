---
status: shipped
---

# TESTING-BOOK.md — Shared Orderbook Crate Tests

Source specs: [ORDERBOOK.md](ORDERBOOK.md),
[CONSISTENCY.md](CONSISTENCY.md)

Crate: `rsx-book` — shared by matching engine and market data.

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
| B1 | Price/Qty are i64 newtypes, never float | §1 |
| B2 | Tick/lot size validation at order entry | §2 |
| B3 | Compressed zone indexing (5 zones) | §2.5 |
| B4 | Bisection lookup 2-3 comparisons | §2.5 |
| B5 | Smooshed ticks store exact price per order | §2.6 |
| B6 | Matching at smooshed levels checks actual price | §2.6 |
| B7 | Incremental copy-on-write recentering | §2.7 |
| B8 | Two pre-allocated level arrays (active+staging) | §2.7 |
| B9 | Frontier-based lazy migration | §2.7 |
| B10 | Slab arena O(1) alloc/free via free list | §3 |
| B11 | OrderSlot 128B, #[repr(C, align(64))] | §3 |
| B12 | PriceLevel 24B (head, tail, total_qty, count) | §3 |
| B13 | Best bid/ask tracking (cached tick index) | §3 |
| B14 | O(1) add, cancel, match in zone 0 | §4 |
| B15 | GTC limit orders only in v1 | §5 |
| B16 | Fill price = maker's price | §5 |
| B17 | Event buffer fixed array, no heap alloc | §6 |
| B18 | Fills precede ORDER_DONE per order | §6 |
| B19 | Reduce-only enforcement via position tracking | §6.5 |
| B20 | User position tracking (net_qty per user) | §6.5 |
| B21 | Zero allocation on hot path | §7 |
| B22 | ~617K level slots, ~14.8MB per array | §2.5 |
| B23 | 78M order slots, ~10GB per book | §7 |
| B24 | Modify order (price change) = cancel + re-insert, loses time priority | §4 |
| B25 | Modify order (qty down) = in-place reduction, keeps time priority | §4 |
| B26 | SymbolConfig distribution: ME polls metadata, emits CONFIG_APPLIED | §2.9 |
| B27 | Single thread per orderbook, no locking | Design Goals |
| B28 | Hot/cold cache line split: matching touches only line 1 (48B) | §7 |
| B29 | GTC limit orders only, perpetuals only (no market orders, no spot) | Design Goals |
| B30 | WAL + online snapshot for durability/recovery | §2.8 |
| B31 | Snapshot never runs during migration | §2.8 |

---

## Unit Tests

See `rsx-book/tests/` — covers price/qty newtypes, tick/lot validation,
compression map zone lookup, modify order (price and qty), struct layout
asserts, slab allocator, PriceLevel operations, best bid/ask tracking,
matching, smooshed tick matching, event buffer, reduce-only/position
tracking, symbol config, and recentering/migration scenarios.

---

## E2E Tests

See `rsx-book/tests/` — covers full order lifecycle (insert/match/fill/done,
rest/cancel), multi-fill whale orders, book state invariants, recentering
under crash scenarios, modify lifecycles, config mid-session, snapshot +
WAL recovery, and stress/slab-leak cycles.

---

## Benchmarks

Targets from ORDERBOOK.md §4 and TESTING.md:

| Operation | Target |
|-----------|--------|
| Add order | O(1), 100-500ns |
| Cancel order | O(1), 100-300ns |
| Match per fill (zone 0) | O(1), 100-500ns |
| Modify (price change) | O(1) + O(1), <1us |
| Modify (qty down) | O(1), <100ns |
| Recentering per access | O(1) amortized, ~1-3us |
| Memory: 78M orders | ~10GB (128B slots) |
| Price level arrays | ~30MB (2 x 617K x 24B) |

---

## Integration Points

- Matching engine imports `rsx-book` for order processing
  (ORDERBOOK.md §3)
- Market data service imports `rsx-book` for shadow orderbook
  (MARKETDATA.md §2, NETWORK.md §MARKETDATA)
- BookObserver trait allows different event handling per consumer
- Event buffer drained into CMP/UDP fan-out (CONSISTENCY.md §1)
- Event routing per consumer: Fill to risk/gateway/mktdata,
  BBO to risk, OrderInserted to mktdata, OrderCancelled to
  gateway/mktdata, OrderDone to risk/gateway
  (CONSISTENCY.md §1)
- Mirrored stream to hot spare ME (CONSISTENCY.md §1)
- WAL + online snapshot for book persistence and recovery
  (ORDERBOOK.md §2.8, DXS.md §3)
- Replica takeover via DXS consumer on ME WAL stream
  (ORDERBOOK.md §2.8)
- SymbolConfig distributed from metadata store, CONFIG_APPLIED
  syncs risk and gateway caches (ORDERBOOK.md §2.9)
- System-level tests verify matching engine uses book correctly
  under load (TESTING.md §6 load tests)
