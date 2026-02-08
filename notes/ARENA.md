# Arena Allocators

## Concept

An arena allocator (also called a bump allocator or region-based allocator) reserves a large block of memory up front and satisfies allocations by bumping a pointer forward. Individual objects are **never freed** — the entire arena is freed at once when it is dropped.

```
               ┌──────────────────────────────────┐
Arena block:   │ A  │ B  │ C  │    free space     │
               └──────────────────────────────────┘
                                 ↑ bump pointer
```

## Why Arenas Are Fast

| Operation | System allocator (`malloc`) | Arena (bump) |
|---|---|---|
| Allocate | Search free list, split block | Bump pointer forward |
| Deallocate | Return to free list, coalesce | No-op (bulk free on drop) |
| Typical cost | ~20–80 ns | ~1–5 ns |

Arenas also produce **excellent cache locality** because allocations are contiguous in memory.

## Rust Implementation

### Using `bumpalo` (most popular)

```rust
use bumpalo::Bump;

let arena = Bump::new();

// Allocate individual values
let x: &mut i32 = arena.alloc(42);
let s: &mut str = arena.alloc_str("hello");

// Allocate collections inside the arena (nightly allocator_api)
let mut v: bumpalo::collections::Vec<i32> = bumpalo::vec![in &arena; 1, 2, 3];

// Everything freed when `arena` is dropped
```

### Using `typed-arena` (single-type arena)

```rust
use typed_arena::Arena;

let arena = Arena::new();
let node_a: &mut Node = arena.alloc(Node::new("a"));
let node_b: &mut Node = arena.alloc(Node::new("b"));
// All nodes freed when `arena` is dropped
```

### Manual bump allocator (simplified)

```rust
struct BumpArena {
    buf: Vec<u8>,
    offset: usize,
}

impl BumpArena {
    fn new(capacity: usize) -> Self {
        Self { buf: vec![0u8; capacity], offset: 0 }
    }

    fn alloc<T>(&mut self, val: T) -> &mut T {
        let align = std::mem::align_of::<T>();
        let size = std::mem::size_of::<T>();

        // Align the offset
        self.offset = (self.offset + align - 1) & !(align - 1);
        assert!(self.offset + size <= self.buf.len(), "arena out of memory");

        let ptr = unsafe { self.buf.as_mut_ptr().add(self.offset) as *mut T };
        self.offset += size;
        unsafe {
            ptr.write(val);
            &mut *ptr
        }
    }

    fn reset(&mut self) {
        self.offset = 0; // "free" everything at once
    }
}
```

## Use Cases

| Domain | Why arenas work well |
|---|---|
| **Compilers** | AST nodes share a compilation-unit lifetime. Allocate all nodes in an arena, drop them together after codegen. |
| **Parsers** | Parse tree nodes live for the duration of a parse. |
| **ECS / Game engines** | Per-frame scratch allocations that reset every tick. |
| **Request-scoped servers** | Allocate into a per-request arena, free everything when the response is sent. |
| **Graph structures** | Arenas eliminate self-referential borrow issues since references share the arena's lifetime. |

## Arenas and the Borrow Checker

Arenas are a common pattern for building **self-referential** and **graph** structures in safe Rust:

```rust
use typed_arena::Arena;

struct Node<'a> {
    value: i32,
    children: Vec<&'a Node<'a>>,
}

let arena = Arena::new();
let root = arena.alloc(Node { value: 1, children: vec![] });
let child = arena.alloc(Node { value: 2, children: vec![] });
root.children.push(child); // Valid — both share lifetime 'a tied to the arena
```

Without an arena, this would require `Rc`, `unsafe`, or index-based graphs.

## Other Allocators in Rust

| Allocator | Description | Crate |
|---|---|---|
| **System (Global)** | Default. Delegates to OS `malloc`/`free`. | `std::alloc::System` |
| **jemalloc** | Multithreaded general-purpose allocator. | `tikv-jemallocator` |
| **mimalloc** | Microsoft's compact, fast allocator. | `mimalloc` |
| **Bump / Arena** | Pointer-bump, bulk-free only. | `bumpalo`, `typed-arena` |
| **Pool / Slab** | Fixed-size slot pre-allocation. Fast alloc/free for uniform objects. | `slab`, `slotmap` |
| **Stack** | Allocates from a stack buffer, LIFO free order. | — |
| **Buddy** | Power-of-two block splitting. Common in OS kernels. | — |
| **wee_alloc** | Tiny code-size allocator for WebAssembly. | `wee_alloc` |

## Allocator API (Nightly)

Rust's unstable `Allocator` trait lets you parameterize standard collections:

```rust
#![feature(allocator_api)]
use std::vec::Vec;
use bumpalo::Bump;

let arena = Bump::new();
let mut v: Vec<i32, &Bump> = Vec::new_in(&arena);
v.push(1);
v.push(2);
```

## Trade-offs

- **Pros:** Near-zero allocation cost, great locality, simplifies lifetimes for graphs/trees.
- **Cons:** No individual deallocation (memory stays live until arena drop), can waste memory if some objects die early.
- **Rule of thumb:** Use arenas when many objects share a well-defined lifetime scope.
