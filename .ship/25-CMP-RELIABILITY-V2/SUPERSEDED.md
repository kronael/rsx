# Superseded

Superseded by `.ship/26-CMP-RELIABILITY-V4/SPEC.md` (2026-05-24).

Codex critique on this v2 spec (heap-of-NAKs + RESET tier) drove the
v4 simplification. Key issues codex flagged:

- RESET is fake novelty (it's just a large NAK; sender already clamps).
- `BinaryHeap` is the wrong structure for the actual mutations (merge,
  trim, retire).
- "One NAK per turn" is caller-scheduling-dependent, not a protocol
  guarantee.
- Optimizing "number of gaps" is wrong — on FIFO only the oldest
  prefix matters.

v4 drops the heap, drops the RESET tier, uses a ring-buffer reorder,
NAKs only the oldest run. ~50% smaller LOC, same correctness, better
performance.

This SPEC.md is kept for historical reference. Implementation should
read v4 only.
