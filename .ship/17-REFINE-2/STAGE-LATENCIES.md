# Per-Stage Latencies — First Measurement

Captured 2026-05-22 after the refine-2 fixes landed, against
the live cluster (gw-0, risk-0, me-pengu running fresh
binaries from this round). Lines from `log/{gw-0,risk-0,
me-pengu}.log` filtered on `target=latency`.

## Sample from the live cluster

```
gateway_in  t_us=0            oid=019e503e8d1c7dd28f0f12207baaff97
risk_in     t_us=77   (Δ 77)  same oid
me_in       t_us=126  (Δ 49)  same oid
me_out      t_us=137  (Δ 11)  same oid
```

Multiple consecutive orders showed deltas in the same band:

| Stage     | Δ µs (range) | Cumulative µs |
|-----------|--------------|---------------|
| gateway_in → risk_in | 77-307 | 77-307 |
| risk_in → me_in | 49-150 | 126-457 |
| me_in → me_out | 11-25 (match itself) | 137-482 |
| me_out → risk_out | (not yet sampled — broken-pipe path issue) | — |
| risk_out → gateway_out | (same) | — |

The half-round-trip (gateway → ME match exit) is therefore
**~150-500 µs**, depending on host load. That's the inner
Rust path the project's `<50 µs` design budget actually
addresses.

## What this means for the headline number

The previous bench-baseline.json (p50 = 11 780 µs / p99 =
233 447 µs) is **the Python aiohttp probe round-trip**, NOT
the Rust exchange path. The CTO+CEO synthesis and the
pre-refine diagnosis both predicted this. Now we have
direct measurement on the same workload to back it.

Approximate breakdown of the 11 780 µs p50:
- Python aiohttp WS handshake / send / recv / GIL: ~10 000 µs
- Two full WS round-trips (U frame + F frame): doubled
- Real exchange path (Rust): ~500 µs total

So the project is roughly **10× over budget on the Rust
side** (500 µs measured vs 50 µs designed) and roughly
**zero excuse** for the Python probe being on the headline
metric. The honest end-to-end-from-Rust number is what the
native bench-probe (F4.1) needs to produce.

## Known gap

The probe is currently failing with a gateway "Broken pipe"
on the close path: the aiohttp `ws_close` happens before
the F frame can complete its write back to the client.
Symptoms: `/api/latency-probe` returns `ok: false, error:
"timeout waiting for fill", skipped_fills: 1` consistently
even when the maker is quoting actively. Per-stage tracing
confirms the gateway/risk/ME ingest the order but the
write-back leg can't reach the closed WS.

Two paths to fix (out of refine-2 scope):
1. Make `_run_latency_probe` keep the WS open longer than
   the deadline — read until either F-match OR deadline,
   without closing on first read of a non-F frame.
2. Or: switch `make latency-publish` over to `rsx-cli
   bench-probe` once that's been hardened for production
   use.

## Open follow-ups

- Capture the post-refine p50/p99 (blocked by the broken
  pipe — see above).
- Wire the per-stage medians into `/x/latency-stages`
  endpoint (Bucket 4 shipped the endpoint but it needs the
  fresh log format proven above).
- Add per-stage thresholds: alert if gateway_in→risk_in
  exceeds 1 ms (= 10× the band measured today).
