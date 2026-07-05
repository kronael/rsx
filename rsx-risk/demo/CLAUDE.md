# demo/rsx-risk — the pre-trade gate

How to demo `rsx-risk` (the per-user-shard Risk tile). Read the `speed-demo`
skill first for the general method. This runs the REAL `cargo bench` and records
it live; numbers are never fabricated.

## The story: the pre-trade gate is ~110 ns, 45× under budget
Every order pays the Risk shard's margin check before it can reach the book.
The full pre-trade gate (`pretrade_check_latency`,
`reports/20260530_component-benches.md`) measures **~110 ns** — against a
per-order budget of 5 µs, that is **~45× of headroom**. The demo streams the
real supporting numbers too: applying a fill 3.7 ns, exposure lookup 1.6 ns
(flat 100→1000 users), BBO→index 5.6 ns.

## Artifacts in this folder
- `bench-live.sh` — runs the REAL `cargo bench -p rsx-risk --bench risk_bench`
  filtered to the four critical-path checks, streams the actual Criterion
  medians as they land (the "measuring…" pauses are real), then clears to a
  compact headline card. This is the demo source of truth.
- `risk-live.cast` / `risk-live-opt.gif` — the recorded real run + the portrait
  GIF (578×700, ~20 KB).

## Regenerate the GIF
```
cd rsx-risk/demo
# 1. box should be QUIET — stop the RSX cluster first (its ME+Risk busy-spin
#    poisons Criterion). Then record the real bench (~35 s):
TERM=xterm-256color asciinema rec --overwrite -c "bash bench-live.sh" risk-live.cast
# 2. portrait GIF, trimming the real measuring pauses:
agg --cols 46 --rows 24 --font-size 20 --idle-time-limit 1 risk-live.cast risk-live.gif
gifsicle -O3 risk-live.gif -o risk-live-opt.gif && rm risk-live.gif
```

## Honesty (on screen + in the caption)
Single core · shared docker host · Criterion · **a lab microbench, not system
throughput** (the full GW→Risk→ME→Risk→GW round-trip is transport-bound). The
pre-trade check measures ~105-108 ns run-to-run — the "~110 ns / 45×" headline
rounds it conservatively. The 5 µs budget is the per-order Risk latency target;
the measured floor sits ~45× under it.
