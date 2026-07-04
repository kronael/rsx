# demo/rsx-book — the orderbook lib

How to demo `rsx-book` (the matching/orderbook lib). Read the `speed-demo` skill
first for the general method. This is a pure lib (no I/O) → the demo runs the
REAL `cargo bench` and records it live; numbers are never fabricated.

## The story: matching is O(1) in book depth
Matching one order stays **~60-65 ns whether the book holds 100 K or 10 M
resting orders** (`deep_flat_match`) — the happy path, where the touch level
survives the fill. When a match CLEARS the touch level, next-best-level
lookup stays O(depth=3) too (occupancy bitmap, `da9a2b4`): **145 ns**, still
depth-invariant, no longer the O(compression-slots) scan that used to cost
32-224 µs on that path (bugs.md `MATCHING-BENCH-ORDERTYPE-FIXTURE`, fixed).
FOK's fill-or-kill feasibility check (`can_fill_fully`) was a separate O(N)
full-book scan; fixed 2026-07-04 (bugs.md `FOK-AVAILABLE-LIQUIDITY-ON-SCAN`)
by walking only the crossing levels in price order — no more O(N) outlier.
Full numbers + caveats: `reports/YYYYMMDD_book-bench.md`.

## Artifacts in this folder
- `bench-live.sh` — runs the REAL `cargo bench -- deep_flat_match` and streams
  the actual Criterion results into a clean narrow view as they land (the
  "measuring…" pauses are real). This is the demo source of truth.
- `reveal.sh` — a faster scripted reveal (pre-set numbers) if a live run is too
  slow to record; keep it in sync with the report.
- `book-live.cast` / `book-live-opt.gif` — the recorded real run + the portrait
  GIF (578×700, ~13 KB).

## Regenerate the GIF
```
cd rsx-book/demo
# 1. box must be QUIET — stop the RSX cluster first (its ME+Risk busy-spin
#    poisons Criterion). Then record the real bench (~1 min):
TERM=xterm-256color asciinema rec --overwrite -c "bash bench-live.sh" book-live.cast
# 2. portrait GIF, trimming the real measuring pauses:
agg --cols 46 --rows 24 --font-size 20 --idle-time-limit 1 book-live.cast book-live.gif
gifsicle -O3 book-live.gif -o book-live-opt.gif
```

## Honesty (on screen + in the caption)
Single core · AMD Ryzen 9 5950X · Criterion · **a lab microbenchmark, not
system TPS** (the cross-process exchange round-trip is a separate, transport-
bound ~1 ms story). The live numbers vary run-to-run in the ~59-66 ns band —
that IS the honest picture; show the real per-run figures, not a rounded ideal.

## Do NOT
- Cite the OLD `match_by_type/*full` / `sweep` numbers (99 µs-1 ms) — those
  were the pre-fix O(compression-slots) scan bug (MATCHING-BENCH-ORDERTYPE-
  FIXTURE in bugs.md, fixed by `da9a2b4`), not a fixture artifact as first
  triaged. Current numbers: `ioc_full`/`gtc_full_cross` ~80 ns,
  `sweep_10_levels` ~700 ns. `fok_full` was ~296 µs (a separate O(N) FOK
  pre-check) — also fixed 2026-07-04 (`FOK-AVAILABLE-LIQUIDITY-ON-SCAN`), now
  in the same fast band as the other order types.
- "Faster than X" now has a baseline: `rsx-book/benches/compare_naive_bench.rs`
  (rsx-book vs. a naive `BTreeMap<price, VecDeque<order>>` book, same
  harness/box/depths). 1.5-2x on match/insert+cancel, 5.5-10x on cancel
  (widening with depth). See `reports/20260704_book-bench.md` for the table.
