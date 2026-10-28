# Slab arena for orders

Sources: [slab crate](https://docs.rs/slab),
[bumpalo crate](https://docs.rs/bumpalo),
[Drepper cpumemory.pdf §6.4](https://www.akkadia.org/drepper/cpumemory.pdf).

## Problem

Orders are inserted, cancelled and filled all the time, each with its own
lifetime. A heap allocation per order adds allocator state to the match and
cancel path, and it scatters the orders across memory. A bump arena cannot free
one order, because it frees everything at once.

## Fix

`Slab<T>` in `rsx-book/src/slab.rs` holds fixed-size slots in one `Vec`, a free
list and a bump cursor. `alloc` pops the free list or bumps the cursor, `free`
pushes the slot onto the free list, and both are O(1). The free list chains
through each slot's `next` field. The handle is the slot index, so a cancel
unlinks the order directly. The book uses it as `Slab<OrderSlot>`.

The capacity is a constructor argument. `Slab::new` writes a default value into
every slot, so the whole capacity is resident from construction. `alloc` panics
when the slab is exhausted.

## Cost

- The caller sizes the slab for the largest book it will hold. 78 million
  slots take about 10 GB, and an unused slot costs the same memory as a used
  one.
- A handle names a slot, not an order. After a `free` the next `alloc` reuses
  the index, so a stale handle names another order. `cancel_order_checked`
  compares the user id and order id before it cancels.
- `free` checks double frees only under `debug_assert`, because the check walks
  the free list.
