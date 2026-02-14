# Building a Matching Engine in Rust

The matching engine is the core of any exchange. It receives orders,
matches them against resting liquidity, produces fills, and maintains
the orderbook. In RSX, each symbol gets its own matching engine
instance running on a dedicated pinned core. This post walks through
the data structures and algorithms.

## The Orderbook

An orderbook is a collection of resting orders organized by price.
Buy orders (bids) are sorted highest-first. Sell orders (asks) are
sorted lowest-first. When a new order arrives, it matches against
the best price on the opposite side.

The naive implementation -- a `BTreeMap<Price, Vec<Order>>` -- has
two problems. First, the BTree allocates nodes on the heap during
insertion. Second, the `Vec<Order>` at each price level reallocates
when it grows. Both cause unpredictable latency.

### Slab Allocator

We store all orders in a pre-allocated slab:

```rust
pub struct Slab<T: SlabItem> {
    slots: Vec<T>,
    free_head: u32,
    bump_next: u32,
}
```

`slots` is allocated once at startup. `alloc()` pops from the free
list (O(1)) or bumps the pointer. `free()` pushes back to the free
list. All operations are O(1) with no heap allocation.

Each order slot contains the order data plus linked-list pointers:

```rust
pub struct OrderSlot {
    pub price: Price,
    pub remaining_qty: Qty,
    pub original_qty: Qty,
    pub side: Side,
    pub tif: u8,
    pub user_id: u32,
    pub reduce_only: bool,
    pub timestamp_ns: u64,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub tick_index: u32,
    pub prev: u32,
    pub next: u32,
}
```

Orders at the same price level form a doubly-linked list through
the slab. Insertion at tail is O(1). Removal (cancel) is O(1)
given the handle. No pointer chasing through heap-allocated nodes.

The slab tracks its own allocation health. The invariant
`allocated = free + active` is verified in tests after every
operation sequence.

### CompressionMap: Price to Array Index

The orderbook needs to look up price levels by price. A hash map
would work but adds cache misses. Instead, we map prices to
contiguous array indices.

```rust
pub struct CompressionMap {
    pub mid_price: i64,
    pub thresholds: [i64; 4],
    pub compressions: [u32; 5],
    pub base_indices: [u32; 5],
    pub zone_slots: [u32; 5],
}
```

Five zones radiate outward from the mid price:

| Zone | Distance | Resolution |
|------|----------|------------|
| 0 | 0-5% | 1 tick per slot |
| 1 | 5-15% | 2 ticks per slot |
| 2 | 15-30% | 4 ticks per slot |
| 3 | 30-50% | 8 ticks per slot |
| 4 | 50%+ | catch-all (2 slots) |

Zone 0 covers the spread and nearby prices with full tick
resolution. Distant zones compress multiple ticks into one slot.
This is called "smooshed tick" matching -- orders at different
prices within one compressed slot still match at their individual
prices, but they share a price level for queue management.

The total number of slots is bounded (typically ~15,000) and
allocated as a flat array. `price_to_index()` is a branch-free
computation: subtract mid, check thresholds, divide by zone
compression, add base index.

When the market moves significantly, the CompressionMap migrates.
Migration uses a lazy frontier: levels are moved on access, bounded
by the old min/max price. This avoids a stop-the-world migration
at the cost of slightly slower access during the transition.

### Price Level

Each slot in the level array is a `PriceLevel`:

```rust
pub struct PriceLevel {
    pub head: u32,        // first order handle
    pub tail: u32,        // last order handle
    pub total_qty: i64,
    pub order_count: u32,
}
```

FIFO ordering is maintained by always inserting at the tail and
matching from the head. Time priority within a price level is
guaranteed by the linked list order.

## The Matching Algorithm

The entry point is `process_new_order()`. Here is the flow:

### 1. Validation

```rust
if \!validate_order(&book.config, Price(order.price), Qty(order.qty)) {
    book.emit(Event::OrderFailed {
        user_id: order.user_id,
        reason: FAIL_VALIDATION,
    });
    return;
}
```

Price must be a multiple of tick_size. Quantity must be a multiple
of lot_size. Both must be positive. This happens once per order,
not per fill.

### 2. Reduce-Only Check

Reduce-only orders can only decrease an existing position. A buy
reduce-only is rejected if the user has no short position (or a
net long). The order quantity is clamped to the position size:

```rust
let abs_pos = nq.unsigned_abs() as i64;
if order.remaining_qty > abs_pos {
    order.remaining_qty = abs_pos;
}
```

### 3. Post-Only Check

Post-only orders must not cross the spread. If a buy order's price
is at or above the best ask, it would immediately match -- so we
reject it:

```rust
let would_cross = match order.side {
    Side::Buy => {
        book.best_ask_tick \!= NONE
            && order_tick >= book.best_ask_tick
    }
    Side::Sell => {
        book.best_bid_tick \!= NONE
            && order_tick <= book.best_bid_tick
    }
};
if would_cross {
    book.emit(Event::OrderCancelled { ... });
    return;
}
```

### 4. Matching Loop

The matching loop walks through price levels on the opposite side:

```rust
Side::Buy => {
    while order.remaining_qty > 0
        && book.best_ask_tick \!= NONE
    {
        let ask_tick = book.best_ask_tick;
        match_at_level(book, ask_tick, order);
        let level = &book.active_levels[ask_tick as usize];
        if level.order_count == 0 {
            book.best_ask_tick = book.scan_next_ask(ask_tick);
        }
    }
}
```

At each level, `match_at_level()` walks the FIFO queue:

```rust
let fill_qty = aggressor.remaining_qty.min(maker_qty);
aggressor.remaining_qty -= fill_qty;
maker_slot.remaining_qty.0 -= fill_qty;

book.emit(Event::Fill {
    maker_handle: cursor,
    maker_user_id,
    taker_user_id: aggressor.user_id,
    price: Price(maker_price),
    qty: Qty(fill_qty),
    side: aggressor.side as u8,
    ...
});
```

Fills always execute at the maker's price. The aggressor walks
through resting orders in time priority (FIFO within each price
level, price priority across levels).

When a maker order is fully filled, it is unlinked from the
doubly-linked list and returned to the slab's free list. The
level's order count and total quantity are updated.

### 5. Time-in-Force Handling

After the matching loop, the remaining quantity determines what
happens:

- **FOK (Fill-or-Kill):** If any quantity remains, roll back all
  fills from this order (by resetting `event_len`) and emit
  `OrderFailed`.
- **IOC (Immediate-or-Cancel):** Emit `OrderDone` with the
  partially filled quantity. No resting order.
- **GTC (Good-til-Cancel):** Insert the remainder as a resting
  order via `insert_resting()`.

### 6. BBO Emission

After processing, if the best bid or ask changed, the engine emits
a BBO event routed to the risk engine (for mark price / margin
recalc) and market data:

```rust
if book.best_bid_tick \!= old_bid || book.best_ask_tick \!= old_ask {
    emit_bbo(book);
}
```

## Deduplication

Orders arrive over UDP (CMP protocol), which does not guarantee
exactly-once delivery. The matching engine tracks recently seen
order IDs in a `DedupTracker`:

```rust
pub struct DedupTracker {
    seen: HashMap<Key, ()>,
    pruning_queue: VecDeque<(Key, Instant)>,
    last_cleanup: Instant,
}
```

The dedup window is 5 minutes. On duplicate detection, the engine
emits `OrderFailed` with a duplicate reason and writes an
`OrderAcceptedRecord` to the WAL for the first occurrence. The
pruning queue ensures the map does not grow without bound.

## Event Buffer

The matching engine does not directly call into the risk engine or
network. Instead, it writes events to an in-process buffer:

```rust
pub fn emit(&mut self, event: Event) {
    self.events[self.event_len] = event;
    self.event_len += 1;
}
```

The buffer is a fixed-size array. After `process_new_order()`
returns, the caller drains events to SPSC rings (one for risk,
one for market data) and to the WAL writer. This separation
means the matching algorithm has no knowledge of networking,
persistence, or downstream consumers.

Fan-out to multiple consumers uses separate SPSC rings. A slow
market data consumer does not stall the risk feed. Each ring has
independent backpressure.

## Snapshot Save/Load

The orderbook supports binary serialization for fast recovery.
On crash, the matching engine loads the latest snapshot, then
replays WAL records from `snapshot.last_seq + 1`. Snapshots
capture the full slab, compression map, price levels, and user
state. Serialization blocks during migration (the lazy frontier
must complete first).

## What We Learned

The matching engine is roughly 400 lines of core logic across
`matching.rs`, `book.rs`, and `slab.rs`. The simplicity comes
from the data structure choices: a slab eliminates allocation
complexity, a compression map eliminates lookup complexity, and
doubly-linked lists at each level eliminate queue management
complexity.

The event buffer pattern -- emit events, let the caller decide
where they go -- keeps the matching algorithm pure. We can test
it without any network or persistence infrastructure. The 97
orderbook tests and 30 matching tests run in under a second
because they exercise the algorithm directly, not through a
simulated network stack.
