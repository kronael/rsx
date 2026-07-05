# demo/rsx-matching — the matching engine

How to demo `rsx-matching` (the per-symbol ME tile). Read the `speed-demo`
skill first for the general method. This runs the REAL `cargo bench` and records
it live; numbers are never fabricated.

## The story: the match is O(1) in book depth
Matching one order stays **~30 ns whether the book holds 1 or 100 K resting
orders** (`match_by_depth`, `reports/20260703_matching-benches.md`) — a qty-1
taker does one non-draining partial fill, so match work is held constant and
depth is the only variable. Best-level access is O(1), so a fuller book/slab
does not slow the match. The trailer numbers (full order accept 266 ns,
duplicate rejected 3.7 ns) are labeled "from the report" on screen — cited from
the same report, not measured in this run.

## Artifacts in this folder
- `bench-live.sh` — runs the REAL `cargo bench -p rsx-matching --bench
  match_depth_bench` and streams the actual Criterion medians as they land
  (the "measuring…" pauses are real), then clears to a compact headline card.
  This is the demo source of truth.
- `match-live.cast` / `match-live-opt.gif` — the recorded real run + the
  portrait GIF (578×700, ~22 KB).

## Regenerate the GIF
```
cd rsx-matching/demo
# 1. box should be QUIET — stop the RSX cluster first (its ME+Risk busy-spin
#    poisons Criterion). Then record the real bench (~45 s):
TERM=xterm-256color asciinema rec --overwrite -c "bash bench-live.sh" match-live.cast
# 2. portrait GIF, trimming the real measuring pauses:
agg --cols 46 --rows 24 --font-size 20 --idle-time-limit 1 match-live.cast match-live.gif
gifsicle -O3 match-live.gif -o match-live-opt.gif && rm match-live.gif
```

## Honesty (on screen + in the caption)
Single core · shared docker host · Criterion · **a lab microbench, not system
TPS** (the cross-process GW→ME→GW round-trip is transport-bound — a separate
~ms story, see `reports/20260530_e2e-ws-probe.md`). The live numbers vary
run-to-run in the ~29-32 ns band — that IS the honest picture; show the real
per-run figures, not a rounded ideal. The report's own µs order-type/sweep
figures are a fixture artifact (see the report) — do NOT cite them here.
