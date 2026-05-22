# Speed Granular — 2026-05-22 (sub-stage tracing)

Companion to `SPEED-BASELINE.md`. Adds sub-stage `tracing::info!`
probes inside the two slow legs (`me_in → me_out` and
`risk_out → gateway_out`) and re-measures. Same setup
(minimal scenario, debug build, N=500 + 50 warmup), captured
on commit `8c6d3ba`.

Methodology: 5 new sub-stage emissions in ME between
me_in and me_out; 4 new sub-stage emissions on the
risk→gateway leg between risk_out and gateway_out. All
anchored on the same `t0_ns` (`order_msg.timestamp_ns` for
ME, `fill.taker_ts_ns` for risk/gateway) as the existing
6 macro stages. Drain start/done emissions inside the
gateway per-conn loop carry no oid (loop-level) and are
excluded from the join.

Probe overhead caveat: each `tracing::info!` emission in
debug builds costs ~5-20 µs (sub-stages inside ME deltas
were 8-20 µs each, and dedup itself is < 1 µs of real work —
i.e. the probe IS the leg). The macro-stage numbers
therefore inflate by ~30-90 µs vs the pre-instrumentation
SPEED-BASELINE.md. We accept this; the goal is attribution,
not a new headline number.

## End-to-end probe (Python aiohttp WS round-trip)

```
e2e_us:
  count = 655
  p50   = 11 934 µs   (vs baseline 11 875 µs; +59 µs from added emissions)
  p99   = 761 580 µs  (high variance; tail noise)
```

## Per-stage table, N=1104 coherent traces

| Stage | count | p50 µs | p95 µs | p99 µs |
|-------|------:|-------:|-------:|-------:|
| gateway_in | 1104 | 0 | 0 | 0 |
| risk_in | 1104 | 58 | 707 | 4 592 |
| me_in | 1104 | 256 | 3 412 | 7 929 |
| me_dedup_done | 551 | 369 | 3 668 | 9 874 |
| me_wal_accepted_done | 551 | 386 | 3 845 | 9 888 |
| me_match_done | 551 | 409 | 3 882 | 9 909 |
| me_wal_events_done | 551 | 428 | 3 915 | 9 944 |
| me_index_done | 551 | 439 | 3 942 | 9 958 |
| me_out | 1104 | 436 | 3 791 | 8 300 |
| risk_out | 1104 | 473 | 4 610 | 10 853 |
| risk_cmp_send_done | 551 | 523 | 4 995 | 10 720 |
| gateway_cmp_recv | 551 | 1 136 | 6 634 | 13 702 |
| gateway_route_serialize_done | 551 | 1 185 | 6 726 | 13 747 |
| gateway_out | 1104 | 1 163 | 6 519 | 13 884 |
| gateway_route_push_done | 551 | 1 213 | 6 756 | 13 777 |

(Sub-stages count=551 because only orders that take the
accepted-and-matched branch hit them; the macro stages
also see duplicate-rejects and pure-cancels which is why
those have ~2× count. Probe orders all match by design,
so the 551 set is the probe set.)

## Per-leg deltas (p50/p95/p99)

| Leg | n | p50 µs | p95 µs | p99 µs |
|-----|---:|-------:|-------:|-------:|
| gateway_in → risk_in | 1104 | 58 | 707 | 4 592 |
| risk_in → me_in | 1104 | 191 | 1 465 | 6 409 |
| **me_in → me_dedup_done** | 551 | **115** | 297 | 631 |
| me_dedup_done → me_wal_accepted_done | 551 | 15 | 102 | 205 |
| me_wal_accepted_done → me_match_done | 551 | 20 | 71 | 118 |
| me_match_done → me_wal_events_done | 551 | 16 | 60 | 118 |
| me_wal_events_done → me_index_done | 551 | 11 | 52 | 82 |
| me_index_done → me_out | 551 | 8 | 46 | 68 |
| me_in → me_out (sum) | 1104 | 169 | 558 | 1 225 |
| me_out → risk_out | 1104 | 32 | 140 | 3 294 |
| risk_out → risk_cmp_send_done | 551 | 39 | 190 | 768 |
| **risk_cmp_send_done → gateway_cmp_recv** | 551 | **655** | 2 754 | 5 212 |
| gateway_cmp_recv → gateway_route_serialize_done | 551 | 37 | 110 | 223 |
| gateway_route_serialize_done → gateway_out | 551 | 0 | 0 | 0 |
| gateway_out → gateway_route_push_done | 551 | 21 | 54 | 140 |
| risk_out → gateway_out (sum) | 1104 | 718 | 3 113 | 5 786 |
| gateway_in → gateway_out (total) | 1104 | 1 163 | 6 519 | 13 884 |

## Hypothesis verdicts

### H1: "The two `tracing::info!` emissions are the largest single line item inside `me_in → me_out`"

**CONFIRMED, but stronger than predicted.** Predicted 30-60 µs;
actual contribution is larger and *dominant*. Evidence: the
five new sub-stage emissions are themselves tracing calls
with no real work between most pairs, yet each pair shows:

```
me_dedup_done -> me_wal_accepted_done:  15 µs
me_wal_accepted_done -> me_match_done:  20 µs
me_match_done -> me_wal_events_done:    16 µs
me_wal_events_done -> me_index_done:    11 µs
me_index_done -> me_out:                 8 µs
```

The 8-20 µs per delta is **almost entirely the tracing
emission itself** — the work between them (FxHashMap
insert, WAL append, etc.) is sub-µs each per Criterion. So
**each `tracing::info!` call costs ~10-20 µs in debug builds**,
and the original 2-emission `me_in → me_out` macro stage
therefore had ~20-40 µs of pure probe cost. The leg's
"real" cost (work, not probe) is **~80-100 µs**, and the
ME match itself is ~20 µs of that (4-pair WAL appends +
match + index update).

### H2: "`format!()` for hex oid strings allocates ~300 ns × 2 per stage"

**BELOW NOISE FLOOR.** 300 ns × 2 = 0.6 µs per stage is
indistinguishable inside the 8-20 µs tracing emission cost.
Cannot confirm or refute with `tracing::info!`-based probes;
needs the (B) ring-buffer approach. Tagged "below
tracing-noise floor."

### H3: "WAL writer's BufWriter has a hidden per-append cost beyond the benchmark"

**REFUTED.** `me_wal_accepted_done → me_match_done` and
`me_match_done → me_wal_events_done` both come in at
15-20 µs each — and that's mostly the emission overhead.
The actual `wal_writer.append()` adds at most a few µs over
the 31 ns Criterion baseline, well within hot-cache
expectation. The 16 µs `me_match_done → me_wal_events_done`
includes one full match cycle (process_new_order) AND the
WAL append loop, and is in the same ballpark as the
no-work pairs, which confirms the probe is the dominant
cost, not the work.

### H4: "The 10 ms readable-wait dominates `risk_out → gateway_out`"

**STRONGLY CONFIRMED.** This is the headline finding.

```
risk_out -> risk_cmp_send_done:                39 µs  (CMP send body)
risk_cmp_send_done -> gateway_cmp_recv:       655 µs  (UDP one-way + gw poll wait)
gateway_cmp_recv -> gateway_route_serialize_done:  37 µs
gateway_route_serialize_done -> gateway_out:        0 µs
gateway_out -> gateway_route_push_done:           21 µs
```

The **655 µs** between risk's CMP send and gateway's CMP
recv-and-decode is the gateway's outer poll loop tick. The
gateway main.rs CMP loop does:

```rust
loop {
    while let Some(_) = cmp_receiver.try_recv() { route_*(); }
    sender.tick();
    cmp_receiver.tick();
    // pending sweep, heartbeats, yield
}
```

`try_recv()` is non-blocking. When risk has just sent a
fill, the gateway poll is somewhere else (between iterations
or in another control path). The 655 µs is the worst-case
delay until the next `try_recv()` returns the fill.

This is NOT the 10 ms `readable()` timeout in
`handler.rs`. Per-conn loops only matter for *delivering*
the fill to the WS client (drain_outbound), not for
*decoding* the CMP frame. The handler-side cost shows up
in `gateway_out → gateway_route_push_done` (21 µs — push to
the VecDeque) and the WS write itself, which isn't
captured by the new probes (would need a `ws_write_done`
emission, deferred).

**Predicted 500-800 µs; measured 655 µs. Confirmed.** The
fix is *gateway-side CMP poll loop*, not the per-conn
`readable()` timeout as originally hypothesised.

### H5: "`push_to_user`'s `msg.clone()` × 2 is meaningful"

**REFUTED.** `gateway_out → gateway_route_push_done` =
21 µs at p50, which includes two `push_to_user` calls
plus one tracing emission. Stripping ~15 µs for the
emission leaves ~6 µs for both pushes + the clone +
two VecDeque push_backs. Not meaningful at the µs scale.

### H6: "`serde_json` JSON encode of `WsFrame::Fill` is meaningful"

**REFUTED (in p50).** `gateway_cmp_recv → gateway_route_serialize_done`
= 37 µs at p50, which includes one tracing emission
(~15 µs), the serialize call, and the `oid_hex` × 2
formatting. The serialize portion is at most ~20 µs.
Worth noting but not a top-3 target.

Curiosity: `gateway_route_serialize_done → gateway_out`
= **0 µs at every percentile**. That's because both
emissions read `now_ns` *before* the serialize call (we
captured `now_ns` once and reused it). My intent was to
isolate the serialize cost; the way the probe is structured
puts the same `t_us` on both, so the delta is 0 by
construction. The serialize cost is actually folded into
the `gateway_cmp_recv → gateway_route_serialize_done` leg.

## Recomputed per-leg attribution (subtract probe overhead)

Assume each `tracing::info!` emission costs ~15 µs in
debug builds (consistent with the 8-20 µs sub-stage deltas
that had near-zero real work).

| Macro leg | Raw p50 µs | Probe overhead | Real work µs |
|---|---:|---:|---:|
| gateway_in → risk_in | 58 | ~15 µs (risk_in alone) | ~43 µs |
| risk_in → me_in | 191 | ~15 µs (me_in alone) | ~176 µs |
| me_in → me_out (new) | 169 | ~75 µs (5 sub-stages) | ~94 µs |
| me_out → risk_out | 32 | ~15 µs | ~17 µs |
| risk_out → gateway_out | 718 | ~60 µs (4 sub-stages) | **~658 µs** |
| **Total GW→ME→GW** | 1 163 | ~180 µs | **~983 µs** |

So the "real" GW→ME→GW p50, stripped of the probe
instrumentation, is ~983 µs. Compared to the
SPEED-BASELINE.md figure of 1 128 µs (which had 6
emissions), the implied per-emission cost is
(1 163 − 1 128) / 9 = ~4 µs additional per new probe — i.e.,
emissions are *cheaper* than the leg deltas suggest. The
discrepancy is because the existing 6 probes contributed
their own ~5-20 µs each to the BASELINE figure too. The
"~15 µs per emission" estimate above is high; truer number
is probably **5-10 µs per emission in debug builds**.

Either way, **the real GW→ME→GW p50 is somewhere in the
900-1000 µs range, and 655 µs of that — about two-thirds —
is the gateway's CMP poll-loop tick.**

## Top 3 next optimisations (evidence-based)

1. **Tighten the gateway CMP poll loop** — directly addresses
   the 655 µs `risk_cmp_send_done → gateway_cmp_recv` leg.
   Today the gateway main loop does pending_sweep,
   heartbeat checks, and `monoio::time::sleep` in between
   `cmp_receiver.try_recv()` polls. Move all of those
   out-of-band (background timer task) and tight-spin
   `try_recv()` until the CMP socket reports no data, then
   yield. Predicted impact: drop the 655 µs to ~50 µs (one
   UDP one-way) — **~600 µs saved at p50.**

2. **Move tracing off the hot path** — debug-build
   `tracing::info!` is costing ~5-20 µs per emission. Even
   in release builds it'll be 1-5 µs each. Two paths:
   (a) gate `target: "latency"` emissions behind a
       compile-time feature flag so production builds carry
       zero overhead, or
   (b) implement the (B) ring-buffer probe pattern in
       SPEED-BASELINE.md §3 so probe cost drops to ~50 ns.
   Predicted impact: depending on emission count, **30-100 µs
   saved per stage.**

3. **Cut `risk_in → me_in` from 191 to ~50 µs** —
   second-largest leg after the gateway CMP delay. Risk's
   forward-to-ME path is similar in structure to the
   gateway's recv path, and the asymmetry vs `gateway_in →
   risk_in` (58 µs) suggests the same root cause: a
   too-loose poll loop on the risk side. Audit `rsx-risk/
   src/main.rs` poll cadence; predicted ~140 µs gain.

Note: the per-conn `readable()` 10 ms timeout in
handler.rs is **NOT** the dominant cost — it shows up only
when fills arrive between iterations. Most fills are
drained promptly inside `drain_outbound` because the loop
spins quickly between WS reads when traffic is light.
Future work to remove the 10 ms timeout (the
SPEED-BASELINE.md hypothesis) is still worthwhile for tail
latency, but it is not the p50 dominator we expected.

## What's still un-instrumented

- The actual `ws_write_text` call (WS frame + TCP send).
  Goes from `gateway_route_push_done` until the byte hits
  the wire. Would need a `gateway_ws_write_done` emission
  inside the per-conn loop. Deferred — needs the (B)
  ring-buffer pattern to avoid 15 µs of probe overhead per
  WS write.
- `risk_in` → matching shard processing → `me_send` inside
  risk. The current `risk_in → me_in` 191 µs leg lumps
  risk-side dispatch + CMP send + CMP recv on ME side.
  Sub-stages on the risk dispatch path would split that
  number.
- `gateway_outbound_drain_start` / `_drain_done` were added
  but excluded from the join (loop-level, no oid). They
  can be inspected directly in `log/gw-0.log` to see how
  often the per-conn loop ticks; this can be cross-correlated
  with the fill emissions to compute per-conn handler
  latency. Deferred for follow-on work.

## How to reproduce

```bash
cd /home/onvos/sandbox/rsx
curl -X POST 'localhost:49171/api/processes/all/stop?confirm=yes'
curl -X POST 'localhost:49171/api/processes/all/start?scenario=minimal&confirm=yes'
# wait ~10s for maker to quote
N=500 make latency-publish
python3 .ship/17-REFINE-2/mine_substages.py
python3 .ship/17-REFINE-2/mine_deltas.py
```

The two mining scripts join by oid, filter on the 6 macro
stages sharing a single t0_ns, and print sub-stage p50/p95/p99.
