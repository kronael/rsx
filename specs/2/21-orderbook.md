---
status: shipped
---

# Orderbook Data Structures & Matching Algorithm

## Table of Contents

- [Design Goals](#design-goals-from-todomd)
- [1. Price & Quantity Representation](#1-price--quantity-representation)
- [2. Tick Size & Lot Size](#2-tick-size--lot-size)
- [2.5 How Compressed Indexing Bounds the Array](#25-how-compressed-indexing-bounds-the-array)
- [2.6 Smooshed Ticks](#26-smooshed-ticks)
- [2.7 Incremental Copy-on-Write Recentering](#27-incremental-copy-on-write-recentering)
- [2.8 Durability: WAL + Online Snapshot](#28-durability-wal--online-snapshot)
- [2.9 Symbol Config Distribution](#29-symbol-config-distribution-fees-ticks-metadata)
- [3. Orderbook Data Structure](#3-orderbook-data-structure)
- [4. Operation Complexity](#4-operation-complexity)
- [5. Matching Algorithm](#5-matching-algorithm)
- [6. Event Types](#6-event-types)
- [7. Memory Layout & Performance](#7-memory-layout--performance)
- [8. Why This Design](#8-why-this-design-alternatives-considered)

---

## Design Goals (from TODO.md)

- Single core/thread per orderbook — no locking
- No hash lookup, linear processing, good cache locality
- v1: GTC limit orders only (no market orders)
- Perpetuals trading (no spot in v1)
- All operations O(1) on the hot path
- Zero heap allocation during matching

---

## 1. Price & Quantity Representation

### Fixed-Point Integer — Never Floating Point

Floating point is non-deterministic (rounding varies across architectures) and introduces
precision errors that compound across trades. Every production exchange uses integer arithmetic.

`Price(i64)` and `Qty(i64)` are `#[repr(transparent)]` newtypes. Display conversion
happens only at the API boundary. See `rsx-types/src/lib.rs`.

---

## 2. Tick Size & Lot Size

### v1: Constant Per Symbol

Each symbol has a single, fixed tick size and lot size. All valid prices are
multiples of tick_size. All valid quantities are multiples of lot_size.

`SymbolConfig` holds `tick_size`, `lot_size`, `price_decimals`, `qty_decimals`.
Validation checks alignment to tick/lot at order entry. See `rsx-types/src/`.

Variable tick sizes (changing with price) are a v2 feature — see
[ORDERBOOK_V2.md](ORDERBOOK_V2.md).

---

## 2.9 Symbol Config Distribution (Fees, Ticks, Metadata)

All components must use the same symbol config to avoid mismatches. v1 uses
scheduled configs from the metadata store (see METADATA.md). `symbol_id` is
immutable; all other fields can change in v1.

**Source + scheduling:**
- Matching polls the metadata store every 10 minutes.
- Applies configs by `effective_at_ms` and `config_version`.
- Emits `CONFIG_APPLIED` to sync Risk and Gateway caches.

**Enforcement:**
- Matching validates tick/lot sizes and status.
- Risk applies fee schedule and margin logic.
- Gateway validates basic constraints (tick/lot alignment) to fail fast.

---

## 2.5 How Compressed Indexing Bounds the Array

### The Problem with Naive Indexing

If every market tick gets its own array slot, BTC-PERP at $0.01 ticks covering
$1K-$200K = 20M slots = 477 MB. And it can't handle prices escaping the range.

### Solution: Distance-Based Compression Zones

The array uses **constant cutoff zones** centered on mid-price. Near mid: 1:1
resolution. Farther out: coarser. Beyond 50%: catch-all smoosh to infinity.

**Market tick sizes are constant in v1.** Variable ticks (scaling with price) are a
future extension. The compression below is purely internal indexing — the market
tick size doesn't change, but the orderbook groups distant ticks into shared slots.

```
Zone     Distance from mid    Compression    Slot covers
─────────────────────────────────────────────────────────
0        0-5%                 1:1            1 market tick
1        5-15%                1:10           10 market ticks
2        15-30%               1:100          100 market ticks
3        30-50%               1:1000         1000 market ticks
4        50%+                 CATCH-ALL      ∞ (single slot per side)
```

**Example: BTC-PERP at mid=$50,000, market tick=$0.01**

```
Zone 0: $47,500-$52,500 (±5%)    → 500,000 ticks / 1   = 500,000 slots
Zone 1: $42,500-$47,500 + $52,500-$57,500 (5-15%)
                                  → 1,000,000 ticks / 10 = 100,000 slots
Zone 2: $35,000-$42,500 + $57,500-$65,000 (15-30%)
                                  → 1,500,000 ticks / 100 = 15,000 slots
Zone 3: $25,000-$35,000 + $65,000-$75,000 (30-50%)
                                  → 2,000,000 ticks / 1000 = 2,000 slots
Zone 4: <$25,000 + >$75,000 (50%+)
                                  → 1 slot per side       = 2 slots

Total: ~617,002 slots * 24B = ~14.8 MB
```

The catch-all slots (zone 4) smoosh ALL far-from-mid orders into a single slot
per side. These orders will never be matched until mid shifts dramatically — and
when it does, recentering unsmooshes them.

### Compression Band Lookup — Bisection, Not Linear

The zone lookup uses pre-computed absolute price thresholds and binary search
(not iteration, not logarithms). With 5 zones, a bisection is 2-3 comparisons.
Cost: ~2-5ns (2-3 branches + one integer division by a constant).
Division by compression factor can be replaced with multiply+shift for known
constant factors (1, 10, 100, 1000).

See `rsx-book/src/compression.rs` for `CompressionMap` and `price_to_index`.

---

## 2.6 Smooshed Ticks

### What Happens in Coarse Zones

In zones 1-3, multiple market prices share one index slot. In zone 4 (catch-all),
ALL far prices share a single slot per side.

```
Zone 2, slot 8042 (compression 1:100, covers ticks 50420-50519):
  Order A: price=$504.21, qty=10, time=T1
  Order B: price=$504.85, qty=5,  time=T2
  Order C: price=$504.21, qty=3,  time=T3
```

### Why This Is Correct

1. **Each order stores its exact price** — no information lost
2. **Matching walks the linked list** checking each order's actual price
3. **Time priority within the slot** — maintained by insertion order
4. **Interleaved prices** — orders at different prices coexist in one slot
5. **Unsmooshing**: when mid shifts toward this region, recentering spreads
   these orders across finer-grained slots in the new array

### Matching at Smooshed Levels

During extreme moves, matching may reach smooshed levels. Within a smooshed
level, scan instead of skip: check each order's actual price and skip non-matching
prices without breaking (later orders in the slot may still qualify).

See `rsx-book/src/matching.rs` for `match_at_level`.

This is O(k) per smooshed slot where k = orders in that slot. But:
- **Near mid (zone 0)**: 1:1, no smooshing, O(1) per level as before
- **Far from mid**: smooshed, but these orders are rarely matched
- **During tail events**: some extra scanning, but acceptable latency

---

## 2.7 Incremental Copy-on-Write Recentering

### Why: Keep Fine Granularity Around Active Trading

When mid-price shifts significantly (trigger: moved beyond zone 0), the
compression zones are wrong — fine granularity is no longer centered on the
action. Recenter by migrating to a new array with updated zone boundaries.

**No stop-the-world pause.** Migration is interleaved with order processing.

### Two Pre-Allocated Arrays

`Orderbook` holds `active_levels` (current write target) and `staging_levels`
(spare, used during recenter). `BookState` is either `Normal` or `Migrating`
with `bid_frontier`/`ask_frontier` (two i64s). Everything between the frontiers
is in the new array; everything outside is in old. No bitmap needed.

See `rsx-book/src/migration.rs` for `BookState`, trigger logic, `resolve_level`,
`migrate_single_level`, and `migrate_batch`.

### Trigger & Start

When mid-price drifts > 50% of zone 0 width from array center: swap staging into
active, compute new `CompressionMap` centered on current mid, set frontiers to
current mid. All new writes go to the new array from this instant.

### Main Loop: Interleaved Migration

Main loop steals idle cycles for `migrate_batch(100)` when no orders are pending;
lazy frontier advance fires on every `resolve_level` call for a price outside the
current frontier. Empty levels cost ~1ns each to skip; a jump of 10K empty levels
costs ~10us.

### Proactive Sweep

`migrate_batch` alternately expands bid frontier down and ask frontier up, calling
`migrate_single_level` per step. Orders in smooshed old slots unsmoosh naturally:
each stores its exact price, mapping to potentially different new indices.

---

## 2.8 Durability: WAL + Online Snapshot

The matching engine persists orderbook state using DXS WalWriter
([DXS.md](DXS.md)) plus online snapshots. Recovery restores the
latest snapshot and replays the WAL. The ME also embeds a DxsReplay
server so downstream consumers (risk engines, recorders) can
subscribe to its event stream.

### WAL

- ME embeds DXS WalWriter ([DXS.md](DXS.md) section 3).
- Append every order, cancel, and fill as DXS WalRecords.
- WAL is per-symbol (`stream_id` = `symbol_id`), local disk.
- Same raw bytes on disk and over the wire — no transformation.
- DxsReplay server ([DXS.md](DXS.md) section 5) serves replay and
  live tail to risk engines and other consumers.

### Online Snapshot (Shared Algorithm)

Snapshots reuse the same traversal logic as migration to keep code minimal.
The snapshot walk iterates price levels and order lists exactly as migration
does, then serializes the live orderbook state.

Key rules:
- Never migrate during a snapshot.
- If migration is active, snapshot waits.
- Mechanism: snapshot checks `book.state`. If `Migrating`,
  snapshot returns early (no-op). Next snapshot cycle retries.
  No lock needed — single-threaded main loop serializes access.
- Snapshot runs incrementally during idle cycles or on access.

### Recovery

1. Load latest snapshot (includes all state up to
   `snapshot_seq`).
2. Replay WAL from `snapshot_seq + 1` (exclusive —
   snapshot already includes `snapshot_seq`).
3. Resume matching.

### Replica Takeover (Same Mechanism as Risk)

- Replica runs a DxsConsumer on the ME WAL stream.
- Replica tracks a per-symbol tip (`seq`) and buffers ahead.
- On main failure, replica promotes and continues from last tip.
- Split-brain is avoided via Postgres advisory locks (same as risk).
- Loss window is bounded by WAL retention and tip persistence.

### Tail Event Efficiency

**50% crash:** mid drops fast, trigger fires, migration starts.
- Levels near new mid migrate first (center-out sweep + lazy access)
- Matching engine is flooded with orders → each triggers lazy migration of its level
- Far levels drain in idle gaps between orders
- No single order pays more than ~1-3us extra
- Normal latency ~100-500ns → during migration ~200-800ns

**3x rally:** same, opposite direction. Catch-all zone absorbs the extreme
until recentering catches up.

### Cancel/Modify During Migration

New orders always go to the new array. `resolve_level` ensures the target level
is migrated first. Zero special handling beyond the one `is_migrating()` branch.

---

## 3. Orderbook Data Structure

The core Book struct (PriceLevel, OrderSlot, Slab, CompressionMap)
is extracted into the shared `rsx-book` crate. Both matching engine
and MARKETDATA import it. See [MARKETDATA.md](MARKETDATA.md)
section 2 for the shared abstraction and BookObserver trait.

### Architecture: Array-Indexed Price Levels + Slab Arena

```
                    Orderbook
                   /         \
          Bid Levels          Ask Levels
     (compressed zones)    (compressed zones)
            |                    |
     [PriceLevel]          [PriceLevel]
      head → Order → Order → Order → (tail)
              ↕        ↕        ↕
          (doubly linked, stored in slab)
```

### Price Level Array

Prices map to array indices via compressed zone lookup (see section 2.5).
`PriceLevel` is 24 bytes (`head`, `tail`, `total_qty`, `order_count`).
~617K slots * 24B = ~14.8 MB per array. Two arrays = ~30 MB.
The order slab is the main memory consumer (~10 GB).

See `rsx-book/src/` for `PriceLevel`, `OrderSlot` (128B, 2 cache lines, hot/cold
split at 48B), and `Slab<T>` (Vec + free list chained through the slot's own
`next` field — O(1) alloc/free, index IS the handle).

### Best Bid/Ask Tracking

`best_bid_tick` and `best_ask_tick` are cached u32 indices. On removal of the
last order at the best level, a linear scan finds the next populated level.
In practice spread is 1-2 ticks so this scan is O(1) amortized.

During migration, all level access goes through `resolve_level(price)` which
adds one `is_migrating()` branch (predicted away in Normal state).

---

## 4. Operation Complexity

| Operation | Time | Details |
|---|---|---|
| Add order (any level) | O(1) | Bisection (2-3 cmp) + slab alloc + list append |
| Cancel order by handle | O(1) | Slab lookup + doubly-linked unlink + free list push |
| Match (per fill, zone 0) | O(1) | Walk head of best level — 1:1 resolution |
| Match (per fill, smooshed) | O(k) | Scan within smooshed tick, check exact price |
| Get best bid/ask | O(1) | Cached tick index |
| Modify order (price change) | O(1) + O(1) | Cancel + re-insert (loses time priority) |
| Modify order (qty down) | O(1) | In-place qty reduction (keeps time priority) |
| Resolve level (migrating) | O(1) amortized | One branch + occasional frontier advance |

---

## 5. Matching Algorithm

Three phases: (1) match against opposite side (walk levels from best, emit fills,
update positions inline); (1.5) TIF enforcement — FOK rolls back event_len to saved
position and emits OrderFailed if any qty remains unfilled; IOC cancels remainder
without inserting; (2) insert remainder as resting order for GTC.

Reduce-only: checked before matching, qty clamped to position size; rejects if
no open position on the incoming side.

Cancel: O(1) doubly-linked unlink + slab free; scans for new best only when the
cancelled order was at the best level.

See `rsx-book/src/matching.rs` for `process_new_order`, `match_at_level`,
`cancel_order`, and fill logic.

---

## 6. Event Types

Fixed-size array on the Orderbook struct (`[Event; 10_000]`). `event_len = 0` resets
per cycle — single store, no clear, no heap. `emit()` writes `event_buf[event_len]`
and bumps `event_len`.

Variants: `Fill`, `OrderInserted`, `OrderCancelled`, `OrderDone`, `OrderFailed`.
`OrderDone` signals the risk engine and user that an order no longer exists (fully
filled or cancelled). Emitted after the last fill or after a cancel.

See `rsx-book/src/event.rs`.

Events are drained after each order is processed. CMP/UDP fan-out to
downstream consumers:
- Risk engine (position updates from fills, OrderDone for margin release)
- Persistence layer (trade log)
- Market data dissemination (shadow orderbook, see [MARKETDATA.md](MARKETDATA.md))
- Recorder (archival via DXS consumer, see [DXS.md](DXS.md) section 8)

### 6.5 User Position Tracking

`UserState` tracks `net_qty` (signed) and `order_count` per user per symbol.
Assigned lazily on first order; reclaimed via free list when `net_qty == 0 &&
order_count == 0` (with 60s grace period). Updated on every fill for both taker
and maker.

See `rsx-book/src/user.rs` for `UserState`, `get_or_assign_user`, and
`update_positions_on_fill`.

Active user_id lifecycle:
- Assign on first order for this symbol (risk supplies mapping).
- Reclaim only when `net_qty == 0 && order_count == 0` for a grace
  period (60s) to avoid churn. Grace period is disabled
  during WAL replay (reclamation deferred until live).
- Reuse reclaimed slots via free list.

How events are consistently delivered to these systems, ordering guarantees,
and failure handling are covered in [CONSISTENCY.md](CONSISTENCY.md).

---

## 7. Memory Layout & Performance

Pre-allocate everything. Roughly 15 MB for the two level arrays; the slab dominates
(~10 GB for ~78M order slots). See bench results in `rsx-book/benches/`.

Hot path is zero-allocation: slab provides all storage, event buffer is fixed array
reset by single store, no Vec growth, no String formatting.

---

## 8. Why This Design (Alternatives Considered)

| Approach | Pros | Cons | Verdict |
|---|---|---|---|
| **Array + Slab** (chosen) | O(1) everything, cache-friendly, no pointers | Memory usage for sparse price ranges | Best for single-threaded, known tick size |
| BTreeMap<Price, Level> | Flexible price range, O(log n) | Pointer-heavy, cache-unfriendly, allocations | Too slow for HFT |
| Critbit tree (Serum) | Good for on-chain, compact | Designed for blockchain constraints, not in-memory | Wrong trade-offs |
| Skip list | O(log n), lock-free possible | Complex, poor cache locality | Overkill when single-threaded |
| Adaptive Radix Tree (exchange-core) | Good range queries, compact | Complex implementation, still tree traversal | Good alternative if price range is huge |

### References

- [exchange-core](https://github.com/exchange-core/exchange-core) — Java, Adaptive Radix Trees, LMAX Disruptor, ~0.5us matching
- [WK Selph LOB](https://github.com/Crypto-toolbox/HFT-Orderbook) — Binary tree + doubly-linked lists
- [Serum DEX](https://github.com/project-serum/serum-dex) — Critbit tree + slab arena (on-chain)
- [QuantStart LOB article](https://www.quantstart.com/articles/high-frequency-trading-ii-limit-order-book/)
- [firedancer](https://github.com/firedancer-io/firedancer) — Tile architecture, single-thread-per-core
