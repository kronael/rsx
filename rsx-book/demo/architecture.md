# rsx-book — the machine, in one screen

The benchmark GIF shows *how fast*. This shows *why*. An order's whole path
through the engine and the data structures behind the fixed-work depth run.
Full detail: `../ARCHITECTURE.md` and `../notes/`.

```
  order  ──  buy 10 @ 100
    │
    │   price → slot        COMPRESSION MAP    a huge price range folded into
    │   fixed bisection     (sawtooth)         a bounded table of slots — no
    │                                          tree, no search, just arithmetic
    ▼
  best level?  ───────────  OCCUPANCY BITMAP   1 bit / slot + a 3-deep summary
    │   next-best in a few   (hierarchical)     tree; find the next resting
    │   word reads, not                         level with trailing-zeros,
    │   a scan of every slot                    never a linear sweep
    ▼
  match FIFO  ────────────  SLAB ARENA         fixed-size orders in a flat
    │   direct unlink,       (bump + freelist)  array, filled at construction,
    │   time priority                           zero heap on the hot path
    ▼
  emit  ──  Fill · OrderDone   →   EVENT BUFFER  (fixed Vec, drained by the ME loop)

  ───────────────────────────────────────────
  fixed-work observation: no depth-related latency increase
  from 100 K to 10 M resting orders in the 2026-07-27 run.
```

The compression map makes price-to-slot lookup fixed arithmetic, the occupancy
bitmap finds the next occupied slot through summary words, and the slab gives
stable handles for direct unlink and reuse. Fixed-point `i64` values avoid
floating-point conversion. The benchmark holds one partial fill constant while
changing preloaded depth; consuming more makers remains separate work.
The p50, standard error and p99 are in
`../../reports/20260727_book-deep-match-tail.md`.
