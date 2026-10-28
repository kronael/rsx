# rsx-book/notes

Why the book's data structures are designed the way they are.

| File | Question answered |
|------|-------------------|
| [align.md](align.md) | Why `#[repr(C, align(64))]` on `OrderSlot` |
| [arena.md](arena.md) | Why a slab allocator instead of malloc on the hot path |
| [hotcold.md](hotcold.md) | Why hot/cold field splitting in OrderSlot |
| [occupancy.md](occupancy.md) | Why a summary bitmap finds the next occupied price |
