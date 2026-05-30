# 20260530 — risk engine capacity + flood

**What:** risk-engine CPU ceiling + degradation under flood, driving the REAL `RiskShard::process_order`/`process_fill` (in-process, single shard, no UDP/WS). Source: `rsx-risk/benches/{risk_throughput_bench,risk_flood_bench}.rs`. Oracle-gated.

## Capacity (service ceiling, 10k users / 16 symbols / 64 hot)
| workload | ops/s | p50 ns | p99 ns |
|---|---|---|---|
| order-accept depth=0 | 10.2M | 101 | 171 |
| order-accept depth=8 | 9.30M | 120 | 180 |
| order-accept depth=64 | 5.57M | 181 | 271 |
| order-accept depth=512 | 1.16M | 841 | 1233 |
| reject NotInShard | 97.6M | 30 | 31 |
| reject InsufMargin | 27.2M | 51 | 80 |
| fill hot-users | 3.89M | 250 | 410 |
| mixed 4ord:1fill depth=8 | 4.38M | 200 | 461 |

Accept throughput falls with **resting-order depth** (`frozen_for_user` sums the user's open orders per check) — the main scaling variable.

## Flood (open-loop, latency vs scheduled-due)
Knee at **~8M orders/s offered**: achieved/offered drops to ~76%, scheduled-latency p99 explodes, backlog grows. Below ~5M: p50 ~0.15 µs, p99 < 100 µs.
**Persist backpressure: the engine STALLS, never drops** — achieved fills track drain_rate/~6 (1M drain → 91k fills, 300k → 27k); fast/unthrottled caps ~115k fills/s (cross-core SPSC handoff ceiling in-harness).

## Caveats
Engine-only (excludes casting recv/decode/UDP — gateway's concern); single shard/core (prod runs many shards parallel); flood-harness ceiling (~6M) is below the pure-engine ceiling (pacing+timing per op); default cardinality is warm-cache.
