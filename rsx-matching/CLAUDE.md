# CLAUDE.md — rsx-matching

Local to `rsx-matching/`. Inherits the repo-root `../CLAUDE.md`. This
file pins the crate's **doc conventions and the invariants that must not
regress** — so a later "cleanup" can't quietly gut what makes the docs
honest. The full topology + rationale lives in the `doc-topology` skill;
exemplars to mirror are rsx-cast, rsx-book, and rsx-risk.

The matching engine is the **authoritative writer of fills** (one process
per symbol): it takes orders from risk, matches against `rsx-book`,
persists to the WAL, and fans events out to risk + marketdata. Most of
the matching *algorithm* lives in `rsx-book`; this crate is the tile
around it (casting I/O, dedup, WAL, fan-out, cancel index, recovery).

## Doc topology (which file answers which question)

Follow the `doc-topology` skill. Table only the docs that actually exist
here:

| File | The one question | Notes |
|---|---|---|
| `README.md` | what / why / how-to-run | one-line pitch, the accept-path number with its caveat, `Running` (env vars), env-var table, invariants→panic-message mapping, gotchas |
| `ARCHITECTURE.md` | how it's built | trust boundary, measured-performance table, module layout, the single pinned loop + main-loop steps, cancel index, WAL-crash policy, event fan-out, dedup, config hot reload, snapshot + replay, FAULTED skip |
| `notes/*.md` | why each specific choice | one file per decision, each Problem→Fix→Cost, indexed by `notes/README.md` with a through-line ("trust the boundary above, own the record below") |

No `compare/` or `facts/` dir here. Dated numbers live in root
`reports/` (e.g. `reports/20260703_matching-benches.md`); the Criterion
harness is `benches/` (shared `benches/harness.rs`, one `Me` fixture).

## Keeper sections — do NOT regress

Load-bearing; a "simplification" that drops one is a bug, not a cleanup:

- **Depth-independence is the headline — with its caveat.** Match latency
  flat from 1 to 100k resting orders (`notes/depth-independent.md`,
  `benches/match_depth_bench.rs`). Every number stays flagged as
  single-box, in-process, closed-loop Criterion microbench (compute
  floor, not wire-to-wire) — never quote ~30 ns as end-to-end latency.
- **FIFO / time-priority is the fairness invariant.** The queue
  discipline is rsx-book's; this crate must keep preserving it through
  the event buffer → WAL → replay so match order == on-disk order ==
  replay order (`notes/fifo-time-priority.md`; `specs/2/6-consistency.md`
  invariants #1, #3, #5). Don't let a rewrite reorder events on the way
  to disk/wire.
- **The trust boundary: ME does NOT re-validate.** Gateway + risk own
  input validation; ME assumes well-formed inputs
  (`notes/trust-boundary.md`, `ARCHITECTURE.md` § "Trust Boundary").
  Docs must keep explaining *why* re-validation here is wrong, not argue
  for adding it — "ME accepts unvalidated input" is a closed finding, not
  an action item.
- **Zero-heap hot path.** No per-order allocation: zero-copy
  `try_recv_with`, fixed event buffer drained in place, cached clocks
  (`notes/zero-heap.md`). Don't reintroduce a per-order `Vec`/`Box`.
- **Authoritative-WAL crash policy.** Fill-path WAL appends `.expect(...)`
  with named-invariant messages; cast sends warn-and-continue
  (`notes/authoritative-wal.md`). Never flatten this to "log and
  continue everywhere" — a silently lost fill is unrecoverable.
- **SEQ-1 one-CRC fan-out.** Each record framed once (single WAL seq +
  CRC), fanned to WAL + both cast streams; every record consuming a seq
  goes to both streams so no stream has a hole (`notes/one-crc-fanout.md`).
  Re-introducing per-stream seq counters brings back the FAULTED storms.
- **Each `notes/` file keeps Problem→Fix→Cost; `notes/README.md` keeps
  its through-line.** These are the shape, not decoration.

## When you touch this crate

- New design decision → a `notes/` file (Problem → Fix → Cost-it-removes,
  plain-English domain term, one ASCII/code sketch, citation) + a row in
  `notes/README.md`; fold the headline into the through-line paragraph.
- New measured number → land it in `reports/YYYYMMDD_*.md`, then quote it
  (with the bench name + the microbench caveat) — never inline a raw
  number with no bench behind it.
- Behavior change on the hot path → check it against the keeper sections
  above before editing docs; if a keeper's claim changes, update the
  claim *and* its caveat together.
- Matching-algorithm changes belong in `rsx-book` (and its `notes/`), not
  here; this crate's notes cover the tile, not the book.
