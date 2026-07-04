# rsx-book — the machine, in one screen

The benchmark GIF shows *how fast*. This shows *why*. An order's whole path
through the engine, and the one data structure that removes the depth term at
each step. (Full detail: `../ARCHITECTURE.md` + `../notes/`.)

```
  order  ──  buy 10 @ 100
    │
    │   price → slot        COMPRESSION MAP    a huge price range folded into
    │   O(1) bisection      (sawtooth)         ~120k fixed slots — no tree,
    │                                          no search, just arithmetic
    ▼
  best level?  ───────────  OCCUPANCY BITMAP   1 bit / slot + a 3-deep summary
    │   next-best in ~3      (hierarchical)     tree; find the next resting
    │   word reads, not                         level with trailing-zeros,
    │   an O(120k) scan                         never a linear sweep
    ▼
  match FIFO  ────────────  SLAB ARENA         fixed-size orders/levels in a
    │   O(1) unlink,         (bump + freelist)  flat array; 556 ps to alloc,
    │   time priority                           zero heap on the hot path
    ▼
  emit  ──  Fill · OrderDone   →   EVENT RING   (drained by the ME tile)

  ───────────────────────────────────────────
  net: ~60 ns to match — and it stays ~60 ns
  whether 100 thousand or 10 million orders
  are already resting.  O(1) in book depth.
```

**Why the depth term vanishes.** The naive book slows as it fills: a tree walk
is O(log n), a flat-array scan for the next level is O(price-range). Here every
step above is O(1) or O(depth=3): the compression map makes price→slot
arithmetic, the occupancy bitmap makes next-best a couple of word reads, the
slab makes alloc/unlink pointer-free. Fixed-point i64 throughout — no floats,
no rounding. That is the whole trick, and the benchmark is just it, measured.
