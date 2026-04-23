# rsx-book Architecture

Shared orderbook with slab-allocated orders, compressed price
levels, and price-time priority matching.
See `specs/2/21-orderbook.md`.

## Module Layout

| File | Purpose |
|------|---------|
| `book.rs` | `Orderbook` struct, insert/cancel/modify, BBA scan |
| `matching.rs` | `process_new_order()`, `match_at_level()` -- GTC/IOC/FOK/post-only/reduce-only |
| `slab.rs` | Generic `Slab<T>` arena allocator |
| `compression.rs` | `CompressionMap` -- 5-zone price-to-index bisection |
| `level.rs` | `PriceLevel` -- doubly-linked list node |
| `order.rs` | `OrderSlot` -- 128-byte cache-aligned order |
| `event.rs` | `Event` enum, event constants |
| `user.rs` | `UserState`, position tracking within the book |
| `migration.rs` | Lazy level migration during recentering |
| `snapshot.rs` | Binary snapshot save/load with magic + version header |

## Key Types

- `Orderbook` -- central struct: levels, slab, BBA tracking,
  user state, migration state, event buffer
- `BookState` -- `Normal` or `Migrating`
- `OrderSlot` -- 128-byte `#[repr(C, align(64))]` order record,
  hot fields in first cache line
- `PriceLevel` -- head/tail slab handles, total qty, order
  count (24 bytes)
- `Slab<T>` -- generic arena allocator with free-list + bump
- `CompressionMap` -- 5-zone price-to-index mapping
- `Event` -- `Fill`, `OrderInserted`, `OrderCancelled`,
  `OrderDone`, `OrderFailed`, `BBO`
- `IncomingOrder` -- input to `process_new_order()`
- `UserState` -- per-user net position and active order count

## Orderbook Structure

```
          Orderbook
         /         \
    Bid Levels      Ask Levels
  (compressed)    (compressed)
       |               |
  [PriceLevel]    [PriceLevel]
   head -> Order -> Order -> (tail)
            |        |
        (doubly linked, stored in slab)
```

**PriceLevel** (24 bytes): head, tail (SlabIdx), total_qty
(i64), order_count (u32).

**OrderSlot** (128 bytes, `#[repr(C, align(64))]`):
- Cache line 1 (hot): price, remaining_qty, side, flags,
  tif, next/prev/tick_index
- Cache line 2 (cold): user_id, sequence, original_qty,
  timestamp_ns

## Slab Arena Allocator

`Slab<OrderSlot>` -- pre-allocated Vec + free list. O(1)
alloc and free. No heap allocation during matching. Free list
chains through each slot's `next` field. Pre-allocated for
~78M slots (~10 GB).

## CompressionMap (Price-to-Index)

Distance-based compression zones centered on mid-price.
Bounds the price level array to ~617K slots (~15 MB).

```
Zone  Distance from mid  Compression  Slot covers
0     0-5%               1:1          1 market tick
1     5-15%              1:10         10 market ticks
2     15-30%             1:100        100 market ticks
3     30-50%             1:1000       1000 market ticks
4     50%+               CATCH-ALL    single slot/side
```

Lookup: bisection on 4 thresholds (~2-5ns).

## Order Types

| Type | Behavior |
|------|----------|
| GTC (Limit) | Match, rest remainder |
| IOC | Match, cancel remainder |
| FOK | Match all or reject (rollback fills) |
| Post-Only | Reject if would take |
| Reduce-Only | Clamp qty to position, reject if wrong side |

## Matching Algorithm

```
process_new_order(book, incoming):
    validate tick/lot
    reduce-only enforcement (clamp qty)

    Phase 1: match against opposite side
        walk best levels, FIFO within each level
        smooshed levels: scan, check exact price

    Phase 1.5: TIF enforcement
        FOK not fully filled -> rollback events, reject
        IOC remainder -> emit OrderDone(CANCELLED)

    Phase 2: insert remainder as resting GTC
        slab alloc, link into price level
```

## Smooshed Ticks

In zones 1-4, multiple market prices share one slot. Each
order stores its exact price. During matching, scanner checks
actual price per order. O(k) per smooshed slot. Near mid
(zone 0): 1:1, no smooshing.

## Incremental Recentering

When mid drifts >50% of zone 0 width, migrate to new array:
- Two pre-allocated arrays (active + staging)
- Lazy: `resolve_level(price)` migrates on access
- Proactive: `migrate_batch(100)` in idle cycles
- No stop-the-world pause

## Event Buffer and Fanout

Fixed array `[Event; 10_000]` on Orderbook struct. Reset per
cycle. Two independent CmpSenders:
- ME -> Risk: fills, BBO, order done/failed
- ME -> Marketdata: inserts, cancels, fills

## Memory Layout

```
Component          Sizing                      Memory
Order slab         78M slots * 128B            ~10 GB
Price levels (x2)  617K slots * 24B * 2        ~30 MB
Event buffer       [Event; 10K] fixed          ~1.3 MB
Total per book                                 ~10 GB
```

## Operation Complexity

| Operation | Time |
|-----------|------|
| Add order | O(1) (bisect + alloc + append) |
| Cancel by handle | O(1) (lookup + unlink + free) |
| Match (zone 0) | O(1) per fill |
| Match (smooshed) | O(k) per slot |
| Best bid/ask | O(1) (cached) |
