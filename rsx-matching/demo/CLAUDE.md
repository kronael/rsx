# demo/rsx-matching — the matching engine

How to demo `rsx-matching` (the per-symbol ME tile). Read the `speed-demo`
skill first for the general method. This runs the REAL `cargo bench` and records
it live; numbers are never fabricated.

## The story: the match is O(1) in book depth
Matching one order stayed **46.343–49.491 ns from 1 to 100 K resting
orders** (`match_by_depth`, `reports/20260716_book-matching-publication.md`) — a qty-1
taker does one non-draining partial fill, so match work is held constant and
depth is the only variable. Best-level access is O(1), so a fuller book/slab
does not slow the match. Accept and throughput are separate fixtures in the
dated report, not claims made by the live depth demo.

## Artifacts in this folder
- `bench-live.sh` — runs the REAL `cargo bench -p rsx-matching --bench
  match_depth_bench` and streams the actual Criterion medians as they land
  (the "measuring…" pauses are real), then clears to a compact headline card.
  Derives the final displayed range from those results and closes on a single
  CTA ("Read the code." /
  github.com/kronael/rsx). This is the demo source of truth.
- `Makefile` — `make rec` (asciinema, pinned --cols 46 --rows 24) →
  `make gif` (agg --theme monokai + gifsicle).
- `match-live-opt.gif` — the tracked postable GIF (raw `.gif` and
  `.cast` are gitignored intermediates; `make rec gif` regenerates). The
  portrait GIF (578×700, ~24 KB).

## Regenerate the GIF
```
cd rsx-matching/demo
# 1. box should be QUIET — stop the RSX cluster first (its ME+Risk busy-spin
#    poisons Criterion). Then record the real bench (~45 s):
make rec
# 2. portrait GIF (renders with agg --theme monokai), trimming measuring pauses:
make gif
```

## Palette
Uses the project's canonical **"Cemani"** palette — sampled hexes + hue
meanings documented once in `rsx-cast/demo/CLAUDE.md` (don't duplicate here).
Mapping in this demo: **teal** = live/cited result numbers, **gold** = the
claim opener, the flat-across-depth payoff, and the CTA, **dim** =
captions/caveats, on a warm-dark **`agg --theme monokai`** base.

## Honesty (on screen + in the caption)
Single core · shared 8-vCPU KVM host · Criterion · **a lab microbench, not system
TPS** (the cross-process GW→ME→GW round-trip is transport-bound — a separate
~ms story, see `reports/20260530_e2e-ws-probe.md`). The committed GIF is a
later live capture whose rounded medians span 54–58 ns; the dated report records
the earlier 46–49 ns run. Show that shared-host variance instead of smoothing
it away. The older report's µs order-type/sweep
figures are a fixture artifact (see the report) — do NOT cite them here.
