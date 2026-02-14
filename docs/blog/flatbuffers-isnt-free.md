# FlatBuffers Isn't Free

FlatBuffers promises zero-copy deserialization. It delivers. But in a
low-latency trading system, "zero-copy read" doesn't mean "zero cost."

Here's what FlatBuffers actually costs you.

## Write-Side Overhead

FlatBuffers is zero-copy on read. Not on write.

```rust
// Building a FlatBuffers message
let mut builder = FlatBufferBuilder::new();

let symbol = builder.create_string("BTCUSD");
let mut trade_builder = TradeBuilder::new(&mut builder);
trade_builder.add_symbol(symbol);
trade_builder.add_price(50000.0);
trade_builder.add_qty(1.5);
let trade = trade_builder.finish();

builder.finish(trade, None);
let bytes = builder.finished_data();
```

What just happened:
1. Allocated builder buffer (dynamic, grows)
2. Wrote string, returned offset
3. Built vtable for `Trade`
4. Wrote struct fields in reverse order (FlatBuffers writes back-to-front)
5. Deduplicated vtables (hashtable lookup)
6. Finalized buffer (wrote root offset)

Compare to raw struct:

```rust
let trade = Trade {
    symbol: BTCUSD,  // u32 enum
    price: 50000.0,
    qty: 1.5,
};
let bytes: &[u8] = bytemuck::bytes_of(&trade);
```

Benchmark: 8ns (raw) vs 180ns (FlatBuffers). That's 22x slower on the write
path.

In a trading system, writes are your hot path. Every market data tick, every
order update, every fill—written once, read once. You just paid 170ns extra.

## Wire Size Bloat

FlatBuffers adds metadata for forward compatibility. This isn't free.

```rust
// Raw struct: 16 bytes
#[repr(C)]
struct Trade {
    symbol: u32,    // 4 bytes
    price: f64,     // 8 bytes
    qty: f32,       // 4 bytes
}

// FlatBuffers: ~40 bytes
// - 4 byte root offset
// - 4 byte vtable offset
// - vtable: 6 bytes (size + field offsets)
// - fields: 16 bytes
// - padding: ~10 bytes (alignment)
```

That's 2.5x size overhead. For a market data feed pushing 100k msgs/sec, you
just went from 1.6 MB/s to 4 MB/s. Cache pressure, NIC bandwidth, kernel
buffers—all affected.

Nested structures make this worse:

```rust
table OrderBook {
    bids: [Level];   // vector = 12 bytes overhead
    asks: [Level];   // another 12 bytes
}

table Level {
    price: double;   // vtable overhead per level
    qty: double;
}
```

10 levels per side = 20 vtables. Each vtable is 6+ bytes. You're now at 3-4x
wire size.

## Pointer Chasing

FlatBuffers stores offsets, not inline data. Reading nested structures means
pointer chasing.

```rust
let book = get_root_as_order_book(bytes);
let bids = book.bids();  // load offset, add to base, deref
let best_bid = bids.get(0);  // load vector length, bounds check, offset calc
let price = best_bid.price();  // load vtable offset, deref field offset, deref
```

Cache miss on vtable, cache miss on vector, cache miss on level. That's 3
memory loads for a single price read.

Raw struct:

```rust
let book: &OrderBook = bytemuck::from_bytes(bytes);
let price = book.bids[0].price;  // one load
```

Benchmark: 2ns (raw) vs 15ns (FlatBuffers). The 7x difference is cache misses.

## Rust Ecosystem Maturity

FlatBuffers' Rust support is second-class. The official compiler generates
verbose, unidiomatic code. Error handling is weak. Debugging is painful.

```rust
// Generated FlatBuffers code
pub struct Trade<'a> {
    pub _tab: flatbuffers::Table<'a>,
}

// How you actually want to work:
pub struct Trade {
    pub symbol: Symbol,
    pub price: f64,
    pub qty: f32,
}
```

No derives, no pattern matching, no destructuring. You're stuck with accessor
methods and lifetime annotations everywhere.

Third-party crates like `planus` improve this, but you're betting on
less-maintained tooling.

## Awkward Mutation

FlatBuffers is immutable by design. Updating a field means rebuilding the
entire message.

```rust
// Want to update qty? Rebuild everything.
let old_trade = get_root_as_trade(bytes);
let mut builder = FlatBufferBuilder::new();

let symbol = builder.create_string(old_trade.symbol());
let mut new_trade = TradeBuilder::new(&mut builder);
new_trade.add_symbol(symbol);
new_trade.add_price(old_trade.price());
new_trade.add_qty(new_qty);  // only thing that changed

let trade = new_trade.finish();
builder.finish(trade, None);
```

Compare to:

```rust
trade.qty = new_qty;
```

For an order book, this is killer. Every price update = full rebuild.

## When It's Worth It

FlatBuffers wins when:

1. **Schema evolution matters** - External APIs, long-lived data, multi-version
   systems
2. **Read-heavy workloads** - Broadcast to many consumers, replay from disk
3. **Large nested structures** - Deep trees where partial access saves
4. **Untrusted inputs** - Schema validation, bounds checking, buffer overrun
   protection

## When Raw Structs Are Better

Use raw structs (with `zerocopy`/`bytemuck`) when:

1. **Latency-critical hot path** - Matching engine, signal generation,
   risk checks
2. **Internal IPC** - Processes you control, same version, same endianness
3. **Flat data** - Market data ticks, order updates, fills
4. **Write-heavy** - Order entry, modification, cancellation

## The Hybrid Strategy

Best of both worlds:

- **External APIs**: FlatBuffers (evolution, safety, cross-language)
- **Internal IPC**: Raw structs over SMRB (latency, throughput)
- **Boundary**: Gateway normalizes FlatBuffers → internal structs

```rust
// Gateway receives FlatBuffers from exchange
let fb_trade = get_root_as_trade(wire_bytes);

// Normalize to internal struct
let trade = Trade {
    symbol: Symbol::from_str(fb_trade.symbol()),
    price: fb_trade.price(),
    qty: fb_trade.qty(),
    timestamp: now(),
};

// Write raw struct to SMRB
smrb.write(bytemuck::bytes_of(&trade));
```

Now your hot path (normalizer → strategy → engine) is raw structs. Your cold
path (exchange → gateway) is FlatBuffers.

## Benchmarks

Microbenchmark (1M iterations, simple trade message):

```
Operation           Raw Struct    FlatBuffers   Overhead
-------------------------------------------------------
Serialize           8 ns          180 ns        22.5x
Deserialize         2 ns          2 ns          1.0x
Read nested field   2 ns          15 ns         7.5x
Wire size           16 bytes      40 bytes      2.5x
Update one field    1 ns          180 ns        180x
```

Real-world (orderbook with 10 levels):

```
Operation           Raw Struct    FlatBuffers   Overhead
-------------------------------------------------------
Build               50 ns         800 ns        16x
Read best bid       2 ns          20 ns         10x
Wire size           640 bytes     2100 bytes    3.3x
```

## Conclusion

FlatBuffers is not a free lunch. It trades write-side CPU, wire size, and code
ergonomics for schema evolution and read-side validation.

For a matching engine's internal communication, that's a bad trade.

For an external API, it's often the right one.

Know the cost. Choose deliberately.
