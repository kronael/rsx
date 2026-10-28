# `#[repr(C, align(64))]` on `OrderSlot`

Sources: [Rustonomicon — repr(C)](https://doc.rust-lang.org/nomicon/repr-rust.html),
[Ulrich Drepper — What every programmer should know about memory §6](https://www.akkadia.org/drepper/cpumemory.pdf),
[Intel — cache line size](https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html).

## Problem

An x86_64 cache line is 64 bytes. A 128-byte slot that starts at an arbitrary
address spans three lines, so reading one slot loads one line more than it
needs. With the default `repr(Rust)` the compiler may also reorder fields, so
the split of fields between the two lines is not under the author's control.

## Fix

`OrderSlot` is `#[repr(C, align(64))]` and 128 bytes. `repr(C)` keeps the
fields in declaration order, with explicit `_pad` fields. `align(64)` starts
every slot on a line boundary, so a slot is exactly two lines and two
neighbouring slots never share a line. `size_of` and `align_of` are
compile-time asserted in `order.rs` and checked in
`tests/struct_layout_test.rs`.

`PriceLevel` is 32 bytes and not aligned to 64.

## Cost

The padding fields use bytes that carry no data. A struct under 16 bytes, or
one outside a tight loop, gains nothing from this and does not use it.
