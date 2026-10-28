# Hot and cold fields in `OrderSlot`

Sources: [Drepper cpumemory.pdf §6](https://www.akkadia.org/drepper/cpumemory.pdf),
[Mike Acton — Data-Oriented Design (CppCon 2014)](https://www.youtube.com/watch?v=rX0ItVEVjHc),
[Agner Fog — Optimizing software in C++](https://www.agner.org/optimize/optimizing_cpp.pdf).

## Problem

A match or a cancel reads a few fields of each order it touches. If those
fields share a cache line with fields only an event needs, the line carries
bytes the operation never reads.

## Fix

`OrderSlot` (`rsx-book/src/order.rs`) is 128 bytes, two cache lines:

| line | bytes | fields |
|---|---|---|
| 0 | 0..64 | `price`, `remaining_qty`, `side`, `flags`, `tif`, `next`, `prev`, `tick_index`, `user_id`, `sequence`, `original_qty`, `timestamp_ns` |
| 1 | 64..128 | `order_id_hi`, `order_id_lo`, padding |

Linking, unlinking and the crossing test read line 0 only. The two order ids
sit alone in line 1, and only the events carry them.

## Cost

Line 1 holds two `u64` values and 48 bytes of padding. An operation that emits an
event for a resting order reads line 1, so a fill or a cancel reads both lines. Profile before
splitting another struct.
