# demo/rsx-book — the orderbook lib

How to demo `rsx-book` (the matching/orderbook lib). Read the `speed-demo` skill
first for the general method. This is a pure lib (no I/O) → the demo runs the
REAL `cargo bench` and records it live; numbers are never fabricated.

## The story: matching is O(1) in book depth
Matching one order stays **~60-65 ns whether the book holds 100 K or 10 M
resting orders** (`deep_flat_match`) — the happy path, where the touch level
survives the fill. Clearing the touch level costs **145 ns**, still
depth-invariant (occupancy bitmap, O(depth) next-best lookup). FOK
feasibility (`can_fill_fully`) walks only the crossing levels, early-exit.
Full numbers + caveats: `reports/20260704_book-bench.md`.

## Artifacts in this folder
- `bench-live.sh` — runs the REAL `cargo bench -- deep_flat_match` and streams
  the actual Criterion results into a clean narrow view as they land (the
  "measuring…" pauses are real). Opens on the thesis as a claim ("One order
  matches in ~60 ns — whether the book holds 100 K or 10 M.") and closes on a
  single CTA ("Read the code." / github.com/kronael/rsx). This is the demo
  source of truth.
- `reveal.sh` — a faster scripted reveal (pre-set numbers) if a live run is too
  slow to record; keep it in sync with the report.
- `book-live.cast` / `book-live-opt.gif` — the recorded real run + the portrait
  GIF (578×700, ~42 KB).

## Regenerate the GIF
```
cd rsx-book/demo
# 1. box must be QUIET — stop the RSX cluster first (its ME+Risk busy-spin
#    poisons Criterion). Then record the real bench (~1 min):
make rec
# 2. portrait GIF (renders with agg --theme monokai), trimming measuring pauses:
make gif
```

## Palette
Uses the project's canonical **"Cemani"** palette — the sampled hexes and the
meaning of each hue are documented once in `rsx-cast/demo/CLAUDE.md` (don't
duplicate them here). The mapping in this demo: **teal** = the live/fast
benchmark result numbers, **gold** = the headline claim + the closing CTA,
**rust** = the thing being beaten (BTreeMap, the cited C++ ITCH line), **dim**
= captions/caveats, all on a warm-dark **`agg --theme monokai`** base.

## Honesty (on screen + in the caption)
Single core · AMD Ryzen 9 5950X · Criterion · **a lab microbenchmark, not
system TPS** (the cross-process exchange round-trip is a separate, transport-
bound ~1 ms story). The live numbers vary run-to-run in the ~59-66 ns band —
that IS the honest picture; show the real per-run figures, not a rounded ideal.

The sweep numbers are the ones measured live. The trailer numbers (145 ns
clears-touch, 18 ns cancel) are **labeled "from the report"** on screen — they
are cited from `reports/20260704_book-bench.md`, not measured in this run. The
C++ ITCH `61 ns/tick` line is **cited only**: it is book-*maintenance* on
~2012 hardware in another language, NOT a matching engine — so the fair
rsx-book line to place next to it is insert/cancel, not `match_*`. The trailer
says exactly this (`compare/cross-language-cited.md`); do not upgrade it to
"on par with C++" or imply a head-to-head match comparison. The payoff line
"10 M orders — still ~60 ns" is drawn from the live sweep just shown.

## Do NOT
- Cite µs-range `match_by_type`/`sweep`/`fok_full` figures from older
  reports — superseded. Current band: `ioc_full`/`gtc_full_cross` ~80 ns,
  `sweep_10_levels` ~700 ns, `fok_full` ~118 ns. Only cite
  `reports/20260704_book-bench.md`.
- "Faster than X" has exactly one baseline: `compare_naive_bench.rs`
  (vs a naive `BTreeMap<price, VecDeque<order>>` book, same harness):
  1.5-2x match/insert+cancel, 5.5-10x cancel. Same report.
