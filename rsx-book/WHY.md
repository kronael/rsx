# Why rsx-book is built this way

`ARCHITECTURE.md` says *what* the orderbook is. This says *why* each piece
is shaped the way it is — the problem that forced the choice, and what it
buys. No structure here is clever for its own sake; each one removes a cost
that a naive orderbook pays on every single order.

The one goal behind all of it: **matching an order should cost the same
whether the book holds a hundred orders or ten million.** A textbook book
gets slower as it fills. This one doesn't. Here's how, and why.

## The price map — arithmetic instead of a search

**Problem.** An order arrives at some price. Where does it go? The obvious
answer is a tree keyed by price (`BTreeMap<price, level>`), but a tree walk
is O(log n): the more prices are live, the more hops to find the right one,
on the hottest path in the system.

**Fix.** Prices don't need a tree — they live on a fixed grid (the tick
size). So we pre-quantise the whole tradeable range into a fixed array of
~120 000 slots and fold the huge raw price range into it with plain
arithmetic (a "sawtooth" of ± bands around the mid price, dense where it
matters). Price → slot is now a couple of subtractions and a compare —
**O(1), no search, no pointers to chase.**

**Cost it removes.** Every insert, cancel, and match starts by locating a
price. Making that arithmetic instead of a tree walk takes the depth term
out of the most frequent operation there is.

## The occupancy bitmap — finding the next level without looking

**Problem.** After a trade clears the best price level, matching needs the
*next* resting level. With slots in a flat array, the naive way is to scan
forward until you hit a non-empty one. When the nearby book is thin, that
scan walks thousands of empty slots — we measured it costing **32–224 µs**
on the level-clearing path. A cliff, right on the hot path.

**Fix.** Keep one bit per slot: 1 = something resting here, 0 = empty. Pack
them 64 to a word, then put a second layer of bits summarising which words
are non-zero, and a third summarising the second. To find the next filled
level you read a word, and the CPU's count-trailing-zeros instruction hands
you the answer in one step; if the word is empty you climb one level and
repeat. Three levels cover all 120 000 slots, so **next-best-level is ~3
word reads regardless of how far the next order sits** — O(depth=3), not
O(price-range).

**Why not a tree here either?** A `BTreeMap` would also answer "next key,"
but it re-earns the search every time and scatters nodes across the heap.
The domain is already a dense pre-quantised grid — the slot index *is* the
key — so a bitmap over that grid is both smaller and faster than a tree
laid on top of it. (A single "pointer to the next filled slot" can't help:
inserts arrive in any order and matching seeks in both directions, so you'd
still need the search the bitmap gives you for free.) The measured result:
that 32–224 µs cliff became **~145 ns, flat.**

## The slab — allocation that isn't allocation

**Problem.** Orders and levels are created and destroyed constantly.
Calling `malloc`/`free` for each one costs 20–80 ns and can grab a global
lock under load — unacceptable when the whole match budget is a few hundred
nanoseconds, and a hard "no" for the zero-heap-on-hot-path rule.

**Fix.** Grab one big block at startup and hand out fixed-size slots from
it. Allocating is bumping a counter or popping a free-list head (~1–5 ns);
freeing is pushing it back (O(1)). Slots get reused, so there's no
fragmentation and no trip to the system allocator ever happens mid-trade.
As a bonus the slots sit contiguously, so the CPU's prefetcher works with
us instead of chasing scattered heap pointers.

## Cache layout — not straddling the line

**Problem.** x86 memory moves in 64-byte cache lines. A struct that isn't
line-aligned can sit across two lines, so reading one field drags in two
lines — half the effective L1 bandwidth — and neighbouring order slots can
share a line and fight over it (false sharing).

**Fix.** Every hot struct is `#[repr(C, align(64))]`: fixed C field order
(so we can count bytes and it survives compiler upgrades) and a guaranteed
64-byte start. In a slab of 128-byte slots each order is exactly two lines,
perfectly aligned, never straddling, never sharing a line with its
neighbour.

**Hot/cold split.** Within a slot, the fields touched on *every* match —
price, quantity, side, the linked-list pointers — go in the first cache
line. The fields only touched on insert or cancel — order id, user id,
original quantity, timestamps — go in the second. Matching a resting order
then pulls exactly one line, not two, and never pays to load audit data it
won't read.

## The through-line

Every choice above replaces a variable cost with a fixed one: a tree walk
becomes arithmetic, a linear scan becomes three word reads, a `malloc`
becomes a pointer bump, a two-line read becomes one. That's the whole
reason the numbers stay flat as the book grows — and the benchmark
(`reports/` + the live demo) is just that, measured. Everything is `i64`
fixed-point throughout: no floats, no rounding, exact prices.
