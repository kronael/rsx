# demo/rsx-book — the orderbook lib

How to demo `rsx-book` (the matching/orderbook lib). Read the `speed-demo` skill
first for the general method. This is a pure lib (no I/O) → the demo runs the
REAL `cargo bench` and records it live; numbers are never fabricated.

## The story: fixed-work depth run
The 2026-07-27 run observed no depth-related latency increase from
100,000 to 10 million resting orders. It measures one non-draining partial
fill, not a sweep or a compressed-level match. Same-run p50, standard error
and p99: `reports/20260727_book-deep-match-tail.md`. That report is the only
evidence source for the demo's numbers.

## Artifacts in this folder
- `bench-live.sh` — the demo source of truth. It runs the real
  `cargo bench -- deep_flat_match` and streams Criterion results as they land.
  The final depth range is derived from the output, not hard-coded.
- `assets/gen_collage.py` — builds the four-player intro. Each medallion adds
  one design step to a persistent progression meter; the final, larger
  Gigachad card alone shows measured p50, standard error, and p99.
- `assets/book-versus.gif` — the graphics-only playground artifact. The four
  lanes are an illustrated design progression, not four measured contenders.
  The game ends on a same-run p50/SE/p99 table and measured conclusion; it does
  not cut to the terminal recording.
- `reveal.sh` — a faster scripted reveal that prints fixed numbers if a live
  run is too slow to record. Its numbers must match the report above.
- `book-live-opt.gif` — the tracked terminal recording used after the intro
  (raw `.gif` and `.cast` are gitignored intermediates).

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
Single core · shared AMD Ryzen 9 5950X KVM host · Criterion · **a lab microbenchmark, not
system TPS**. Shared-host variance between runs is part of the evidence, not
something to hide.

The sweep numbers are measured live, and the depth-range payoff is derived
from that output. Do not add old trailer figures or cross-language numbers to
the live card; those belong in their dated reports and comparison notes.

## Do NOT
- Put a latency figure on screen without p50 and p99 from one run in a dated
  report. Older reports hold superseded µs-range figures.
- Say "faster than X" without the baseline: `compare_naive_bench.rs` (a naive
  `BTreeMap<price, VecDeque<order>>` book with an order locator, same
  harness). It shows 1.5-2x on match and insert+cancel and 5.5-10x on cancel,
  in `reports/20260704_book-bench.md`.
