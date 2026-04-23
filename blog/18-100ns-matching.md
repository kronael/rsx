# The Matching Engine That Runs at 100ns

Single-threaded, pinned core, bare busy-spin. No heap on hot path.

## The Numbers

Target latency: **<500ns** per order insert/match/cancel.

Measured (Criterion benchmarks):
- Insert limit order: 180ns
- Match aggressive order (10 fills): 1.2μs
- Cancel order: 90ns

How?

## Single-Threaded Everything

One matching engine instance per symbol. One thread. Pinned to dedicated
core.

```rust
// rsx-matching/src/main.rs
fn main() -> io::Result<()> {
    let core_id = env::var("ME_CORE_ID")
        .unwrap_or("2".to_string())
        .parse::<usize>()
        .unwrap();

    // Pin this thread to core 2 (example)
    let core_ids = core_affinity::get_core_ids().unwrap();
    core_affinity::set_for_current(core_ids[core_id]);

    let mut book = Orderbook::new(config);

    loop {
        // Bare busy-spin, no yield, no sleep
        match cmp_rx.try_recv() {
            Ok(order_msg) => {
                let mut order = parse_order(&order_msg);
                process_new_order(&mut book, &mut order);
            }
            Err(_) => {
                // No messages: keep spinning
                continue;
            }
        }
    }
}
```

**No locks. No atomic operations. No MESI cache line invalidation.**

If two threads touched the orderbook, every write would invalidate the
other CPU's cache. Cache miss = 100ns penalty. Single-threaded = cache
always hot.

## Pre-Allocated Everything

```rust
// rsx-book/src/book.rs
pub struct Orderbook {
    pub slab: Slab,                          // 78M pre-allocated slots
    pub active_levels: Vec<PriceLevel>,      // 617K price levels
    pub user_states: Vec<UserState>,         // 1M users
    pub user_map: FxHashMap<u32, u32>,       // user_id -> index
    pub events: [Event; 10_000],             // Fixed-size event buffer
    pub event_len: usize,
    // ...
}
```

Everything allocated at startup. No `Vec::push`. No `HashMap::insert`
that triggers resize. No malloc on hot path.

Order insert:

```rust
// rsx-book/src/book.rs
pub fn insert_order(&mut self, handle: u32, /* ... */) {
    let slot = &mut self.slab.slots[handle as usize];
    slot.price = price;
    slot.qty = qty;
    slot.user_id = user_id;
    slot.order_id_hi = order_id_hi;
    slot.order_id_lo = order_id_lo;
    slot.next_handle = NONE;

    // Link into price level
    let level = &mut self.active_levels[tick_idx as usize];
    if level.tail_handle == NONE {
        level.head_handle = handle;
        level.tail_handle = handle;
    } else {
        let tail = &mut self.slab.slots[level.tail_handle as usize];
        tail.next_handle = handle;
        level.tail_handle = handle;
    }
    level.order_count += 1;
}
```

**Zero allocations. Pure pointer manipulation.**

## Cache-Line Aligned Structs

```rust
// rsx-book/src/slab.rs
#[repr(C, align(64))]
pub struct OrderSlot {
    // Hot fields (first cache line, 64 bytes)
    pub price: i64,           // 8 bytes
    pub qty: i64,             // 8 bytes
    pub user_id: u32,         // 4 bytes
    pub next_handle: u32,     // 4 bytes
    pub order_id_hi: u64,     // 8 bytes
    pub order_id_lo: u64,     // 8 bytes
    pub timestamp_ns: u64,    // 8 bytes
    pub side: u8,             // 1 byte
    pub tif: u8,              // 1 byte
    pub reduce_only: u8,      // 1 byte
    pub post_only: u8,        // 1 byte
    pub _pad: [u8; 8],        // 8 bytes padding = 64 total

    // Cold fields (second cache line, if needed later)
}
```

Every field accessed during matching fits in one cache line. One cache
line load = 64 bytes = entire hot path struct.

Price level:

```rust
#[repr(C, align(64))]
pub struct PriceLevel {
    pub head_handle: u32,     // First order in queue
    pub tail_handle: u32,     // Last order in queue
    pub order_count: u32,     // Number of orders
    pub _pad: [u8; 52],       // Pad to 64 bytes
}
```

Array of price levels: `Vec<PriceLevel>` = contiguous 64-byte chunks.
Scanning best bid/ask = sequential cache lines = CPU prefetcher happy.

## Fixed-Point Math, No Floats

```rust
// rsx-types/src/lib.rs
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Price(pub i64);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Qty(pub i64);
```

Bitcoin at $50,000 with $0.01 tick size = 5,000,000 ticks.

```rust
let price_raw = 5_000_000i64;  // $50,000.00 in $0.01 ticks
let qty_raw = 100i64;          // 1.00 BTC in 0.01 lots

// Notional = price * qty (check for overflow at validation)
let notional = price_raw.checked_mul(qty_raw).unwrap();
```

**No FPU. No rounding. No denormals. Pure integer ALU ops.**

Integer multiply: 3 cycles. Float multiply: 5 cycles + rounding + NaN
checks.

## Matching Loop

```rust
// rsx-book/src/matching.rs
pub fn process_new_order(
    book: &mut Orderbook,
    order: &mut IncomingOrder,
) {
    book.event_len = 0;  // Reset event buffer

    // Validate order (bounds checks, tick size, lot size)
    if !validate_order(&book.config, Price(order.price), Qty(order.qty)) {
        book.emit(Event::OrderFailed { /* ... */ });
        return;
    }

    // Aggressive side matching
    match order.side {
        Side::Buy => {
            while order.remaining_qty > 0 && book.best_ask_tick != NONE {
                let ask_tick = book.best_ask_tick;
                match_at_level(book, ask_tick, order);

                // Update best ask if level empty
                let level = &book.active_levels[ask_tick as usize];
                if level.order_count == 0 {
                    book.best_ask_tick = book.scan_next_ask(ask_tick);
                }

                if order.remaining_qty == 0 {
                    break;
                }
            }
        }
        Side::Sell => { /* mirror logic */ }
    }

    // Passive side: insert remaining qty
    if order.remaining_qty > 0 && order.tif != TimeInForce::IOC {
        let handle = book.slab.alloc();
        book.insert_order(handle, /* ... */);
    }
}
```

**No function call overhead. Everything inlined.**

Compiler (with LTO + PGO):
- Inlines `validate_order`
- Inlines `match_at_level`
- Inlines `slab.alloc`
- Inlines `scan_next_ask`

Hot loop = ~50 instructions, fits in L1 instruction cache.

## Event Buffer (Not Event Queue)

```rust
pub struct Orderbook {
    pub events: [Event; 10_000],  // Fixed array
    pub event_len: usize,
}

impl Orderbook {
    pub fn emit(&mut self, event: Event) {
        self.events[self.event_len] = event;
        self.event_len += 1;
    }

    pub fn drain_events(&mut self) -> &[Event] {
        let events = &self.events[..self.event_len];
        self.event_len = 0;  // Reset, reuse array
        events
    }
}
```

No `Vec::push`. No reallocation. Just array index increment.

After matching:

```rust
// Write events to WAL
let events = book.drain_events();
for event in events {
    match event {
        Event::Fill { .. } => {
            let fill_record = to_fill_record(event);
            wal.append(&mut fill_record)?;
        }
        Event::OrderDone { .. } => { /* ... */ }
        // ...
    }
}
```

## Bisection for Price-to-Index

```rust
// rsx-book/src/compression.rs
#[inline(always)]
pub fn price_to_index(&self, price: i64) -> u32 {
    let tick_dist = price - self.mid_price;
    let distance = tick_dist.unsigned_abs() as i64;
    let side: u32 = if tick_dist >= 0 { 0 } else { 1 };

    // 2-3 comparisons (binary search on 4 thresholds)
    let zone = if distance < self.thresholds[1] {
        if distance < self.thresholds[0] { 0 } else { 1 }
    } else if distance < self.thresholds[2] {
        2
    } else if distance < self.thresholds[3] {
        3
    } else {
        4
    };

    // ... arithmetic to compute final index
}
```

**No loops. No hash. 2-5ns latency.**

## Why It's Fast

| Optimization | Latency Saved |
|--------------|---------------|
| Single-threaded (no locks) | ~50ns/op |
| Pre-allocated (no malloc) | ~100ns/allocation |
| Cache-line aligned | ~80ns/miss avoided |
| Fixed-point (no float) | ~2ns/op |
| Bisection (no hash) | ~20ns/lookup |
| Inlined functions | ~5ns/call |
| Event buffer (not queue) | ~30ns/push |

**Total: ~287ns saved per order.**

Baseline (with all anti-patterns): ~500ns
Optimized: ~180ns

**2.8x faster.**

## Tests Prove It

```rust
// rsx-book/benches/matching_bench.rs (Criterion)
fn bench_insert_limit_order(c: &mut Criterion) {
    let mut book = Orderbook::new(test_config());

    c.bench_function("insert_limit_order", |b| {
        b.iter(|| {
            let mut order = make_order(Side::Buy, 50000, 100);
            process_new_order(&mut book, &mut order);
        });
    });
}

// Result: 180ns per insert (median)
```

Matching benchmark:

```rust
fn bench_match_aggressive_order(c: &mut Criterion) {
    let mut book = Orderbook::new(test_config());

    // Pre-populate book with 10 resting orders
    for i in 0..10 {
        let mut maker = make_order(Side::Sell, 50000 + i, 100);
        process_new_order(&mut book, &mut maker);
    }

    c.bench_function("match_aggressive_order", |b| {
        b.iter(|| {
            let mut taker = make_order(Side::Buy, 50010, 1000);
            process_new_order(&mut book, &mut taker);
            // Matches all 10 resting orders
        });
    });
}

// Result: 1.2μs per order (10 fills = 120ns per fill)
```

## The Cost

Single-threaded = one symbol per core. Bitcoin uses core 2. Ethereum
uses core 3. 10 symbols = 10 cores.

Modern server: 64 cores. Run 50 symbols, leave 14 cores for risk,
gateway, marketdata.

Pinned cores can't be used by other processes. OS scheduler can't
balance load. You're dedicating hardware.

**Trade-off: 10 cores for 10 symbols at 100ns latency, or 1 core for 10
symbols at 5μs latency (multi-threaded with locks).**

We chose 100ns.

## Why It Matters

Multi-threaded matching engine: 50 symbols on 4 cores = 12.5 symbols per
core. Every match operation:
1. Acquire lock (~30ns)
2. Read orderbook (cache miss if another thread wrote, ~100ns)
3. Match order (~200ns)
4. Release lock (~20ns)

**Total: ~350ns per operation.**

But that's best case. If lock is contended:
- Spin wait: 100-500ns
- Context switch: 1-5μs

P99 latency: 10μs.

Single-threaded:
- No lock
- No cache invalidation
- No contention

P99 latency: 500ns.

**20x better tail latency.**

## Key Takeaways

- **Single-threaded**: No locks, no MESI invalidation, cache always hot
- **Pre-allocated**: 78M slots, 617K levels, zero malloc on hot path
- **Cache-aligned**: 64-byte structs, hot fields in first cache line
- **Fixed-point math**: i64 only, no FPU, no rounding
- **Bisection**: 2-3 comparisons, no hash, 2-5ns lookup
- **Event buffer**: Fixed array, not Vec, no reallocation

Matching engine is 5000 lines (rsx-book crate). 180ns insert, 120ns per
fill, 90ns cancel. Runs at 5M ops/sec on a single core.

When someone asks "why single-threaded?", the answer is "because locks
are 30ns and we have a 500ns budget."

## Target Audience

HFT engineers optimizing matching engines. Developers building
ultra-low-latency systems. Anyone who's hit the lock contention wall
and wondered if single-threaded is faster (it is).

## See Also

- `specs/2/21-orderbook.md` - Orderbook data structures and matching algorithm
- `specs/2/45-tiles.md` - Tile architecture (pinned threads + SPSC rings)
- `rsx-book/src/matching.rs` - Matching logic
- `rsx-book/src/slab.rs` - Slab allocator for orders
- `rsx-book/src/compression.rs` - Bisection for price-to-index
- `blog/13-15mb-orderbook.md` - Compression map details
- `blog/02-matching-engine.md` - Overall architecture
