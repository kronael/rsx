# CLAUDE.md — rsx-book

Local to `rsx-book/`; the root `../CLAUDE.md` applies. The order book and
matching core as a library: compression map, slab arena, occupancy bitmap,
the cross/rest/cancel algorithm. Finished, and presented as finished
(alongside `rsx-cast`). Specs: `specs/2/21-orderbook.md`; tests
`34-testing-book.md`.

## Rules

- The book knows nothing about users, the WAL, decimals or display: it takes
  tick- and lot-aligned i64 and returns i64. Display scaling belongs at the
  API boundary, user-keyed state in matching, the WAL seq to the caller. A
  snapshot captures pointers; the caller serializes and releases it.
- API shape: the incoming order is the wire struct, results are structs
  (`current_bbo` returns a BBO, not a tuple), flags are enums. NEVER
  `#[inline]` nudges.
- Cache locality first: fast IOC at the bests and O(1) cancel by handle;
  a deep sweep may cost more. Bids and asks are compressed separately,
  quantized by distance from mid; the book owns recentering, and recentering
  must be lazy and incremental, never a one-shot migration on an order's
  critical path.
- Zone lookup is bisection over four constant thresholds (2–3 comparisons);
  NEVER a logarithm or a linear scan on the order path.
- Capacity means millions of resting orders per symbol. A fixed slab cap is
  a limit to record in `BUGS.md`, not a feature to present. Benches seed
  100k, 1M and 10M resting orders with distributed prices and sizes; 1B per
  side is the next measurement.
- The per-cycle event buffer panics on overflow (the ME never drops events);
  this is kept simple on purpose and the docs say so plainly.
- A change to a scan or BBO API runs the whole workspace's tests;
  `rsx-marketdata` iterates a live book.
- No demo or reproduction recipe in `README.md`: `demo/` holds the demo and
  the playground `/demos` page shows it.

## Docs (which file answers which question)

| File | Question | Notes |
|---|---|---|
| `README.md` | what / why / how to start | elevator pitch, glossary before jargon (symbol, tick, depth, compression map, occupancy bitmap), quick start, "how to read this" pointers into `notes/` and `compare/` |
| `ARCHITECTURE.md` | how it is built | module table (`src/`), the data structures and how they compose, invariants; defers "why" to `notes/` and the spec |
| `WHY.md` | why this shape, in one page | the consolidated Problem → Fix → Cost narrative with its closing through-line |
| `notes/*.md` | why each choice | `align.md`, `arena.md`, `hotcold.md`, `occupancy.md`; one file per decision, indexed by `notes/README.md` |
| `compare/*.md` | vs named alternatives | benched: `naive-btree.md`, `hftbacktest.md`, `lob.md`; cited only: `cross-language-cited.md`, `orderbook-inv2004.md`, `orderbook-rs.md`; tables in `compare/README.md` |

No `facts/` here. Dated numbers live in `compare/README.md` (benched, from
the Criterion run) and in root `reports/` (e.g.
`reports/20260704_book-bench.md`); the harness is
`benches/compare_all_bench.rs`.

## Keeper sections — do not regress

- **The honest trade in `compare/README.md`.** A bare BTreeMap wins pure
  level churn by a small constant and the doc says so; the bookkeeping buys
  direct cancel-by-handle and bounded next-level lookup. Never delete the
  admitted loss to make the numbers look cleaner.
- **Benched vs cited, never blurred.** Same-box numbers and published
  numbers stay separate and flagged; keep "What this does NOT show" and
  "Fairness bar per row".
- **The README glossary before jargon.**
- **Design separate from evidence.** O(1) lookup and cancel-by-handle are the
  design claims; the 100K–10M fixed-work sweep is dated evidence, never a
  production-latency claim or a "depth-invariant" slogan. Methodology and
  caveats stay in the dated report.
- **Latency tables carry p50 and p99 from the same run**; a standard error
  may accompany p50, never replace p99.
- **Each `notes/` file keeps Problem → Fix → Cost; `WHY.md` keeps its
  through-line.**
- **i64 fixed-point framing**: no floats, exact prices.

New design decision → a `notes/` file + a row in `notes/README.md`, headline
folded into `WHY.md`. New contender or re-run → `compare/README.md` run
header, capability and fairness tables. New number → `reports/YYYYMMDD_*.md`,
quoted with bench name and caveat.
