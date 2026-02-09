# Orderbook Data Structures & Matching Algorithm

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

```rust
/// Price in smallest tick units. 1 = one tick.
/// For BTC-USD with tick_size = 0.01 USD: Price(5000000) = $50,000.00
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Price(pub i64);

/// Quantity in smallest lot units. 1 = one lot.
/// For BTC with lot_size = 0.001 BTC: Qty(1000) = 1.0 BTC
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Qty(pub i64);
```

Conversion from human-readable:
```
price_raw = (human_price / tick_size) as i64     // e.g. 50000.00 / 0.01 = 5000000
qty_raw   = (human_qty / lot_size) as i64        // e.g. 1.5 / 0.001 = 1500
```

All arithmetic in the matching engine uses these integer types. Display conversion
happens only at the API boundary.

---

## 2. Tick Size & Lot Size

### v1: Constant Per Symbol

Each symbol has a single, fixed tick size and lot size. All valid prices are
multiples of tick_size. All valid quantities are multiples of lot_size.

```rust
struct SymbolConfig {
    symbol_id: u32,
    /// Price decimals (e.g. 8 = prices stored as price * 10^8)
    price_decimals: u8,
    /// Qty decimals (e.g. 8 = qtys stored as qty * 10^8)
    qty_decimals: u8,
    /// Minimum price increment (in Price units)
    tick_size: i64,
    /// Minimum quantity increment (in Qty units)
    lot_size: i64,
}
```

**Examples:**

| Symbol | Tick Size | Lot Size | Price(50000.01) | Qty(1.5) |
|---|---|---|---|---|
| BTC-PERP | $0.01 (tick_size=1) | 0.001 BTC (lot_size=1000) | Price(5000001) | Qty(1500000) |
| ETH-PERP | $0.01 (tick_size=1) | 0.01 ETH (lot_size=10000) | Price(300001) | Qty(150000) |

**Validation at order entry:**
```rust
fn validate_order(config: &SymbolConfig, price: Price, qty: Qty) -> bool {
    price.0 % config.tick_size == 0
    && qty.0 % config.lot_size == 0
    && qty.0 > 0
    && price.0 > 0
}
```

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
(not iteration, not logarithms). With 5 zones, a bisection is 2-3 comparisons:

```rust
/// Pre-computed zone boundaries (absolute prices, not distances)
/// Recomputed once per recenter — NOT per order
struct CompressionMap {
    mid_price: i64,
    /// Sorted thresholds: [zone0_end, zone1_end, zone2_end, zone3_end]
    /// Zone 4 is everything beyond zone3_end (catch-all)
    thresholds: [i64; 4],      // absolute price boundaries
    compressions: [u32; 5],    // ticks-per-slot for each zone [1, 10, 100, 1000, INF]
    base_indices: [u32; 5],    // first array index for each zone
    zone_slots: [u32; 5],      // number of slots per zone
}

impl CompressionMap {
    /// Bisection: 2-3 comparisons, no loops, no log/div on hot path
    #[inline(always)]
    fn price_to_index(&self, price: i64) -> u32 {
        let distance = (price - self.mid_price).unsigned_abs() as i64;
        let side = if price >= self.mid_price { 0 } else { 1 }; // ask=0, bid=1

        // Binary search on 4 thresholds → 2-3 branches
        let zone = if distance < self.thresholds[1] {
            if distance < self.thresholds[0] { 0 } else { 1 }
        } else {
            if distance < self.thresholds[2] { 2 }
            else if distance < self.thresholds[3] { 3 }
            else { 4 } // catch-all
        };

        if zone == 4 {
            // Catch-all: one slot per side
            return self.base_indices[4] + side as u32;
        }

        let zone_start_distance = if zone == 0 { 0 } else {
            self.thresholds[zone - 1]
        };
        let local_offset = ((distance - zone_start_distance) / self.compressions[zone] as i64) as u32;
        let zone_base = self.base_indices[zone];
        let half_slots = self.zone_slots[zone] / 2;

        if side == 0 { // ask side
            zone_base + half_slots + local_offset
        } else { // bid side
            zone_base + half_slots - 1 - local_offset
        }
    }
}
```

**Cost: ~2-5ns** (2-3 branches + one integer division by a constant).
The division by compression factor can be replaced with multiply+shift for
known constant factors (1, 10, 100, 1000).

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
level, scan instead of skip:

```
fn match_at_level(book, tick, aggressor, events):
    level = &book.levels[tick]
    cursor = level.head

    while cursor != NONE AND aggressor.remaining_qty > 0:
        maker = &book.orders[cursor]

        // In smooshed ticks, check ACTUAL price
        if aggressor.side == Buy AND maker.price.0 > aggressor.price.0:
            cursor = maker.next   // skip — but don't break (later orders may qualify)
            continue
        if aggressor.side == Sell AND maker.price.0 < aggressor.price.0:
            cursor = maker.next
            continue

        fill_qty = min(aggressor.remaining_qty, maker.remaining_qty)
        // ... normal fill logic, fill price = maker.price (exact)
```

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

```rust
struct Orderbook {
    active_levels: Vec<PriceLevel>,    // current, all writes go here
    staging_levels: Vec<PriceLevel>,   // spare, used during recenter
    compression: CompressionMap,        // current zone boundaries
    state: BookState,
    // ... rest of book
}

enum BookState {
    Normal,
    Migrating {
        old_levels: Vec<PriceLevel>,   // being drained
        old_compression: CompressionMap,
        bid_frontier: i64,             // lowest price migrated (expands down)
        ask_frontier: i64,             // highest price migrated (expands up)
    },
}
```

**Migration tracking = two i64 prices.** Everything between bid_frontier and
ask_frontier is in the new array. Everything outside is in old. No bitmap.

### Trigger & Start

```
When mid-price drifts > 50% of zone 0 width from array center:

1. new_levels = staging_levels (pre-allocated, zeroed)
2. Compute new CompressionMap centered on current mid
3. old_levels = swap out active_levels
4. active_levels = new_levels
5. bid_frontier = ask_frontier = new mid-price
6. state = Migrating { old_levels, old_compression, bid_frontier, ask_frontier }
```

From this instant: **all new writes go to new array.**

### Main Loop: Interleaved Migration

```
loop {
    if let Some(order) = spsc.try_pop() {
        process_order(order)           // may trigger lazy migration
    } else if book.is_migrating() {
        migrate_batch(100)             // steal idle cycles, expand frontiers
    } else {
        // busy-spin: no pause/yield — dedicated core, bare loop
    }
}
```

### Lazy Migration: Frontier Advance on Access

Every level access goes through `resolve_level`. One branch on hot path:

```
fn resolve_level(&mut self, price: Price) -> &mut PriceLevel {
    if self.is_migrating() && !self.is_within_frontier(price) {
        self.advance_frontier_to(price);
    }
    let idx = self.compression.price_to_index(price.0);
    &mut self.active_levels[idx as usize]
}

fn is_within_frontier(&self, price: Price) -> bool {
    // Two comparisons
    price.0 >= self.bid_frontier && price.0 <= self.ask_frontier
}
```

`advance_frontier_to` walks from the current frontier toward the target price,
migrating every populated level it passes. Empty levels = one `order_count == 0`
check (~1ns each). A jump of 10K empty levels = ~10us.

### migrate_single_level

```
fn migrate_single_level(&mut self, old_idx: u32) {
    let old_level = &self.old_levels[old_idx];
    if old_level.order_count == 0 { return; }

    // Iterate orders — each stores its exact price.
    // Orders in a smooshed old level may have different prices,
    // mapping to different new indices (unsmooshing).
    let mut cursor = old_level.head;
    while cursor != NONE {
        let next = self.orders[cursor].next;
        let price = self.orders[cursor].price;
        let new_idx = self.compression.price_to_index(price.0);

        // Unlink from old level, append to new level
        let new_level = &mut self.active_levels[new_idx];
        if new_level.order_count > 0 {
            self.orders[new_level.tail].next = cursor;
            self.orders[cursor].prev = new_level.tail;
            self.orders[cursor].next = NONE;
            new_level.tail = cursor;
        } else {
            new_level.head = cursor;
            new_level.tail = cursor;
            self.orders[cursor].prev = NONE;
            self.orders[cursor].next = NONE;
        }
        new_level.total_qty += self.orders[cursor].qty;
        new_level.order_count += 1;
        self.orders[cursor].tick_index = new_idx;

        cursor = next;
    }
}
```

### Proactive Sweep: Expand Frontiers in Idle Cycles

```
fn migrate_batch(&mut self, batch_size: u32) {
    let mut migrated = 0;
    while migrated < batch_size {
        // Alternate: expand bid frontier down, ask frontier up
        if self.bid_frontier > self.old_min_price {
            self.bid_frontier -= self.old_tick_step;
            let old_idx = self.old_compression.price_to_index(self.bid_frontier);
            self.migrate_single_level(old_idx);
            migrated += 1;
        }
        if self.ask_frontier < self.old_max_price {
            self.ask_frontier += self.old_tick_step;
            let old_idx = self.old_compression.price_to_index(self.ask_frontier);
            self.migrate_single_level(old_idx);
            migrated += 1;
        }
        // Both edges reached → done
        if self.bid_frontier <= self.old_min_price
            && self.ask_frontier >= self.old_max_price {
            self.staging_levels = std::mem::take(&mut self.old_levels);
            self.state = BookState::Normal;
            break;
        }
    }
}
```

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

```
fn cancel_order(&mut self, handle: SlabIdx) {
    let price = self.orders[handle].price;
    self.resolve_level(price);  // ensures level is in new array
    // Cancel normally from new array
    ...
}
```

New orders always go to new array. `resolve_level` ensures the target level
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

Prices map to array indices via compressed zone lookup (see section 2.5):

```rust
struct PriceLevel {
    head: SlabIdx,      // u32 — first order (FIFO front)
    tail: SlabIdx,      // u32 — last order (FIFO back)
    total_qty: i64,     // aggregate quantity at this level
    order_count: u32,   // number of orders
}
// 24 bytes per level — compact
```

~617K slots * 24B = ~14.8 MB per array. Two arrays = ~30 MB.
The order slab is the main memory consumer (~10 GB).

### Order Slab (Arena Allocator)

```rust
const NONE: u32 = u32::MAX;

type SlabIdx = u32;

#[repr(C, align(64))]
struct OrderSlot {
    // === Cache line 1: hot fields (touched during matching) ===
    price: Price,           // i64, 8B
    remaining_qty: Qty,     // i64, 8B
    side: u8,               // 1B (0=Buy, 1=Sell)
    flags: u8,              // 1B (bit 0: is_active, bit 1: reduce_only)
    tif: u8,                // 1B (0=GTC, 1=IOC, 2=FOK)
    _pad1: [u8; 5],         // 5B alignment
    next: SlabIdx,          // u32, 4B — next in price level
    prev: SlabIdx,          // u32, 4B — prev in price level
    tick_index: u32,        // u32, 4B — backpointer to price level
    _pad2: u32,             // 4B
    // subtotal: 48B, fits in one cache line with padding to 64

    // === Cache line 2: cold fields ===
    user_id: u32,           // 4B
    sequence: u16,          // 2B — wrapping sequence, unique with timestamp
    _pad3: [u8; 2],         // 2B
    original_qty: Qty,      // i64, 8B
    timestamp_ns: u64,      // 8B — nanosecond epoch
    _pad4: [u8; 40],        // pad to 128B total (2 cache lines)
}
// Total: 128 bytes, aligned to 64B boundary
// Note: order_id and client_order_id live at the gateway layer, not here.
// The (timestamp_ns, sequence) pair uniquely identifies an order within the book.
```

**Allocation** uses a generic slab — `Vec` + free list. O(1) alloc and free
without ever shrinking the Vec (which would be O(n)). Reusable across the
codebase for anything fixed-size.

```rust
struct Slab<T> {
    slots: Vec<T>,
    free_head: u32,    // NONE = free list empty, use bump
    bump_next: u32,    // next virgin slot
}

impl<T> Slab<T> {
    fn alloc(&mut self) -> u32 {
        if self.free_head != NONE {
            let idx = self.free_head;
            self.free_head = self.slots[idx as usize].next(); // pop
            idx
        } else {
            let idx = self.bump_next;
            self.bump_next += 1;
            idx
        }
    }

    fn free(&mut self, idx: u32) {
        self.slots[idx as usize].set_next(self.free_head); // push
        self.free_head = idx;
    }
}
```

Free list chains through the slot's own `next` field — dead slots already
have it, zero extra memory. Index IS the handle — O(1) lookup, no HashMap.

For the orderbook: `Slab<OrderSlot>` pre-allocated (e.g. 78M slots = ~10 GB).

### Best Bid/Ask Tracking

```rust
struct Orderbook {
    active_levels: Vec<PriceLevel>,   // current write target
    staging_levels: Vec<PriceLevel>,  // spare for recentering
    orders: OrderSlab,

    best_bid_tick: u32,        // highest populated bid tick (NONE if empty)
    best_ask_tick: u32,        // lowest populated ask tick (NONE if empty)

    compression: CompressionMap, // zone boundaries + index mapping
    state: BookState,            // Normal or Migrating { old, frontiers }

    config: SymbolConfig,      // tick/lot curves, symbol params
    sequence: u64,             // monotonic order ID counter

    event_buf: [Event; MAX_EVENTS], // fixed array, no heap
    event_len: u32,                 // reset to 0 each cycle

    // Active user position tracking (per symbol)
    user_states: Vec<UserState>,       // indexed by active_user_id
    user_map: FxHashMap<u32, u16>,     // user_id -> active_user_id
    user_free_list: Vec<u16>,          // deferred reclamation
    user_bump: u16,                    // next virgin slot
}
```

On removal of last order at best level, scan linearly to next populated level.
In practice, spread is 1-2 ticks so this scan is O(1) amortized.

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

### Main Loop

```
fn process_new_order(book, incoming):
    book.event_len = 0    // single store, no clear needed
    let saved_event_len = book.event_len    // for FOK rollback
    validate_price_tick(incoming.price, book.config)
    validate_qty_lot(incoming.qty, incoming.price, book.config)

    // Reduce-only enforcement (before matching)
    if incoming.reduce_only:
        let user_state = book.user_map.get(&incoming.user_id)
            .map(|&idx| &book.user_states[idx as usize])
        if user_state is None or
           (incoming.side == Buy and user_state.net_qty >= 0) or
           (incoming.side == Sell and user_state.net_qty <= 0):
            book.emit(OrderFailed {
                user_id: incoming.user_id,
                reason: REDUCE_ONLY_VIOLATION })
            return
        // Clamp qty to position size
        incoming.remaining_qty = min(
            incoming.remaining_qty,
            user_state.net_qty.unsigned_abs())

    // Phase 1: Match against opposite side
    if incoming.side == Buy:
        while incoming.remaining_qty > 0 AND book.best_ask_tick != NONE:
            ask_level = book.levels[book.best_ask_tick]
            ask_price = tick_to_price(book.best_ask_tick, book)

            if incoming.price < ask_price:
                break   // incoming bid below best ask — no match

            match_at_level(book, book.best_ask_tick, incoming)

            if ask_level.order_count == 0:
                // Level exhausted, find next ask
                book.best_ask_tick = scan_next_ask(book, book.best_ask_tick)

    else: // Sell
        while incoming.remaining_qty > 0 AND book.best_bid_tick != NONE:
            bid_level = book.levels[book.best_bid_tick]
            bid_price = tick_to_price(book.best_bid_tick, book)

            if incoming.price > bid_price:
                break   // incoming ask above best bid — no match

            match_at_level(book, book.best_bid_tick, incoming)

            if bid_level.order_count == 0:
                book.best_bid_tick = scan_next_bid(book, book.best_bid_tick)

    // Phase 1.5: Time-in-force enforcement
    if incoming.tif == FOK:
        if incoming.remaining_qty > 0:
            // FOK not fully filled — reject entire order
            // Undo any fills emitted above (revert event_len)
            book.event_len = saved_event_len
            book.emit(OrderFailed {
                user_id: incoming.user_id,
                reason: FOK_NOT_FILLED })
            return

    // Phase 2: Insert remainder as resting order
    if incoming.remaining_qty > 0:
        if incoming.tif == IOC:
            // IOC: cancel remainder, don't insert
            book.emit(OrderDone {
                user_id: incoming.user_id,
                reason: CANCELLED,
                filled_qty: incoming.original_qty
                    - incoming.remaining_qty,
                remaining_qty:
                    incoming.remaining_qty })
        else:
            handle = insert_resting(book, incoming)
            book.emit(OrderInserted { handle, ... })

    // caller drains book.event_buf[0..book.event_len], then event_len resets next call
```

### Fill Logic

```
fn match_at_level(book, tick, aggressor):
    level = &book.levels[tick]
    cursor = level.head

    while cursor != NONE AND aggressor.remaining_qty > 0:
        maker = &book.orders[cursor]
        fill_qty = min(aggressor.remaining_qty, maker.remaining_qty)

        // Execute fill
        aggressor.remaining_qty -= fill_qty
        maker.remaining_qty -= fill_qty
        level.total_qty -= fill_qty

        book.emit(Fill {
            maker_order_id: maker.order_id,
            taker_order_id: aggressor.order_id,
            maker_user_id:  maker.user_id,
            taker_user_id:  aggressor.user_id,
            price:          maker.price,
            qty:            fill_qty,
            timestamp:      now_ns(),
        })

        update_positions_on_fill(book,
            aggressor.user_id, maker.user_id,
            aggressor.side, fill_qty)

        next_cursor = maker.next

        if maker.remaining_qty == 0:
            // Fully filled — remove from book
            unlink_order(book, cursor)
            free_slot(book, cursor)
            level.order_count -= 1

        cursor = next_cursor
```

### Cancel Order

```
fn cancel_order(book, handle: SlabIdx) -> Option<Event>:
    order = &book.orders[handle]
    if !order.is_active(): return None

    tick = order.tick_index
    level = &book.levels[tick]

    // Update level aggregates
    level.total_qty -= order.remaining_qty
    level.order_count -= 1

    // Unlink from doubly-linked list
    unlink_order(book, handle)

    // Update best bid/ask if this was the last order at best level
    if level.order_count == 0:
        if order.side == Buy AND tick == book.best_bid_tick:
            book.best_bid_tick = scan_next_bid(book, tick)
        elif order.side == Sell AND tick == book.best_ask_tick:
            book.best_ask_tick = scan_next_ask(book, tick)

    free_slot(book, handle)

    return Some(OrderCancelled { order_id: order.order_id, remaining_qty: order.remaining_qty })
```

---

## 6. Event Types

Fixed-size array on the Orderbook struct. `event_len = 0` resets per cycle — single
store, no clear, no heap. `emit()` writes `event_buf[event_len]` and bumps `event_len`.

```rust
const MAX_EVENTS: usize = 10_000;

enum Event {
    Fill {
        maker_handle: SlabIdx,
        taker_user_id: u32,
        price: Price,
        qty: Qty,
        side: u8,           // taker side
    },
    OrderInserted {
        handle: SlabIdx,
        user_id: u32,
        side: u8,
        price: Price,
        qty: Qty,
    },
    OrderCancelled {
        handle: SlabIdx,
        user_id: u32,
        remaining_qty: Qty,
    },
    OrderDone {
        handle: SlabIdx,    // fully filled or cancelled — order is gone
        user_id: u32,
        reason: u8,         // 0=filled, 1=cancelled
    },
    OrderFailed {
        user_id: u32,
        reason: u8,         // maps to FailureReason enum
    },
}

fn emit(&mut self, event: Event) {
    self.event_buf[self.event_len as usize] = event;
    self.event_len += 1;
}
```

`OrderDone` signals the risk engine and user that an order no longer exists (fully
filled or cancelled). Emitted after the last fill or after a cancel.

Events are drained after each order is processed. Multiple SPSC ring buffers
fan out to downstream consumers:
- Risk engine (position updates from fills, OrderDone for margin release)
- Persistence layer (trade log)
- Market data dissemination (shadow orderbook, see [MARKETDATA.md](MARKETDATA.md))
- Recorder (archival via DXS consumer, see [DXS.md](DXS.md) section 8)

### 6.5 User Position Tracking

```rust
/// Per-user position state tracked by matching engine.
/// Updated on every fill. Used for reduce-only enforcement.
struct UserState {
    user_id: u32,
    net_qty: i64,        // long - short (signed)
    order_count: u16,    // resting orders in book
    _pad: [u8; 2],
}

/// Assign active_user_id on first order for a user on this
/// symbol. Risk engine provides the mapping; ME uses it as Vec
/// index.
fn get_or_assign_user(book: &mut Orderbook, user_id: u32)
    -> u16 {
    if let Some(&idx) = book.user_map.get(&user_id) {
        return idx;
    }
    let idx = if let Some(free) = book.user_free_list.pop() {
        book.user_states[free as usize] =
            UserState::new(user_id);
        free
    } else {
        let idx = book.user_bump;
        book.user_bump += 1;
        book.user_states.push(UserState::new(user_id));
        idx
    };
    book.user_map.insert(user_id, idx);
    idx
}

/// On fill: update both taker and maker positions.
fn update_positions_on_fill(book: &mut Orderbook,
    taker_user_id: u32, maker_user_id: u32,
    taker_side: Side, qty: i64) {
    let sign = if taker_side == Buy { 1 } else { -1 };
    let taker_idx = get_or_assign_user(book, taker_user_id);
    book.user_states[taker_idx as usize].net_qty +=
        sign * qty;
    let maker_idx = get_or_assign_user(book, maker_user_id);
    book.user_states[maker_idx as usize].net_qty -=
        sign * qty;
}
```

Active user_id lifecycle:
- Assign on first order for this symbol (risk supplies mapping).
- Reclaim only when `net_qty == 0 && order_count == 0` for a grace
  period (300s) to avoid churn. Grace period is disabled
  during WAL replay (reclamation deferred until live).
- Reuse reclaimed slots via free list.

How events are consistently delivered to these systems, ordering guarantees,
and failure handling are covered in [CONSISTENCY.md](CONSISTENCY.md).

---

## 7. Memory Layout & Performance

### Pre-allocate Everything

```
Component           Sizing                        Memory
Order slab          78,000,000 slots * 128B       ~10 GB
Price levels (x2)   617,000 slots * 24B * 2       ~30 MB
CompressionMap      5 zones, constant              ~200 B
Event buffer        [Event; 10,000] fixed array     1.3 MB
Migration state     2 x i64 (bid/ask frontier)     16 B
                                                  --------
Total per book                                    ~10 GB
```

The order slab dominates. Price level arrays are negligible thanks to
compressed indexing. Two arrays always pre-allocated (active + staging).

### Cache Optimization

- **Hot/cold split**: Matching only touches cache line 1 of OrderSlot (48 bytes)
- **Contiguous slab**: Orders in `Vec<OrderSlot>` — sequential memory, prefetch-friendly
- **Compact PriceLevel**: 24 bytes — multiple levels fit in one cache line
- **No pointers**: Only u32 slab indices — half the size of pointers on x86_64
- **No String/Vec/Box**: Everything fixed-size, Copy, no heap indirection
- **Alignment**: `#[repr(C, align(64))]` on OrderSlot for cache line alignment

### Zero Allocation Hot Path

- No `malloc`/`new` during matching — slab provides all storage
- Event buffer is a fixed array — `event_len = 0` resets per cycle (single store)
- No `Vec` growth during matching (capacity pre-reserved)
- No `String` formatting or logging on hot path

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
