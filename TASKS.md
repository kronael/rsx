# RSX Tasks

## Crates to Build

### smrb - Shared Memory Ring Buffer Crate

**Status:** TODO
**Priority:** High
**Used by:** Exchange (risk engine <-> orderbook), general-purpose IPC

**Goal:** Self-contained, generally usable Rust crate for ultra-low-latency
inter-process and inter-thread communication.

**Requirements:**
- Lock-free SPSC ring buffer (~50-100ns per op)
- Support both inter-thread (in-process) and inter-process (shared memory) modes
- `shm_open` / `mmap` backed shared memory for cross-process use
- Cache-line padded read/write indices (avoid false sharing)
- Power-of-2 capacity (bitwise AND instead of modulo)
- Zero-copy where possible — flat structs, no serialization
- `no_std` compatible core (with optional `std` feature for shm_open/mmap)
- Safe Rust API, unsafe internals only where necessary
- Busy-spin and blocking consumer modes
- Huge page support (optional feature)

**API Sketch:**
```rust
// In-process (thread-to-thread)
let (producer, consumer) = smrb::RingBuffer::<Order>::new(8192);

// Cross-process (shared memory)
let producer = smrb::SharedProducer::<Order>::create("/smrb-risk-ob", 8192)?;
let consumer = smrb::SharedConsumer::<Order>::open("/smrb-risk-ob")?;
```

**Stretch goals:**
- Seqlock mode for latest-value-only use cases (market data)
- Benchmarks vs rtrb, crossbeam, unix domain sockets
- Optional MPSC mode

### orderbook - Limit Order Book Crate

**Status:** TODO
**Priority:** High
**Used by:** Exchange matching engine
**Dependencies:** none (uses only primitives)
**Design doc:** [ORDERBOOK.md](ORDERBOOK.md)

**Goal:** Self-contained, high-performance limit order book for crypto perpetuals.

**Requirements:**
- Array-indexed price levels + slab arena for orders
- GTC limit order matching (price-time priority)
- Fixed-point `i64` price/qty — no floating point
- Tick size & lot size curves (price-dependent bands)
- Pre-allocated, zero-allocation hot path
- O(1) add, cancel, match, best bid/ask
- Event generation (fills, inserts, cancels)
- Hot/cold field splitting in Order struct (cache-line aligned)

**API Sketch:**
```rust
let config = SymbolConfig::new("BTC-PERP")
    .price_decimals(2)
    .tick_bands(vec![
        TickBand { min_price: 0, tick_size: 10, lot_size: 100 },
        TickBand { min_price: 100_000, tick_size: 100, lot_size: 10 },
    ]);

let mut book = Orderbook::new(config, 1_000_000); // 1M order capacity

let events = book.new_order(NewOrder {
    user_id: 42,
    side: Side::Buy,
    price: Price(5_000_000),  // $50,000.00
    qty: Qty(1_000),          // 1.0 BTC
});

let events = book.cancel(order_handle);
```

**Benchmarks:**
- Orders/sec throughput
- Match latency (p50, p99, p99.9)
- Cancel latency
- Memory usage per 1M orders
