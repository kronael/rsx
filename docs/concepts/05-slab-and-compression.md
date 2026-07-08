# Slab and Compression

A naive price-level array for BTC-PERP at a $0.01 tick size
across a $1-$1 000 000 price range would need 100 million
slots. At 64 bytes per level, that is 6.4 GB for one symbol.
RSX keeps the exact touch cheap by spending memory on two fixed
arrays: a configurable order slab and a five-zone CompressionMap.

Together they compress that 100-million-slot address space down to
about 617 000 price-level slots. Each `PriceLevel` is 32 bytes and
the book holds two arrays of them (active plus a staging copy for
recentering), so the level storage is about 40 MB, with 2-5 ns
price lookup and no malloc on the matching hot path.

## The slab

`rsx-book` builds the order slab from `Orderbook::new(config,
capacity, mid_price)`. `Slab::new(capacity)` pre-allocates that
many `OrderSlot`s; the 65 536 constant belongs to the per-cycle
event buffer (`MAX_EVENTS`), not to the order arena.

Production sizing is tens of millions of slots. A 78M-slot arena
is roughly 10 GB of virtual memory because each `OrderSlot` is
128 bytes. Linux demand paging is the lazy allocator here: the
address range is reserved up front, while physical pages are
faulted in only as orders actually rest. Physical memory tracks
live touched slots, not the 10 GB virtual reservation.

The fixed arena buys determinism, not a small cap. Handles are
stable `u32` indices into a flat array, so cancel-by-handle is
O(1). Slots never move, so there is no realloc/copy spike. Alloc
is either a bump of `bump_next` or a pop from the free list, and
free pushes the index back. There is no `malloc`, no `Box`, and
no heap growth inside matching.

The alternative is a chunked slab: allocate new chunks on demand
and make the handle `(chunk, offset)`. That reduces the virtual
reservation but makes every handle two-part and every lookup one
more indirection. The current tradeoff keeps the handle a flat
array index and lets the kernel supply laziness at page granularity.

Tail handling is fixed today. The order slab capacity is the
`Orderbook::new(config, capacity, mid_price)` constructor argument;
when `Slab::alloc` runs past that capacity it asserts `"slab
exhausted"`. The per-cycle event buffer is also fixed:
`Box<[Event; MAX_EVENTS]>` with `MAX_EVENTS = 65_536`; `emit`
asserts before overflow instead of growing. The intended direction is
automatic tail handling for both structures: with enough RAM, a
pathological cascade such as 1 billion events should grow or spill
instead of asserting.

## The CompressionMap

Price levels use five distance-based zones centered on the mid:

```
Zone  Distance from mid  Ticks per slot
  0   0-5%               1
  1   5-15%              10
  2   15-30%             100
  3   30-50%             1 000
  4   50%+               1 (two catch-all slots)
```

Zone 0 is exact tick resolution because the touch lives within
about +/-5% of mid; that is where matching happens and where
price-time priority must not be blurred. Far from mid, exact
ticks are mostly empty address space, so RSX coarsens by 10:1,
100:1, then 1000:1. Beyond 50%, the map collapses to 2 slots:
one bid side and one ask side.

For BTC-PERP at $50 000 with a $0.01 tick:

- Zone 0: +/-5% -> 500 000 ticks -> 500 000 slots
- Zone 1: 5-15% -> 1 000 000 ticks / 10 -> 100 000 slots
- Zone 2: 15-30% -> 1 500 000 ticks / 100 -> 15 000 slots
- Zone 3: 30-50% -> 2 000 000 ticks / 1 000 -> 2 000 slots
- Zone 4: everything else -> 2 slots

Total: about 617 000 slots — down from 100 million. At 32 bytes
per `PriceLevel` across the active and staging arrays, that is
about 40 MB of level storage.

The lookup is fixed work. `CompressionMap::new` pre-computes
four raw-distance thresholds at 5%, 15%, 30%, and 50% of mid.
`price_to_index` does a bisection over those 4 thresholds
(2-3 integer comparisons), then one integer divide by the
zone's raw price per slot and one add for the in-zone offset.
Measured lookup is 2-5 ns and O(1) in book depth.

Orders in coarser zones can share one slot. Each order still
stores its exact price; matching checks the raw price before it
crosses. Insertion order preserves time priority inside the slot,
but the price granularity is deliberately coarser away from the
touch. The tradeoff is honest: exact priority where liquidity
clusters, compressed storage where resolution is wasted.

## Recentering keeps the touch exact

The map is centered on one mid, fixed at construction. The market
moves, so the mid drifts. Left alone, the touch would drift out of
zone 0 into a coarsened zone — where two distinct prices share a slot
and, because "side" in the map means above/below mid rather than
buy/sell, a slot can even hold both order sides. Matching, the
per-side occupancy bitmap, and the best-bid/ask cache all assume one
price and one side per slot; that holds *only* at 1:1. So a stale mid
is a correctness hazard, not just lost precision.

Recentering is the guarantor. `should_recenter(mid)` fires once the mid
drifts past half of zone 0's half-width (a >2.5% move), so it is rare.
Live matching then calls `recenter_now`, which swaps in a map rebuilt
around the new mid and migrates the book **eagerly, in one shot** into
the staging array. The cost is a scan of the old slot array (~617k
slots), skipping empty levels in O(1) and remapping only the occupied
ones — bounded by slot count, not order count, but still a spike far
above a 30 ns match. That spike is the deliberate price of correctness.

The eager choice is not laziness of implementation — `rsx-book` also has
an incremental frontier-walk migration (`migrate_batch`), but live
matching does not use it. The reason is in `recenter_now`'s own contract:
a marketable order's crossing prices can lie *outside* a partially
migrated band, so a half-migrated book would let the matcher miss
unmigrated crossing liquidity and violate price-time priority. Eager
migration removes the partial-book window entirely. A correct lazy scheme
is possible but would have to make the matcher safe against the mixed
old/new map during the window; today the engine trades a rare bounded
spike for that guarantee. The invariant it protects: **the touch always
sits in zone 0, at 1:1 resolution** — what makes the compressed book
correct, not an optimization on top of it.

## Recovery: the book is derived

The slab and the map hold no durable state. On a crash the book is
rebuilt by replaying the ME's WAL from the tip — every accepted order
and fill re-applied in sequence reconstructs the exact resting book. So
the arena size and map shape are for speed, not durability; the durable
record is the WAL. The book is a derived structure, like positions
(see [07-asymmetric-durability](07-asymmetric-durability.md),
[01-casting](01-casting.md)).

## Why pre-allocation wins here

Dynamic allocation would also solve sparse prices. A hash map gives
O(1) average lookup; a tree gives O(log n) order. But their constant
cost is on the wrong side of the 30 ns match budget: hash lookup is
30-50 ns, and a tree is pointer chasing through cache misses. An
array index is a single load.

The matching engine is one pinned thread. Its hot path wants the
active touch levels and live `OrderSlot`s in cache. The slab fixes
order handles; the CompressionMap keeps far-away prices from
turning into dead array space. The cost is up-front virtual address
reservation and bounded capacity chosen at process start.

---

Deeper: [blog/13-15mb-orderbook.md](../../blog/13-15mb-orderbook.md),
[blog/18-100ns-matching.md](../../blog/18-100ns-matching.md),
[specs/2/21-orderbook.md](../../specs/2/21-orderbook.md)
