# Why rsx-book has this shape

An order book holds every resting buy and sell order for one market. Most
resting quotes are cancelled or repriced before they fill, so lookup, cancel,
and reuse matter as much as crossing.

`rsx-book` combines a bounded price map, an occupancy summary, and a
preallocated order arena. Each removes a different variable cost from the
common path.

## Compression map

**Problem.** A tree keyed by exact price performs comparisons and follows
nodes whenever an order locates a level. A flat slot for every possible tick
would waste most of its memory on empty prices.

**Fix.** A five-zone map keeps exact tick slots near the current mid and uses
coarser slots farther away. Price-to-slot mapping is a fixed set of integer
comparisons and arithmetic.

**Cost removed.** Common near-market prices use direct array locations without
a tree walk, while far prices do not require one slot per tick.

Compressed slots may contain several exact prices and both sides. Zone 0
keeps one exact price on one side per slot. A taker that stops inside a
compressed slot fills the best crossing price first, so price priority holds
there too ([notes/compressed-slot-priority.md](notes/compressed-slot-priority.md)).
The design makes no general depth-independence claim.

## Occupancy summary

**Problem.** When the best level empties, scanning the whole level array would
pay for every empty slot between occupied prices.

**Fix.** Each side has one bit per slot that holds at least one order of that
side. Higher bitmap layers summarize non-empty words below them. A next-level
lookup climbs and descends this small summary instead of walking the price
array.

**Cost removed.** Empty price space does not turn into a linear level-array
scan. The work follows the fixed bitmap height and the compression map's
price-ordered subranges.

## Slab arena

**Problem.** Orders are inserted, cancelled, and filled continuously. General
heap allocation adds allocator state, unstable addresses, and scattered
storage to that path.

**Fix.** The caller chooses capacity at construction. Orders occupy fixed-size
slots in one arena that `Slab::new` fills at construction. A free list reuses
cancelled and filled slots. Stable handles allow direct unlink.

**Cost removed.** Matching does not allocate from the system heap. Cancel does
not search an order queue by ID after the matching process resolves the slab
handle.

## Cache layout

**Problem.** Matching needs only a subset of each resting order's fields. A
poor layout pulls unrelated identity and audit data into the same cache work.

**Fix.** `OrderSlot` is 128 bytes aligned to 64. Cache line 0 holds the
fields that crossing, linking and unlinking read. Cache line 1 holds the order
ids, which only the emitted events need.

**Cost removed.** Linking, unlinking and the crossing test read one aligned
cache line of the slot. The events a fill or a cancel emits read the second.

## Fixed-point values

Prices and quantities are raw `i64` tick and lot units. Human display
precision and currency conversion stay at the outer API and risk boundaries.
The book performs no floating-point conversion.

## Through-line

The map bounds price lookup, the bitmap bounds empty-level lookup, and the slab
bounds order storage. These are data-structure properties. Performance claims
remain separate, dated measurements in `reports/`, with p50 and p99 from the
same workload.
