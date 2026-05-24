# `#[repr(C, align(64))]`

Sources: [Rustonomicon — repr(C)](https://doc.rust-lang.org/nomicon/repr-rust.html),
[Ulrich Drepper — What every programmer should know about memory §6](https://www.akkadia.org/drepper/cpumemory.pdf),
[Intel — cache line size](https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html).

## repr(C)

Rust's default `repr(Rust)` gives the compiler freedom to reorder fields and change
layout across compiler versions. `repr(C)` fixes field order to declaration order with
C padding rules. Use it when:
- You count bytes (hot/cold split, explicit `_pad` fields)
- Two processes or compilation units share the same struct
- You need layout to survive a compiler upgrade

## align(64)

x86_64 cache lines are 64 bytes. A struct without alignment can straddle two lines —
reading one field pulls in two lines instead of one, halving effective L1 bandwidth.

`align(64)` guarantees the struct starts on a 64-byte boundary. In a slab array
of 128-byte structs (2 cache lines each), every element is perfectly boundary-aligned:
no element ever straddles three lines, and adjacent elements don't share a line
(no false sharing between slots).

## When not to bother

- Structs < 16 bytes: padding overhead exceeds the benefit
- One-off heap allocations: alignment only matters in tight loops or arrays
- Non-hot-path structs: profile before splitting
