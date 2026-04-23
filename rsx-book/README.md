# rsx-book

Shared orderbook library with slab-allocated orders,
compressed price levels, and price-time priority matching.

## What It Provides

- `Orderbook` -- full orderbook with insert/cancel/match
- `process_new_order()` -- matching algorithm (GTC, IOC,
  FOK, post-only, reduce-only)
- `Slab<T>` -- generic arena allocator (zero heap on hot path)
- `CompressionMap` -- 5-zone price-to-index mapping
- `Event` -- fill, insert, cancel, done, failed, BBO events
- Binary snapshot save/load

## Public API

```rust
use rsx_book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_book::Event;

let mut book = Orderbook::new(config, 1024, mid_price);
let mut order = IncomingOrder { /* ... */ };
process_new_order(&mut book, &mut order);
for event in book.events() {
    // handle Fill, OrderInserted, OrderDone, BBO...
}
```

Used by: rsx-matching (ME), rsx-marketdata (shadow book).

## Building

```
cargo check -p rsx-book
cargo build -p rsx-book
```

## Testing

```
cargo test -p rsx-book
```

15 test files in `tests/`: book, compression, config update,
level, matching, migration, modify, order, post-only, reduce,
slab, snapshot, and more. All tests non-flaky with migration
completion assertions and zone boundary edge cases.
See `specs/1/34-testing-book.md`.

## Dependencies

- `rsx-types` -- Price, Qty, Side, SymbolConfig

## Gotchas

- The slab pre-allocates ~10 GB for 78M order slots. This is
  virtual memory; physical pages are faulted in on use.
- `CompressionMap` smooshes ticks in outer zones (1-4).
  Orders in smooshed zones store exact prices and are
  scanned during matching. Near mid (zone 0) is 1:1.
- During large price moves the book enters `Migrating` state
  with lazy level migration. Two arrays are live simultaneously.
- Event buffer is `[Event; 10_000]`, reset per matching cycle.
  If a single order generates >10K events, it will panic.

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) -- slab internals,
  compression zones, matching algorithm, memory layout
- `specs/1/21-orderbook.md`
