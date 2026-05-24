# Hot/cold field splitting

Sources: [Drepper cpumemory.pdf §6](https://www.akkadia.org/drepper/cpumemory.pdf),
[Mike Acton — Data-Oriented Design (CppCon 2014)](https://www.youtube.com/watch?v=rX0ItVEVjHc),
[Agner Fog — Optimizing software in C++](https://www.agner.org/optimize/optimizing_cpp.pdf).

## Concept

Put fields accessed in the hot loop in the first cache line; push rarely-used fields
(audit, debug, metadata) to a later line or a separate struct. A 64-byte line loaded
for `price` shouldn't carry `created_at` and `user_id_string` along with it.

## Pattern

```rust
#[repr(C, align(64))]
struct OrderSlot {
    // cache line 0 — hot: touched on every match
    price: i64, qty: i64, side: u8, tif: u8, next: u32, prev: u32, ...
    // cache line 1 — warm/cold: touched on insert/cancel, not on match
    order_id_hi: u64, order_id_lo: u64, user_id: u32, original_qty: i64, ...
}
```

See `rsx-book/src/slab.rs` for the live layout.

## When to apply

Profile first. Split only when cache miss rate is a measured bottleneck.
Works best for large structs (≥ 64 bytes) iterated in tight loops.
At the extreme: Struct-of-Arrays (SoA) puts each field in its own contiguous
array — maximally SIMD-friendly, used in ECS engines like [Bevy](https://bevyengine.org/).
