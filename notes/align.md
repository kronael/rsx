# Why `#[repr(C, align(64))]`

## `repr(C)`: Predictable Layout

Rust's default layout (`repr(Rust)`) makes **no guarantees** about field ordering or
padding. The compiler is free to reorder fields, insert padding anywhere, and change
the layout between compiler versions.

`repr(C)` gives you:

1. **Fields laid out in declaration order** — what you write is what you get in memory
2. **Deterministic padding** — follows C struct rules (each field aligned to its own alignment)
3. **Stable across compilations** — layout won't change if you recompile with a different Rust version

This matters when:
- **You're counting bytes** — hot/cold cache line splitting requires knowing exactly which
  fields land in which cache line. With `repr(Rust)` the compiler might put your "cold"
  `order_id` right next to your "hot" `price`, defeating the split.
- **Manual padding** — explicit `_pad` fields only work if you control field order.
  `repr(Rust)` would silently rearrange around your padding.
- **Shared memory / IPC** — if two processes or two compilation units read the same struct,
  layout must be identical. `repr(C)` guarantees it.
- **Debug / profiling** — you can reason about cache behavior by reading the struct definition.
  No surprises.

### What You Lose

`repr(Rust)` can sometimes produce a **smaller** struct by reordering fields to minimize
padding. With `repr(C)`, you manage padding yourself. In hot-path structs you're doing
this anyway (explicit `_pad` fields), so nothing is lost.

## `align(64)`: Cache Line Alignment

x86-64 cache lines are 64 bytes. `align(64)` ensures every instance of the struct
starts at a 64-byte boundary.

### Why This Matters

**Without alignment** — a struct can straddle two cache lines:

```
Cache line N:     [....][OrderSlot bytes 0-30 ]
Cache line N+1:   [OrderSlot bytes 31-63][....]
```

Reading `price` (bytes 0-7) and `next` (bytes 28-31) now touches **two** cache lines
instead of one. That's two L1 loads (~1ns each) instead of one, and it evicts an
extra line from L1 (32KB, only 512 lines).

**With `align(64)`** — the struct starts at a cache line boundary:

```
Cache line N:     [OrderSlot hot fields: price, qty, side, next, prev, tick_index]
Cache line N+1:   [OrderSlot cold fields: order_id, user_id, original_qty, ...]
```

All hot fields in one cache line. Matching never touches line N+1.

### Slab Array Benefit

When `OrderSlot` is 128 bytes (2 cache lines) and aligned to 64:

```
orders[0]: cache lines 0-1
orders[1]: cache lines 2-3
orders[2]: cache lines 4-5
...
```

Each order starts at a cache line boundary. No order straddles three cache lines.
Sequential slab iteration gets perfect prefetch behavior.

### False Sharing Prevention

If two threads could touch adjacent slots (not the case in SPSC, but relevant for
the general crate), alignment prevents two slots from sharing a cache line. Without
it, writing `orders[0].remaining_qty` could invalidate the cache line holding
`orders[1].price` on another core.

## Combined: `repr(C, align(64))`

| Attribute   | What it controls          | Why                                        |
|-------------|---------------------------|--------------------------------------------|
| `repr(C)`   | Field order and padding   | Deterministic layout, hot/cold split works |
| `align(64)` | Struct start address      | Cache line boundary, no straddling         |

Together they give you full control: you decide which fields go in which cache line,
and the hardware gets clean cache line access patterns.

## When NOT to Bother

- Small structs (<16 bytes) used in arrays — alignment padding wastes more than it saves
- Structs not on the hot path — profile first
- Heap-allocated one-off structs — cache line alignment only matters in tight loops or arrays
