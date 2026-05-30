# 20260530 — real-gateway WS + REST order latency (warmed clients, baseline)

**What:** order round-trip driven through the **real `rsx-gateway`** over its
actual WebSocket + REST transport with warmed clients, against a live cluster
(gateway + risk + matching; mark/marketdata/recorder skipped — not on the
resting-order path). Postgres in docker, 110 funded users seeded. Hand-rolled
blocking WS client (no async runtime), one OS thread per connection, JWT via
`jsonwebtoken`. Closed-loop (one in-flight order per stream).

## Numbers (all pairing-OK, 0 transport errors, ME survived)

| Workload | p50 | p99 | p999 | max | rate | outcome |
|---|---|---|---|---|---|---|
| WS single warmed (1 conn, 30k) | 11.5 ms | 21.8 ms | 132 ms | 878 ms | 82/s | 30000 rested |
| WS parallel (100 conn × 250, barrier) | 13.5 ms | 37.6 ms | 46 ms | 53.8 ms | 6584/s agg | 25000 rested |
| REST/TCP (`/health`, fresh conn) | 131 µs | 276 µs | 455 µs | 654 µs | 6944/s | 5000 ok |
| client floor (loopback echo, same path) | 15 µs | 31 µs | — | 461 µs | — | — |

Orders **rested** (non-crossing GTC limit BUYs; no maker, so none filled). The
client floor (15 µs p50) ran the identical masked-send / unmasked-read path
over loopback echo and is ~subtractable from the gateway numbers.

## Conclusion

The ~11.5 ms WS p50 is **not** transport and **not** the client: REST over the
same gateway is **131 µs** and the client floor is **15 µs**. It confirms the
known root cause (see `20260530_e2e-ws-probe.md`, `GATEWAY-LATENCY` in
bugs.md): the gateway's **single monoio reactor** parks the per-connection
handler in a 10 ms `readable()` timeout while the casting-recv loop delivers
the response — poll-loop egress starvation, internal reactor sharing, not the
wire. **This is the baseline for the planned gateway egress-tile-split**
(decouple casting-recv to a pinned busy-spin tile → SPSC → WS writers; shard
reactors).

## Caveats

- **Closed-loop** (one in-flight order per stream) — not a saturation /
  coordinated-omission test; the rate column is descriptive, not a throughput
  ceiling.
- Client threads float on non-pinned cores 0/5 while gw/risk/ME are pinned 1–3
  (scheduler noise possible).
- Run against a **fresh, empty ME book**.
- **Why single stream is 30k, not 100k:** with no maker and IOC not honored
  (see `IOC-NOT-HONORED`, bugs.md), every order rests and uncancelled GTCs
  exhaust rsx-book's fixed **65,536-deep slab** (it panicked the ME mid-build).
  The harness has a slab-budget guard refusing to run if
  `single_n + warmup + par_conn*(par_n+par_warmup) ≥ 65536`; defaults sized to
  61k < 65k. 100k uncancelled resting orders on one ME without a maker/IOC is
  physically impossible.

Source: `rsx-gateway/benches/ws_order_latency.rs` (commit landing this report),
live `playground start-all` cluster, 6-core box.
