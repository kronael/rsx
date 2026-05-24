# Arena / slab allocators

Sources: [bumpalo crate](https://docs.rs/bumpalo), [slab crate](https://docs.rs/slab),
[Dmitry Vyukov — Lock-free data structures](https://www.1024cores.net/home/lock-free-algorithms/queues),
[Drepper cpumemory.pdf §6.4](https://www.akkadia.org/drepper/cpumemory.pdf).

## Why

`malloc` takes 20–80 ns and may hold a global lock under contention. A bump or slab
allocator pre-allocates one block at startup; alloc is a pointer increment (~1–5 ns).
Contiguous layout also improves cache prefetch vs scattered heap pointers.

## Bump allocator (arena)

Allocate by bumping a pointer forward; free everything by resetting to zero.
Good for: AST nodes, per-request scratch memory, parse trees — anything with a
single shared lifetime.

Rust: [`bumpalo`](https://docs.rs/bumpalo) — `Bump::alloc(val)` is a pointer bump;
compatible with `allocator_api` for standard collections.

## Slab allocator

Pre-allocated fixed-size slots with a free list. O(1) alloc and O(1) free for
uniform objects; slots are reused. Handles objects with varied lifetimes unlike bump.

Rust: [`slab`](https://docs.rs/slab) gives `slab.insert(val) -> usize` and
`slab.remove(key)`. RSX orderbook uses a hand-rolled 128-byte slab aligned to 64
bytes (`rsx-book/src/slab.rs`) so order slots are cache-line aligned.

## Trade-offs

| | Bump | Slab | malloc |
|---|---|---|---|
| Alloc cost | ~1 ns | ~5 ns | 20–80 ns |
| Free cost | bulk only | O(1) | O(1) |
| Per-object free | no | yes | yes |
| Fragmentation | none | slot-granular | yes |
