# 20260530 — load curves (components / network / whole-e2e)

**What:** honest load-curve numbers across the three layers the founder asked for —
(A) individual components under load, (B) the network stack (rsx-cast) e2e, and
(C) the whole system GW→ME→GW. All numbers below come from bench logs captured this
session (`./tmp/bench-*.log`); nothing is hand-computed.

## Method

- **Commit:** `c38b956`
- **Box:** AMD Ryzen 9 5950X, env-reported **6-core, no isolation** (no
  `isolcpus`/pinning of the box; the cast + flood benches self-pin their worker
  threads to cores 2/3).
- **Build profiles (per measurement):**
  - Criterion benches (`cargo bench`): `profile.bench` = release, `opt-level=3`,
    `lto=false`, `codegen-units=16`. Steady-state, warmed, reports p50-ish
    (the middle of Criterion's `[lo est hi]` triple).
  - `bench-match-rt`: built `--release` (full `profile.release`: `lto=true`,
    `codegen-units=1`). Prints its own per-stage p50/p95/p99/max.
  - `risk_throughput_bench` / `risk_flood_bench`: `cargo bench`, but they print
    their OWN ops/s + p50/p99/p999 tables (not Criterion estimates).
- **Closed vs open loop, service-time vs latency-under-load:** labelled per row.

---

## (A) Individual components under load

### A.1 Matching engine — full accept path (in-process, no UDP/WS)

`rsx-matching/benches/process_order_bench.rs` — dedup miss + `OrderAcceptedRecord`
WAL append (no fsync) + `process_new_order` + `write_events_to_wal` +
`update_order_index`, on real `Orderbook`/`WalWriter`/`DedupTracker`.

| metric | value | kind |
|---|---|---|
| `me_process_order_full_path` | **205 ns** p50 (198–211) | service time, closed loop |

### A.2 Orderbook match — depth load curve (100k / 1M / 10M resting)

`rsx-book/benches/deep_book_bench.rs` (fat-tailed Student-t seed). This is the
clean 100k/1M/10M load point the founder framed — resting depth is the swept N.

| resting orders | match p50 | insert p50 | kind |
|---|---|---|---|
| 100k  | **51.6 ns** | 193 ns | service time, closed loop |
| 1M    | **50.1 ns** | 215 ns | service time, closed loop |
| 10M   | **52.0 ns** | 216 ns | service time, closed loop |

**Match is FLAT across 100k→10M** — O(consumed), not O(resting). At 10M the slab
is RAM-bound (~1.3 GB) but per-op cost does not move.

### A.3 Risk shard — capacity (service ceiling, closed loop)

`rsx-risk/benches/risk_throughput_bench.rs` — drives real `RiskShard::process_order`
/`process_fill`. ops/s = engine CPU ceiling, transport excluded.

| workload | ops/s (med) | p50 ns | p99 ns | kind |
|---|---|---|---|---|
| order-accept depth=0    | **10.0M** | 110 | 181 | service, closed loop |
| order-accept depth=8    | **8.52M** | 121 | 220 | service, closed loop |
| order-accept depth=64   | **5.15M** | 191 | 321 | service, closed loop |
| order-accept depth=512  | **1.13M** | 852 | 1313 | service, closed loop |
| reject NotInShard       | **96.0M** | 30  | 40  | service, closed loop |
| reject InsufMargin      | **26.2M** | 50  | 90  | service, closed loop |
| fill uniform-users      | **1.20M** | 691 | 2205 | service, closed loop |
| fill hot-users          | **4.83M** | 200 | 351 | service, closed loop |
| mixed 4ord:1fill depth=8| **4.24M** | 210 | 471 | service, closed loop |

Accept throughput falls with resting-order **depth** (`frozen_for_user` sums the
user's open orders per pre-trade check) — the dominant scaling variable.

### A.4 Risk shard — FLOOD load curve (open-loop, coordinated-omission-free)

`rsx-risk/benches/risk_flood_bench.rs` — order i is *due* at `start+i*gap`;
`latency = completion - due`. Knee = first rate where achieved/offered < 0.95.

**Table A — order flood (`process_order`):**

| offered/s | achieved/s | a/o % | p50 µs | p99 µs | p999 µs | max µs | backlog | knee |
|---|---|---|---|---|---|---|---|---|
| 100k  | 100k  | 100.0% | 0.23 | 208 | 3346 | 4706 | 469 | |
| 250k  | 250k  | 100.0% | 0.17 | 1513 | 6369 | 9298 | 2322 | |
| 500k  | 500k  | 100.0% | 0.17 | 11.2 | 561 | 2679 | 1337 | |
| 1M    | 1M    | 100.0% | 0.16 | 9.04 | 75.8 | 454 | 452 | |
| 2M    | 2M    | 100.0% | 0.16 | 20.6 | 1351 | 1979 | 3957 | |
| 3M    | 3.00M | 100.1% | 0.15 | 12.1 | 293 | 615 | 1844 | |
| 4M    | 4.00M | 100.0% | 0.16 | 47.7 | 257 | 327 | 1306 | |
| **6M**| 5.46M | **91.1%** | 18809 | 34013 | 34079 | 34079 | 199178 | **← KNEE** |
| 8M    | 5.60M | 70.0% | 54591 | 106037 | 107020 | 107086 | 605127 | |
| 12M   | 5.45M | 45.4% | 98042 | 199098 | 200802 | 200933 | 1083150 | |

The shard holds **flat ~0.16 µs p50 up to 4M orders/s**, knees at **~6M/s** (a/o drops
to 91%, scheduled-latency p99 explodes, backlog grows unbounded). The ~5.5M plateau is
the flood-HARNESS ceiling (per op it also paces + times + histograms) and sits BELOW the
pure engine ceiling (A.3 depth=8 = 8.5M) — use A.3 for the ceiling, this for the *shape*.

**Table B — fill flood + persist backpressure (drain rate swept):**

| drain ev/s | achieved fill/s | p99 µs | backpressured? | 1st-bp ms | bp-time ms | ring hwm |
|---|---|---|---|---|---|---|
| unthrottled | 113977 | 0.95 | yes | 1.63 | 3835 | 16380 |
| 20M | 115222 | 0.72 | yes | 0.80 | 3852 | 16380 |
| 5M  | 113977 | 0.72 | yes | 1.84 | 3852 | 16380 |
| 1M  | 88858  | 0.74 | yes | 0.82 | 3888 | 16383 |

The engine **STALLS, never drops** — achieved fill/s tracks drain_rate/~6 (≈6 persist
slots/fill). A slower PG sidecar directly throttles the fill engine. The
unthrottled/fast rows (~115k fills/s) are the cross-core SPSC persist-handoff ceiling in
*this* harness, NOT the pure `process_fill` CPU rate (see A.3 fill rows for that).

### A.5 Orderbook ops (`book_bench`, supporting)

| op | p50 | kind |
|---|---|---|
| price_to_index bisection | 1.91 ns | service |
| insert (zone 3) | 23.2 ns | service |
| modify price change | 570 ns | service |
| match 10 fills same level | 2.50 µs | service |
| match smooshed 100 levels | 5.96 µs | service |
| best-bid scan after cancel | 34.7 µs | service |
| recenter 10k orders | 283 µs | service |

`match_n_levels_bench`: n=1 → 12.7 µs, n=5 → 8.6 µs, n=20 → 9.6 µs, n=100 → 12.0 µs
(includes per-iter book rebuild; not directly comparable to the ns-scale deep_match).

---

## (B) Network stack (rsx-cast) — e2e

`rsx-cast/benches/{cast_one_way,cast_rtt}_bench.rs`. Both sides on 127.0.0.1,
threads pinned to cores 2/3, cache-hot. Full cast path: WalHeader build + CRC32C +
128-byte FillRecord send-ring → recv parse + CRC verify + in-order seq accounting.
**rsx-cast source is frozen — not edited.**

| measurement | p50 | kind |
|---|---|---|
| `cast_one_way_fill` (send→recv, 1 hop) | **3.89 µs** | latency, loopback, pinned |
| `cast_rtt_fill_echo` (A→B→A, 2 hops) | **7.60 µs** | latency, loopback, pinned |

One casting hop ≈ **3.8 µs** (loopback). This is the per-leg transport budget that
every GW↔Risk↔ME edge pays. Caveat: loopback only — real NIC adds IRQ + wire; the
per-recv `Vec` alloc noted in the bench header is in the harness, not the hot path.

---

## (C) Whole e2e (GW→ME→GW) — SINGLE STREAM ONLY

> **Parallel/flood whole-e2e is BLOCKED** by `ME-FAULTED-NO-REPLAY-ADDR`: a single
> dropped UDP packet under parallel load FAULTs the ME (panic, no replay addr). So
> only single-stream is reported. No parallel flood was forced through the live cluster.

### C.1 In-process round-trip (casting/UDP loopback + full ME), measured this session

`rsx-cli` `bench-match-rt --n 100000 --warmup 2000`, built `--release`. CastSender→UDP
loopback→ME (dedup + WAL accept + match + WAL events) →UDP→back. Per-stage p50 (ns):

| stage | p50 ns | p99 ns |
|---|---|---|
| gw_send | 3096 | 3908 |
| udp_to_me | 381 | 2254 |
| me_dedup | 80 | 1483 |
| me_wal_accept | 100 | 1804 |
| me_match | 70 | 260 |
| me_wal_events | 120 | 2415 |
| me_send | 3166 | 5661 |
| udp_to_gw | 380 | 851 |
| **TOTAL** | **7515** | **16891** |

**Whole in-process round-trip = 7.5 µs p50 / 16.9 µs p99.** The two `*_send`
syscalls (~3.1 µs each) dominate; all matching compute is ~370 ns of it. This is the
transport-bound floor — the same shape as a real GW→ME→GW but without WS framing,
gateway reactor scheduling, or the real network.

### C.2 Live gateway WS single warmed stream (could NOT re-run this session)

The live-cluster WS round-trip (`rsx-gateway/benches/ws_order_latency.rs`) requires the
full cluster (gateway + risk + ME + Postgres + 110 seeded users). This session the
playground orchestrator returned `400 from /api/processes/all/start` (a leftover
dashboard process occupied :49171; after clearing it the cluster `all/start` still 400'd
— a playground state issue, not a code regression). I did **not** fabricate a number.

The clean single-stream WS figures were **measured earlier today on the same commit
lineage** and are recorded in `reports/20260530_gateway-ws-rest-latency.md`:

| workload | p50 | p99 | p999 | max | kind |
|---|---|---|---|---|---|
| WS single warmed (1 conn, 30k) | **11.5 ms** | 21.8 ms | 132 ms | 878 ms | latency, closed loop, **measured today, prior run** |
| REST `/health` (fresh TCP) | 131 µs | 276 µs | 455 µs | 654 µs | same source |
| client floor (loopback echo) | 15 µs | 31 µs | — | 461 µs | same source |

The ~11.5 ms is **not** transport (REST over the same gateway is 131 µs) and **not**
the client (15 µs) — it is the gateway's single monoio reactor parking the
per-connection handler in a 10 ms `readable()` timeout while casting-recv delivers the
response (poll-loop egress starvation). This is **still-a-budget / a known bug**, not the
engine's floor — the C.1 in-process round-trip (7.5 µs) is the real GW→ME→GW compute+transport floor; the 11.5 ms is gateway egress scheduling on top.

---

## Conclusions

- **Compute is ns–µs; the system is transport- and gateway-scheduling-bound.**
  Matching is 51 ns flat to 10M resting; full ME accept 205 ns; risk accept 110 ns.
- **Per-shard risk ceiling ~8.5M order-accepts/s** (depth=8), holding flat p50 to 4M/s
  offered, kneeing at ~6M/s. Risk scales out by user shard.
- **One casting hop ≈ 3.8 µs loopback**; a 4-hop GW→Risk→ME→Risk→GW transport budget is
  ~15 µs, consistent with the 7.5 µs in-process 2-hop round-trip.
- **The 11.5 ms live-WS p50 is the gateway reactor egress bug**, not the engine — it is
  the baseline for the planned gateway egress-tile-split, not a fundamental floor.

## Caveats

- A.1–A.5, B: **in-process / loopback, steady-state, closed-loop or open-loop as
  labelled.** No real NIC, no kernel scheduling pressure beyond the box.
- B is **loopback** (same host); real network adds NIC IRQ + wire latency.
- A.3 ops/s are **service ceilings** (no queue); A.4 is the **open-loop latency-under-
  load** curve (the knee). A.4's plateau is a harness ceiling below A.3 — by design.
- C: **single-stream only.** Parallel/flood whole-e2e is blocked by
  `ME-FAULTED-NO-REPLAY-ADDR`.
- C.2 WS numbers were **measured earlier today** (same commit lineage) — the cluster
  could not be re-launched this session (playground `all/start` 400). Every other number
  here was captured this session from `./tmp/bench-*.log`.

**Sources:** `./tmp/bench-{process_order,deep_book,risk_throughput,risk_flood,cast_one_way,cast_rtt,book,match_n_levels,match-rt-100k}.log`,
commit `c38b956`. C.2 from `reports/20260530_gateway-ws-rest-latency.md`.
