# rsx-book → cast-quality: 3-eval findings + refine plan (2026-07-04)

Three read-only evals of rsx-book (matching engine) after the correctness push
(tick-size fix, distribution tests/benches, MIGRATE fix, cast-quality docs).

## Grades
- **13yo (newcomer on-ramp):** a domain expert gets it in 10s; a true newcomer
  bounces at sentence 2. No plain-English "what a matching engine is" + no felt
  speed anchor.
- **CEO (adoptability):** 7/10 — "technically excellent, unusually honest
  benchmarking, wrapped in an internal-only package that literally cannot be
  adopted." ~8.5 on bench credibility, ~5 on adoptability.
- **CTO (technical):** 8.2/10 — SHIP for the single-symbol/demo mandate; HOLD on
  the "finalized/frozen" label until 4 narrow items close. Every load-bearing
  claim CONFIRMED (O(1)/depth-invariant all paths, isolated benches, invariants
  cross-checked vs brute-force, zero-heap/no-floats, fair compare/, slab no-leak).

## Refine scope

### A. Correctness (CTO — the finalize-blockers) — IN FLIGHT
1. **FOK-RESTS-IN-COMPRESSED-ZONES (HIGH, latent).** `can_fill_fully` sums a
   level's `total_qty` + tests only the head price; in compressed zones (distinct
   raw prices per slot) it over-counts → FOK passes feasibility → residual branch
   only cancels IOC → a FOK falls through to `insert_resting`. Fix: accurate
   per-order feasibility in compressed zones + FOK residual must cancel, never
   rest. Add a "FOK never rests" test over tick≠1.
2. **ME-REDUCEONLY-IOC-FILLEDQTY.** `filled = qty - remaining` counts the
   reduce-only clamp as execution → wrong client filled_qty (position safe).
3. **BOOK-SLAB-FREE-UNGUARDED / BOOK-STALE-HANDLE-REUSE.** No double-free guard;
   stale-handle aliasing. Add guard + settle the cross-crate contract.
   (Out of scope: panic-on-exhaustion + release overflow guard — correct per the
   ME trust boundary.)

### B. Packaging (CEO — biggest adoptability gap) — QUEUED
- Add `LICENSE` (dual MIT/Apache) + `license = "MIT OR Apache-2.0"` in Cargo.toml
  + a git-dep install snippet + a standalone `examples/book_smoke.rs` (insert →
  match → cancel → print events). Mirror rsx-cast exactly.

### C. Docs / demo (13yo + CEO) — QUEUED
- README: a 3-sentence plain-English preamble at the very top (what a matching
  engine is / why staying fast with millions of resting orders is hard / a felt
  anchor: "60 ns — light travels ~18 m").
- **Fix the broken quick-start** (config/mid_price undefined, `handle` used out
  of scope — won't compile).
- Reframe the "so what" toward the realistic-depth cancel advantage (5.5–10×,
  widening), not the 10M-deep flex.
- One-line plain glossary for symbol / tick / depth-invariant / compression map /
  occupancy bitmap.
- Demo GIF: add an on-screen payoff line ("10 M orders — still 60 ns"); fix the
  "on par with C++ ITCH 61 ns" caption (that's 2012 book-maintenance; the fair
  line is insert/cancel, not match) + label the hardcoded trailer as cited, not
  measured-live.

### D. Deferred / harder
- Tail latency (p99) — benches are p50-only; re-run with percentiles on a quiet
  box, especially the level-clearing path.
