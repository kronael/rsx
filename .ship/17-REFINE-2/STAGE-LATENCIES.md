# Per-Stage Latencies — Proven Measurements

Captured 2026-05-22 after the probe + tracing fixes:
- `82e9966` — probe buffers F until U arrives (was racing to
  timeout when F preceded U)
- `2fc3bac` — FillRecord carries `taker_ts_ns`; all 6 stages
  anchor on the same gateway-ingress timestamp
- `0355f2a` — first reliable e2e baseline post-fix

Methodology: every order's 6 tracing emissions
(`gateway_in / risk_in / me_in / me_out / risk_out /
gateway_out`) carry the same `t0_ns` (the taker order's
gateway-ingress timestamp). The dashboard joins them by
`oid`. Coherent traces are those where every stage shares
`t0_ns` with `gateway_in` — i.e., none fell back to the
ts_ns anchor for missing `taker_ts_ns`.

## Coherent 6-stage trace, N=553

| Stage | p50 µs | p95 µs | p99 µs |
|-------|-------:|-------:|-------:|
| gateway_in | 0 | 0 | 0 |
| risk_in | 60 | 1 264 | 4 577 |
| me_in | 265 | 3 407 | 7 726 |
| me_out | 423 | 3 437 | 7 942 |
| risk_out | 462 | 4 054 | 10 874 |
| **gateway_out** | **1 128** | **6 513** | **14 264** |

## Per-leg breakdown (p50)

| Leg | Δ µs |
|-----|-----:|
| gateway → risk | 60 |
| risk → ME | 205 |
| ME match (in→out) | 158 |
| ME → risk return | 39 |
| risk → gateway → WS write | 666 |
| **GW→ME→GW total** | **1 128** |

## What this proves vs the probe number

- `e2e_us` p50 = 11 875 µs (from the Python aiohttp probe)
- `gw_only` p50 = 301 µs (Python WS round-trip alone — no
  risk/ME involvement; gateway rejects an invalid order fast)
- Rust GW→ME→GW p50 = 1 128 µs (the 6-stage measurement
  above, taken directly from the producer-side logs)

So the 11.8 ms probe number decomposes as:
- ~1.1 ms real exchange path
- ~10.7 ms Python aiohttp WS receive + asyncio scheduling

The earlier hypothesis ("95% is Python") was directionally
right but overstated. The actual ratio: **9.6%** of the
probe number is exchange work; **90.4%** is the probe
client itself.

## Where the Rust gap is

The 1.128 ms is **22× over the < 50 µs design budget**, not
the 234× the raw probe number implies. The biggest
contributor inside that:

- `risk_out → gateway_out` p50 = 666 µs (the gateway's WS
  flush + write_to_user path). This was previously
  invisible — F4.3's `gateway_out` was anchored on the ME
  emit time, so the leg looked tiny. With `taker_ts_ns` we
  can now see it.
- `risk → ME` p50 = 205 µs (CMP/UDP send + recv). Each side
  adds ~100 µs.

Next probable optimisations (out of refine-2 scope):
1. Examine the gateway's `push_to_user` → epoll path —
   667 µs to flush one WS frame is slow for monoio.
2. CMP receive batching at risk and ME — currently each
   datagram is one read syscall; could fuse.

## How to reproduce

```
./rsx-playground/playground start
curl -X POST 'localhost:49171/api/processes/all/start?scenario=minimal&confirm=yes'
# wait for maker to quote (about 10 s)
N=500 make latency-publish
# Then mine the logs:
python3 - <<'PY'
import re, statistics
from pathlib import Path
# (same script as in this report — joins by oid, filters
# coherent t0_ns, prints per-stage p50/p95/p99)
PY
```
