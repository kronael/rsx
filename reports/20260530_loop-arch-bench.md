# 20260530 — loop-architecture bench (monoio vs tokio vs tile vs batched)

**What:** synthetic gateway-shaped loop `recv → calc(512B memcpy) → submit to single echo → recv echo → send up`, ~10k/500 conns, **equal core budget K** (tile/batched charged for their helper core), closed-loop. Source: `rsx-risk/benches/loop_arch_bench.rs`. Oracle-critiqued (gpt-5.5) — fixed the original's free-helper-core unfairness.

## Numbers (closed-loop, conns=500, p50/req-s)
| variant | K | p50 µs | req/s | echo-sys/req |
|---|---|---|---|---|
| tokio | 2 | 2634 | 135,611 | 2.00 |
| monoio-sharded | 2 | 3496 | 120,671 | 2.00 |
| busy-spin-tile | 2 | 7807 | 57,232 | 2.00 |
| batched-syscall | 2 | 7205 | 61,151 | **0.23** |

## Conclusion
- **Syscall-bound:** 2.00 syscalls/req, calc ~22 ns (negligible). The work isn't the cost — the syscalls are.
- **The busy-spin tile does NOT win** — once charged for its helper core it's the worst. Reorganizing work into a tile is not a free speedup at this scale.
- **tokio's work-stealing wins** under closed-loop saturation (best throughput → lowest closed-loop latency).
- **Batching is the real lever:** `recvmmsg/sendmmsg` cut sys/req to ~0.23, but naive batching needs in-flight depth (pipeline>1) or throughput collapses.

## Caveats
**Closed-loop saturation** → latency ≈ in-flight/throughput, so "tokio fastest" is a throughput artifact, NOT per-request latency at sustainable load. Open-loop (`LAB_OPEN_RATE`) mode exists; a clean open-loop per-variant latency run is still TODO (the run hung at busy-spin-tile — investigate). Synthetic, loopback, single box.
