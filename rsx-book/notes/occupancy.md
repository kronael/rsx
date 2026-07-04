# Next-best-level lookup: occupancy bitmap

Why this exists, what it guarantees, and the cache story behind each
guarantee. Design rationale, not "how it is" — see `book.rs` /
`occupancy.rs` for the code.

## The hole it closes

`match_at_level` and `cancel_order` need the *next* best level whenever a
match clears the touch or the touch order is cancelled. The old
`scan_next_bid` / `scan_next_ask` did a linear pass over **all**
`active_levels` — sized to `compression.total_slots()` (~120k for a
realistic MID/tick). Any marketable order that emptied the best level, or
any cancel at the touch, paid ~O(120k) — a full sweep of a 2.4 MB level
array (~4 µs), 200x over the <500 ns match budget. The published "65 ns,
O(1) in depth" match numbers only dodged it because their harness
replenishes the touch so it never fully clears.

Measured cliff (this box, `cargo bench -p rsx-book`):

| bench                  | before   | after   |
|------------------------|----------|---------|
| `match_ioc_vs_1k_asks` | 4.47 µs  | 71 ns   |
| `match_by_type/gtc_full_cross` | ~95 µs | 75 ns |
| `match_by_type/ioc_full`       | ~103 µs | 149 ns |
| `match_by_type/sweep_10_levels`| ~978 µs | 1.37 µs |
| `match_depth/100000` (no clear)| 63 ns  | 62 ns   |

`match_depth` is unchanged: it never clears the touch, so it never hit
the scan, before or after. The FOK `available_liquidity` full-book scan
(`matching.rs`) is a **separate** O(N) pass and is out of scope here.

## Structure

Two per-side hierarchical bitmaps over the compression slots (`bid_occ`,
`ask_occ`): bit set = "that level holds ≥1 resting order of that side".
Level 0 is one bit per slot; each higher level is one bit per word of the
level below (set iff that word is non-zero). ~120k slots => 3 levels
(1929 + 31 + 1 words). Each level is one contiguous `Vec<u64>` — no
pointer chasing, no heap-scattered nodes.

Keyed by **order side**, not by index region: a sell can rest below mid
(a bid-region index) when the book is thin, so occupancy must follow the
resting order's side. The first insert into an empty level sets the bit
(that order becomes the level head, so the bit tracks head-side, matching
the old scan's `head.side` test exactly); the bit clears when the level
empties.

### The sawtooth

The compression map is a **sawtooth**: tick index is not globally
price-monotonic (each zone is a symmetric ± band around mid, laid out at
ascending index, so crossing a zone boundary resets the price). A single
find-next-set over the whole bitmap would therefore return the wrong
level across a zone boundary. `price_asc` precomputes the ≤10 zone-half
index sub-ranges **ordered by price band** (within each, ascending index
== ascending price). `scan_next_ask` walks them low→high price taking the
first set bit; `scan_next_bid` walks high→low taking the last. Recomputed
only on construction / recenter, never on the hot path.

## Guarantees (complexity + cache)

`d` = bitmap depth (3 for a ~120k-slot book). `W` = machine word.

| operation | cost | cache behavior |
|---|---|---|
| **best-IOC match on the touch** (near-BBO, one level cleared) | O(d) find + O(d) clears | the winning `price_asc` range is zone 0; find touches ~d summary words (a few hot cache lines), and the next level's `PriceLevel` is adjacent in `active_levels` — already in cache. Replaces a 2.4 MB (~37k cache-line) scan with ~3 lines. |
| **cancel of a non-best order** (deep in the book) | O(1) unlink + O(d) bit clear | direct slab handle (client oid → `order_index`/`user_map` → handle → unlink). No scan: the best-update path only runs when the cancelled order was the last at the touch. A deep cancel touches only its own level + ≤d bitmap words. |
| **cancel of the touch order** | O(1) unlink + next-best find | same next-best find as the IOC case; cache-local (zone-0 summary words + adjacent level). |
| **best-update when a level empties** | O(d) find | bounded per-zone `find_first_in` / `find_last_in`; in the common case the first non-empty range is zone 0, so it returns after a handful of summary-word reads. |
| **deep full-book sweep** (market/IOC clearing K levels) | O(K · d) | ALLOWED to be slower — each of the K next-best finds is itself cache-local (adjacent occupied levels, hot summary words). Linear in levels swept, not in `total_slots`. |
| `set` / `clear` (occupancy maintenance) | O(d) | walks UP only while a word flips empty↔non-empty; a couple of word writes, all in the summary cache lines. |

The design priority (founder's framing): the **near-BBO IOC path is the
one that must be fast** and is optimized hardest — a handful of cache
lines. The rare deep sweep is O(K) and left simple.

## Invariant: bitmap ⟺ `active_levels[t].order_count`

A stale bit is a phantom or skipped level = a matching bug. The bitmap is
maintained at *every* site that moves a level's `order_count` across the
0 boundary:

- `insert_resting` — 0→1 sets the side's bit.
- `unlink_order` — →0 clears it (covers cancel **and** maker-fill during
  a match; both route through `unlink_order`).
- `migrate_single_level` — 0→1 in the new array sets it.
- `trigger_recenter` — fresh empty array ⇒ both bitmaps reset to the new
  size, `price_asc` recomputed.
- `snapshot::load` — `rebuild_occupancy()` after the level array is
  replaced wholesale.

`scan_reference_test.rs` cross-checks the bitmap path against a
brute-force max-BUY-head / min-SELL-head pass over 6000 random ops
(multi-zone, crossed-region, cancels) and across a full recenter+migrate,
asserting identical `best_bid_tick` / `best_ask_tick` and an uncrossed
book at every step.
