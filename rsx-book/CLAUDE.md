# CLAUDE.md — rsx-book

Local to `rsx-book/`. Inherits the repo-root `../CLAUDE.md`. This file
pins the crate's **doc conventions and the invariants that must not
regress** — so a later "cleanup" can't quietly gut what makes the docs
honest. The full topology + rationale lives in the `doc-topology` skill;
rsx-book is one of its two exemplars (with rsx-cast).

## Doc topology (which file answers which question)

Follow the `doc-topology` skill. Table only the docs that actually exist
here:

| File | The one question | Notes |
|---|---|---|
| `README.md` | what / why / how-to-start | elevator pitch line 1-8, glossary before jargon (symbol/tick/depth/compression map/occupancy bitmap), quick start, "how to read this" pointers into `notes/`+`compare/` |
| `ARCHITECTURE.md` | how it's built | module table (`src/`), the data structures and how they compose, invariants; defers "why" to `notes/` and the spec `specs/2/21-orderbook.md` |
| `WHY.md` | why *this* shape, in one page | the consolidated Problem→Fix→Cost narrative with a closing through-line paragraph; the per-decision detail is split into `notes/` |
| `notes/*.md` | why each specific choice | `align.md` (repr(C, align(64))), `arena.md` (slab vs malloc), `hotcold.md` (hot/cold field split), `occupancy.md` (3-tier bitmap vs linear scan). One file per decision, each Problem→Fix→Cost, indexed by `notes/README.md` |
| `compare/*.md` | vs named alternatives | benched: `naive-btree.md`, `hftbacktest.md`, `lob.md`; cited-only: `cross-language-cited.md`, `orderbook-inv2004.md`, `orderbook-rs.md`. Index + capability/fairness tables in `compare/README.md` |

No `facts/` dir here. Dated numbers live in `compare/README.md` (benched,
copy-pasted from the Criterion run) and in root `reports/` (e.g.
`reports/20260704_book-bench.md`); the harness is `benches/compare_all_bench.rs`.

## Keeper sections — do NOT regress

Load-bearing; a "simplification" that drops one is a bug, not a cleanup:

- **The honest trade in `compare/README.md`.** rsx-book loses pure level
  churn to a bare BTreeMap by a small constant, and the doc says so
  plainly (that constant buys depth-invariant matching, O(1) cancel,
  flat level ops). Never quietly delete the admitted loss to make the
  numbers look cleaner.
- **Benched vs cited, never blurred.** Same-box/same-harness numbers and
  someone-else's-published numbers stay separate, each flagged. Keep the
  "What this does NOT show" and "Fairness bar per row" sections.
- **The README glossary before jargon.** Restate each domain term in
  plain English before using it.
- **Depth-invariance is the claim.** Match latency flat from 100 to 10M
  resting orders — the whole reason the crate exists. Docs must keep the
  caveat that all numbers are single-core, in-process, closed-loop
  Criterion microbenches.
- **Each `notes/` file keeps Problem→Fix→Cost; `WHY.md` keeps its
  through-line.** These are the shape, not decoration.
- **i64 fixed-point framing.** No floats, exact prices — stated in
  `WHY.md` and README.

## When you touch this crate

- New design decision → a `notes/` file (Problem → Fix → Cost) + a row in
  `notes/README.md`; fold the headline into `WHY.md`'s through-line.
- New contender or re-run → update `compare/README.md`'s run header,
  capability table, and fairness table; keep benched/cited separate.
- New measured number → land it in `reports/YYYYMMDD_*.md`, then quote it
  (with bench name + caveat) — never inline a raw number with no bench.
