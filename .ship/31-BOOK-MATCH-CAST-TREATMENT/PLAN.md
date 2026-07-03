# rsx-book + rsx-matching — the "cast treatment"

Give the orderbook (`rsx-book`) and matching engine (`rsx-matching`) the
same rigor `rsx-cast` got: measured latency/throughput, honest
state-of-the-art comparisons, why-notes, an rtrb-grade README, dated
reports, and an adversarial code audit. Ship **gradually**, review
between phases.

## What "the cast treatment" is (the template)

From `rsx-cast`, the bar to hit for each crate:

1. **`compare/` dir** — head-to-head vs named competitors: one speed
   table (measured-here | bench | published/pinned columns), a
   per-competitor note file, a one-workload honesty caveat up top,
   supporting-cast section at the bottom. Runnable benches for what we
   can build; doc-only (published figures) for commercial/closed ones,
   clearly labelled.
2. **Criterion benches** — p50 latency + throughput, pinned cores,
   fixed inputs, directly-comparable numbers, bench name cited by every
   figure.
3. **`notes/`** — why-decisions (not how-it-is).
4. **README to rtrb standard** — one-line elevator pitch, honest perf
   (with "run it yourself" caveat), generous alternatives, lineage/
   acknowledgments, explicit MSRV, no marketing, no badges.
5. **`reports/YYYYMMDD_*.md`** — dated durable record of each run.
6. **cto-eval** — adversarial audit + numeric grade + fixes.

## Current state (build on, don't redo)

- **rsx-book**: benches `book_bench.rs`, `deep_book_bench.rs`;
  `notes/{README,align,arena,hotcold}.md`. No `compare/`. README exists
  (grade TBD).
- **rsx-matching**: benches `match_n_levels_bench.rs`, `matching_bench.rs`,
  `process_order_bench.rs`, `wal_replay_bench.rs`. No `notes/`, no
  `compare/`.
- **reports/**: `20260530_component-benches.md` (ME ~210ns, deep-book
  match ~52ns), `20260530_load-curves.md` (match ~51ns depth-independent).
  Numbers exist but are scattered, not framed as a comparison.

Gaps to close: `compare/` dirs (both), `rsx-matching/notes/`, README
uplift (both), dated per-crate reports, code audit.

## Comparisons — the state of the art (real, citable)

### rsx-book (limit order book data structure)
- **WK Selph "How to Build a Fast Limit Order Book"** — the canonical
  hashmap + intrusive doubly-linked-list design, O(1) add/cancel/execute.
  Bench a faithful version as the reference point.
- **Price-ladder / direct array indexing** — what most HFT books do;
  RSX's compressed tick index is a variant. Compare index cost + memory.
- **`BTreeMap`/`std::map` book** — naive ordered-map baseline (bench it;
  the "TCP_NODELAY" of books — everyone knows it, shows the delta).
- **liquibook** (OCI, open-source C++) — real matching book; doc + (if
  feasible) an FFI/ported micro-bench, else doc-only with published notes.
- **kdb+/q** — commercial columnar in-mem; doc-only.
- **Databento `dbn` / market-by-order builders** — modern reference;
  doc-only.
- Our angle to defend: **compressed tick index + slab arena** — O(1)
  level access, zero-alloc, cache-line packed; the tradeoff is the
  sawtooth index (already a known limitation, see bugs.md book items).

### rsx-matching (matching engine)
- **LMAX Disruptor** (open-source ring buffer) + LMAX Exchange —
  mechanical-sympathy, published sub-100µs; bench a Disruptor-style
  baseline if feasible, else doc-only.
- **Chronicle Matching Engine** (Java, published sub-µs internal) —
  doc-only.
- **Nasdaq INET / OUCH-ITCH** — published wire-to-wire figures
  (~tens of µs); doc-only, cite the numbers.
- **CME Globex** — doc-only, published latency envelope.
- **liquibook** — open-source, real match loop; bench or doc.
- **Modern crypto perp matchers** — dYdX v4 (cosmos app-chain), Hyperliquid
  (custom L1), Injective, Sei — architectural comparison (on-chain/hybrid
  vs RSX's in-proc tile). Doc-only.
- Our angle to defend: **~340ns algorithmic match / ~54ns per fill,
  price-time FIFO, zero-alloc, i64 fixed-point** on the compressed book.

## Phasing (ship gradually + review each)

Numbers first — you cannot compare without your own measured baseline.

- **Phase 1 — Measure.** Consolidate + extend the existing benches into a
  clean, pinned, directly-comparable set per crate (latency by depth,
  by order type, throughput, slab/compression costs). Emit two dated
  reports: `reports/YYYYMMDD_book-benches.md`, `..._matching-benches.md`.
  *Review gate: are the numbers reproducible + honestly caveated?*
- **Phase 2 — Compare.** Build the baseline competitor benches we can run
  (BTreeMap book, hashmap+DLL book, naive matcher) under one harness per
  crate; write `rsx-book/compare/` + `rsx-matching/compare/` (table +
  per-competitor notes), doc-only for commercial. *Review gate: numbers
  directly comparable, commercial figures cited not faked.*
- **Phase 3 — Explain.** README uplift to rtrb grade (both); add
  `rsx-matching/notes/` why-docs (match loop, dedup, FIFO, IOC/FOK,
  event emission) and extend `rsx-book/notes/` (compression tradeoff,
  BBO-by-price). *Review gate: standalone-readable, honest.*
- **Phase 4 — Audit.** cto-eval on each crate (verify claims, attack the
  hot path, numeric grade), fix real findings. *Review gate: grade + all
  HIGH/MED findings resolved or logged.*

Each phase is its own commit set, reviewed before the next starts.

## Honesty guardrails (inherited from cast)

- One-workload caveat prominent (single box, synthetic, loopback,
  in-process — no wire-to-wire claim we can't back).
- Every number cites its bench + date; carried-over figures marked
  "(measured DATE, not re-run)".
- Never claim to beat a commercial/closed engine we can't run — doc-only
  with published figures, explicitly labelled as theirs.
- Real measured baselines for everything we CAN run.
- Keep the known limitations visible (sawtooth compression index, coarse
  far-from-mid price-time priority — already in bugs.md).

## Not in scope

- No hot-path rewrites chasing a comparison — measure and document first;
  optimization is a separate, later decision.
- No external publishing (per CLAUDE.md) — this is internal rigor +
  a GitHub-ready artifact, distribution is a founder call.

## Execution workflow (codex + opus) — added 2026-07-03

The lesson from cast: the *comparison* is where you fool yourself — cast's
oracle pass caught a bench that handed one design a free uncounted core.
So codex's role is **fairness**, not just prose.

Per phase: **opus implements → codex (oracle) reviews adversarially → I
integrate, run, verify numbers → commit → founder reviews the gate.**

- **codex/oracle owns**: (a) are competitor baselines *fair* (real, not
  strawmen)? (b) does the harness charge every contender equally (same
  payload, core pinning, alloc discipline)? (c) a no-fluff prose pass.
- **opus owns**: baseline competitor benches, `compare/` docs, README +
  `notes/` uplift.
- **I own**: running everything, reconciling to `reports/`, honesty
  guardrails, commits.

Applies to both crates. rsx-book first (its `compare/` dir is the only
fully-missing piece; benches + README + notes already exist to build on).

## Fairness & honesty — HARD requirements (added 2026-07-03)

These are non-negotiable; the comparison is worthless (or dishonest) without them.

### Uniform, minimal harness — same resources for every contender
- ONE shared harness module per crate (`#[path="harness.rs"] mod harness;`),
  used by every bench. It fixes: payload size, core pinning (client→core2,
  server→core3), warmup, `sample_size`, alloc discipline. No bench re-rolls
  its own — drift is how unfairness creeps in.
- Every contender gets the SAME cores, SAME payload, SAME measurement config.
- **Cast fairness bug found (2026-07-03): MoldUDP64 + SoupBinTCP benches are
  NOT pinned** (`TODO(pinning)` never done) while raw-UDP/KCP/TCP/Aeron/cast
  pin core2/3 — so the current cast table is not apples-to-apples. Fix as part
  of the shared-harness refactor (founder-authorized change to the frozen
  crate's benches). Also fix the stale "64-byte payload" doc headers in
  mold/soup (const is 128).
- Where a contender genuinely can't share resources (Aeron media-driver agents
  can't be pinned), document the asymmetry explicitly in its note — don't hide it.

### Flag every literature reimplementation
- Any competitor we build from a paper/spec/blog (MoldUDP64, SoupBinTCP,
  WK-Selph hashmap+DLL LOB, Disruptor-style matcher, array-ladder) is OUR
  clean-room reimplementation and **may be incorrect or unoptimized** — it can
  make us look good against a strawman or make the competitor look slow because
  we built it badly.
- Every such bench + its `compare/` note carries an explicit caveat: "clean-room
  reimplementation from <source>; measures our version, NOT the vendor's
  optimized code; treat as a reference-implementation baseline." Same standard
  the cast MoldUDP64 note already sets — make it consistent across all.
- **codex/oracle audits each reimplementation for faithfulness** (does it
  actually implement the algorithm, or a simplified/favorable variant?) BEFORE
  any number is published. This is codex's most important job here.
- Real-vendor numbers (published) stay doc-only and clearly attributed as
  theirs, never mixed with our measured baselines.
