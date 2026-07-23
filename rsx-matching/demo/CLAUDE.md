# demo/rsx-matching — the matching engine

How to demo `rsx-matching` (the per-symbol ME tile). Read the `speed-demo`
skill first for the general method. This runs the REAL benches live and
records them; numbers are never fabricated.

## The story: match → recover → round-trip
A 3-beat arc, each a REAL measurement:

1. **THE MATCH** — the full accept path per order (`process_order_bench`,
   `me_accept_path` + `me_throughput`): dedup + `OrderAccepted` WAL append +
   `process_new_order` + event fan-out + order-index. ~**0.3–0.5 µs/order**,
   ~**2.9 M orders/s**. Single-box, in-process, self-pinned Criterion.
2. **RECOVERY** — cold-start WAL replay (`wal_replay_bench`): drain 30 K
   records (10 K logical orders) to rebuild the book before serving order #1.
   ~**34 ms**. FIFO-preserving: replay order = disk order = match order.
3. **WIRE-TO-WIRE** — the live cluster's full in-exchange round-trip
   (gw→risk→ME→risk→gw) from the embedded per-hop stamps
   (`gw_out_ns − gw_in_ns`), server-side, Python client EXCLUDED.
   **~0.65 ms p50** (the matching leg is ~12 µs of it; the rest is
   transport). Clean-window sample, shared host —
   `reports/20260723_gateway-egress.md`.

## Honesty (on screen + in the caption)
- Beats 1–2 are single-box, self-pinned Criterion microbenches (a compute
  floor) — never quote them as end-to-end latency.
- Beat 3 is the real cross-process round-trip; it is **sub-millisecond, NOT
  sub-50µs**. The sub-50µs figure earlier notes cite was `bench-match-rt`'s
  2-hop model (gateway↔ME directly, **no risk tile**) — a useful
  transport+match lower bound, not the 4-hop round-trip. See the report.
- Depth-independence (the match is O(1) in book depth) is still the
  documented headline in `notes/depth-independent.md` +
  `benches/match_depth_bench.rs`; this demo just leads with the accept path.

## Artifacts in this folder
- `bench-live.sh` — runs the REAL `process_order_bench` + `wal_replay_bench`
  (streaming Criterion medians as they land) then the wire-to-wire card
  (`WIRE_P50` pasted from the gateway-egress report), closing on the neutrino
  mascot + CTA. The demo source of truth. Parses only the value `thrpt:` line
  (Criterion also emits a change-vs-baseline `thrpt:` — filtered).
- `Makefile` — `make rec` (asciinema, --cols 46 --rows 24) → `make gif`
  (agg --theme monokai + gifsicle).
- `match-live-opt.gif` — the tracked postable portrait GIF (~31 KB; raw
  `.gif`/`.cast` are gitignored intermediates; `make rec gif` regenerates).

## Regenerate the GIF
```
cd rsx-matching/demo
# 1. QUIET box — stop the RSX cluster (its busy-spin poisons Criterion).
#    Self-pinned Criterion tolerates other load, but a stopped cluster is the
#    reliable path. Pre-build first: cargo bench -p rsx-matching --no-run.
make rec
# 2. portrait GIF (agg monokai, trims the measuring pauses):
make gif
```

## Palette
Project "Cemani" palette (source `rsx-cast/demo/CLAUDE.md`). Here: **teal** =
live/cited result numbers, **gold** = beat headings + CTA, **dim** =
captions/caveats, on `agg --theme monokai`.
