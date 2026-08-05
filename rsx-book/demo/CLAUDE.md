# demo/rsx-book — the orderbook lib

How to demo `rsx-book` (the matching/orderbook lib). Read the `speed-demo` skill
first for the general method. This is a pure lib (no I/O) → the demo runs the
REAL `cargo bench` and records it live; numbers are never fabricated.

## The story: fixed-work depth sweep
The current fixed-work run observed no depth-related latency increase from
100,000 to 10 million resting orders. It measures one non-draining partial
fill, not a sweep or compressed-level match. Same-run p50 and p99:
`reports/20260727_book-deep-match-tail.md`.

## Artifacts in this folder
- `bench-live.sh` — the demo source of truth. It runs the real
  `cargo bench -- deep_flat_match` and streams Criterion results as they land.
  The final depth range is derived from the output, not hard-coded.
- `assets/gen_collage.py` — builds the four-player intro. Each medallion adds
  one design step to a persistent progression meter; the final, larger
  Gigachad card alone shows measured p50, standard error, and p99.
- `assets/book-versus.gif` — the playground artifact: medallion intro, then
  the recorded terminal benchmark.
- `reveal.sh` — a faster scripted reveal (pre-set numbers) if a live run is too
  slow to record; keep it in sync with the report.
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
system TPS** (the cross-process exchange round-trip is a separate, transport-
bound ~1 ms story). The committed GIF is a later live capture whose rounded
medians span 70–71 ns; the dated report records the earlier 69–71 ns run. That
shared-host variance is part of the evidence, not something to hide.

The sweep numbers are measured live, and the depth-range payoff is derived
from that output. Do not add old trailer figures or cross-language numbers to
the live card; those belong in their dated reports and comparison notes.

## Do NOT
- Cite µs-range `match_by_type`/`sweep`/`fok_full` figures from older
  reports — superseded. Current band: `ioc_full`/`gtc_full_cross` ~80 ns,
  `sweep_10_levels` ~700 ns, `fok_full` ~118 ns. Only cite
  `reports/20260704_book-bench.md`.
- "Faster than X" has exactly one baseline: `compare_naive_bench.rs`
  (vs a naive `BTreeMap<price, VecDeque<order>>` book, same harness):
  1.5-2x match/insert+cancel, 5.5-10x cancel. Same report.
