# Slab and Compression

A naive price-level array for BTC-PERP at a $0.01 tick size
across a $1–$1 000 000 price range would need 100 million slots.
At 64 bytes per level, that is 6.4 GB for one symbol. It does
not fit in L3 cache. It barely fits in RAM.

Two structures solve this: a slab arena for orders and a
CompressionMap for price levels. Together they bring a
20-million-level theoretical book to about 15 MB and price
lookup to 2–5 ns.

## The slab

`rsx-book` pre-allocates 65 536 `OrderSlot`s per symbol at
startup. Each slot is `#[repr(C, align(64))]` — one cache line.
The slab is a flat array; an `alloc()` call returns an index
into that array in a few nanoseconds by popping from a free-list
of available indices. `free(handle)` pushes the index back.
There is no `malloc`, no `Box`, no heap allocation on the hot
path at any point during matching.

Slab capacity bounds the maximum number of live resting orders
per symbol: 65 536. This is a deliberate design constraint, not
an accident. Bounded capacity means bounded memory, bounded
iteration, and bounded worst-case time for any operation that
walks active orders.

## The CompressionMap

Instead of one array slot per market tick, the orderbook uses a
five-zone mapping centered on the current mid-price:

```
Zone  Distance from mid  Ticks per slot
  0   0–5%               1  (exact tick resolution)
  1   5–15%              10
  2   15–30%             100
  3   30–50%             1 000
  4   50%+               catch-all (one slot per side)
```

For BTC-PERP at $50 000 with a $0.01 tick:

- Zone 0: ±5% → 500 000 ticks → 500 000 slots
- Zone 1: 5–15% → 1 000 000 ticks ÷ 10 → 100 000 slots
- Zone 2: 15–30% → 1 500 000 ticks ÷ 100 → 15 000 slots
- Zone 3: 30–50% → 2 000 000 ticks ÷ 1 000 → 2 000 slots
- Zone 4: everything else → 2 slots

Total: ~617 000 slots × 24 bytes = ~14.8 MB.

The zone lookup is bisection over four pre-computed thresholds:
2–3 integer comparisons. Measured: 2–5 ns.

Orders in coarser zones share an index slot. Each order still
stores its exact price; matching walks the linked list and
checks actual prices. Time priority within a slot is preserved
by insertion order. Zone 0 — where market makers concentrate —
has 1:1 resolution and no slot sharing.

When mid-price drifts far enough that zone boundaries are
misaligned, the book recenters incrementally. Migration is
interleaved with order processing using two pre-allocated level
arrays and two frontier pointers; there is no stop-the-world
pause.

## Why pre-allocation wins here

Dynamic allocation — a `HashMap` or a `BTreeMap` per price
level — would solve the memory problem. Insertion is O(1)
amortized for a hash map, O(log n) for a tree. But the
constant factors matter at this scale. A hash lookup takes
30–50 ns. A tree node is a pointer-chasing traversal.
An array index is a single load that the prefetcher can
anticipate.

The matching engine processes orders serially on one core. Its
working set — the active levels near mid-price, the slab slots
for resting orders — must stay in L1 or L2 cache. Zone 0 at
BTC prices is 500 000 × 64 bytes = 32 MB, which does not fit
in L1 or L2 but fits in L3 (~40 MB for 10 symbols). An L3
hit costs ~20 ns. A DRAM miss costs ~100 ns. The compression
keeps the hot zone resident and the cold zones invisible to
the matching loop.

The alternative rejected: a `HashMap<i64, Vec<Order>>`. Every
insert is an allocation. Every tick traversal is a hash lookup
plus pointer indirection into a heap-allocated vector.
Benchmarks showed 30–50 ns per level access vs 2–5 ns with the
slab plus CompressionMap. At 5 million operations per second,
that difference is the budget.

---

Deeper: [blog/13-15mb-orderbook.md](../../blog/13-15mb-orderbook.md),
[blog/18-100ns-matching.md](../../blog/18-100ns-matching.md),
[specs/2/21-orderbook.md](../../specs/2/21-orderbook.md)
