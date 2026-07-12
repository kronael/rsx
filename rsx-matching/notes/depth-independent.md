# Depth-independent match

**Domain term.** An *order book* is the live list of resting bids and
asks for one symbol. Its *depth* is how many resting orders it holds —
from a handful to millions. A *match* pairs an incoming (taker) order
against the best resting price on the other side and produces a *fill*.

## Problem

The naive book is a tree or a sorted array keyed by price. Every match
does an `O(log n)` descent (BTree) or an `O(n)` scan to find and update
the best level — so **a fuller book makes every match slower**. That is
exactly backwards for an exchange: success (more resting liquidity) would
punish latency, and the tail grows without bound as the symbol gains
depth. A book that scans its level array to find the next best level pays
that scan on every touch-clearing match (rsx-book measured a 4.47 µs
cliff on a 1k-ask book before the fix — see
`rsx-book/notes/occupancy.md`).

## Fix

rsx-matching does not implement the match. It delegates to
`rsx-book::matching::process_new_order` (`main.rs:614`,
`wire.rs::to_incoming`), whose book gives `O(1)` best-level access via a
price→slot **compression map** and a hierarchical **occupancy bitmap**
(the "why" for both lives in `rsx-book/notes/`, not here). rsx-matching's
job is to *feed* that book and *own the benchmark that proves the
property holds for the tile*:

```
risk order ──▶ to_incoming ──▶ process_new_order(book) ──▶ events
                                 └ O(1) best level, depth-independent ┘
```

`benches/match_depth_bench.rs` holds the match work constant (one qty-1
non-draining partial fill) and varies only resting depth from 1 to
100 000. If depth leaked into match cost, this bench would show it.

## Cost it removes

The depth-dependent tail. Match latency does not grow as the symbol
accumulates resting orders, so one hot symbol cannot slow its own
matching by succeeding.

## The number (and its caveat)

`match_by_depth`, p50 over 50 samples, timed thread pinned to core 2
(`reports/20260703_matching-benches.md`):

| depth | p50 |
|---|---|
| n=1 | 30.4 ns |
| n=1 000 | 29.3 ns |
| n=100 000 | 29.7 ns — **flat** |

**Caveat, non-negotiable:** these are single-box, in-process Criterion
microbenchmarks — no UDP, no WS, no cross-process transport. They are
compute floors, not wire-to-wire latency, and the capture was on a
shared 4-core docker host (indicative, not an isolated-box baseline —
re-run on a quiet box to cite as a baseline). The full GW→ME→GW
round-trip is transport-bound (~4 casting hops), not compute-bound. Do
not quote 30 ns as an end-to-end latency.

## Cite

- Bench: `benches/match_depth_bench.rs`; numbers:
  `reports/20260703_matching-benches.md`.
- Mechanism (owned elsewhere): `rsx-book/notes/occupancy.md`,
  `specs/2/21-orderbook.md` (§ "All operations O(1) on the hot path").
