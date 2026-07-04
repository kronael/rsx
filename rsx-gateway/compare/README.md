# rsx-gateway: order-ingress / WS-edge alternatives — the wider field

Companion to `rsx-cast/compare/` and `.ship/34-COMPARE-RESEARCH/`.
This is the `niche.md`-style census for the client-facing edge: where
the gateway sits against binary kernel-bypass matching venues
(Nasdaq INET), public-WS crypto exchanges, and the C10M WS-scaling
field.

**Read the honesty trap first — it is the whole point of this doc.**
RSX's live single-warmed-WS order round-trip is **11.5 ms p50**. That
is a **KNOWN egress-starvation bug** (`GATEWAY-LATENCY`), **not the
gateway floor**. The gateway's real transport+compute floor is the
in-process GW→ME→GW round-trip: **7.5 µs p50 / 16.9 µs p99**
(`reports/20260530_load-curves.md` C.1). The ingress path is sub-µs and
alloc-free. Quoting 11.5 ms as "gateway latency" is the single most
dishonest thing one could do with this component — every external
comparison below must state which number and why they differ.

Second: the gateway is a **WS-JSON / monoio** edge; the venues it is
cited against are **binary / kernel-bypass** (OUCH over DPDK/FPGA).
Different protocol class and I/O layer — directional universe check
only, never a ratio.

## What rsx-gateway is

The client-facing WS/REST edge (`rsx-gateway/`, ~3.6k LOC, single
monoio/io_uring reactor, `Rc<RefCell<GatewayState>>`, no locks). Per
connection it reads a WS frame (≤4 KB), UTF-8 + JSON parse, JWT auth
(HS256 + jti-replay), two-layer token-bucket rate limit (per-IP +
per-user), circuit breaker, symbol/tick/lot validation, mints a UUIDv7
oid, and forwards a stack-struct `OrderRequest` to Risk via
`CastSender::send_raw` (**allocation-free** binary path). The return
path decodes Fill/OrderInserted/… off the casting/UDP recv loop and
fans JSON out to each connection's `VecDeque<String>`. It is **on the
GW→ME→GW <50 µs critical path** and holds no durable state (crash =
clients reconnect; Risk + WAL are authoritative).

## The one metric that matters

**Order ingress→cast service time** (frame-in → `send_raw` to Risk) and
**concurrent-WS-connection scaling** (accept + handshake + per-conn
egress latency as connection count climbs). The ingress path is sub-µs
and alloc-free; the *egress* path is where the known bug lives.

## Honest reference points

| System | Number | HW / protocol caveat | Head-to-head? | Source |
|---|---|---|---|---|
| **Nasdaq INET** | round-trip **<40 µs** wire-to-wire; **37 µs** at network boundary (OUCH/10GbE); fastest prod **14 µs** door-to-door; OrderAck inline on receipt | kernel-bypass, user-space net, FPGA-assisted, **binary OUCH** | no — DPDK/FPGA + binary vs our monoio + WS-JSON | [A-Team](https://a-teaminsight.com/blog/with-six-rollout-nasdaq-omx-pushes-matching-latency-below-40-microseconds/) |
| **arxiv "World's Fastest ME"** (2026) | p50 **376 ns** / p99 **524 ns @ 5M msg/s** | AWS r8g.metal (ARM Graviton4), unreleased, full ack path | no — ARM, not our box, not a gateway | [arxiv 2606.01183](https://arxiv.org/html/2606.01183v1) |
| **Crypto exchanges** (Coinbase/Kraken/Binance) | exchange processing **5–10 ms**; WS transport <50 ms gw→client | production internet-facing, includes *full matching* | directional — same *protocol class* (public WS), full-stack | [CoinAPI](https://www.coinapi.io/blog/crypto-trading-latency-guide) |
| **AWS "tick-to-trade" digital assets** | **sub-ms** end-to-end under realistic load | optimised cloud arch, tuned | directional — the universe our 7.5 µs floor sits in | [AWS](https://aws.amazon.com/blogs/web3/optimize-tick-to-trade-latency-for-digital-assets-exchanges-and-trading-platforms-on-aws/) |
| **C10M / MigratoryData** | **10–12M** concurrent conns / 12 cores; WhatsApp 2M/24 cores | connection-scaling context, not order latency | directional — ceiling for the "io_uring for 100k+ conns" claim | [MigratoryData](https://migratorydata.com/blog/migratorydata-solved-the-c10m-problem/), [C10k wiki](https://en.wikipedia.org/wiki/C10k_problem) |

## RSX in-repo anchors (`reports/20260530_load-curves.md`, 6-core Ryzen)

| measurement | number | what it is |
|---|---|---|
| in-process GW→ME→GW round-trip | **7.5 µs p50 / 16.9 µs p99** | the transport+compute **floor** (C.1) |
| REST `/health` over the same gateway | **131 µs** | proves the wire is not the problem |
| hand-rolled thread-per-conn WS client floor | **15 µs** | the naive baseline — *beats* the buggy reactor |
| live single-warmed-WS order round-trip | **11.5 ms p50** | **known egress bug**, NOT the floor |

The 11.5 ms is **not** transport (REST is 131 µs) and **not** the client
(15 µs) — the single monoio reactor parks the per-connection handler in a
10 ms `readable()` timeout while the casting-recv loop delivers the
response (`me_out → gateway_cast_recv` is essentially the whole gap):
**egress poll-loop starvation, not the wire** (documented in
`20260530_e2e-ws-probe.md` and `20260530_gateway-ws-rest-latency.md`).
Presented openly: this is the baseline for the planned egress-tile-split
(pinned busy-spin casting-recv → SPSC → WS writers), **an open bug, not a
win to hide** — and the naive thread-per-conn client (15 µs) currently
**beats** the shared-reactor egress, the "vs the obvious thing"
comparison the gateway presently *loses* (see the naive baseline below).

## One-paragraph framing

- **Nasdaq INET** — the on-path gold standard: 14–40 µs binary OUCH over
  kernel-bypass/FPGA. Different protocol class and I/O layer; universe
  check only, and the fair RSX line is the 7.5 µs floor, never 11.5 ms.
- **Crypto exchanges** — same *protocol class* (public WS), the most
  honest directional peer, but their 5–10 ms includes their *entire*
  matching engine; our 11.5 ms is *only* gateway egress scheduling.
- **MigratoryData / C10M** — the connection-scaling ceiling for the
  ARCHITECTURE's "io_uring for 100k+ connections" claim, which is a
  *design rationale, not yet measured* (WS bench maxes ~100 conns,
  slab-bounded). Aspirational until a scaling harness exists.

## What we could actually build in-repo

The gateway already has the most benches of the three (`gateway_bench`,
`ws_parse_bench`, `ws_encode_bench`, `jwt_validate_bench`,
`jti_tracker_bench`, `ws_order_latency`). The honest gaps:

- **Ingress-only service-time Criterion (FITS Criterion — CPU-bound):**
  frame bytes → parse → validate → rate-limit → `send_raw` into a
  `/dev/null` UDP sink. Reports the true sub-µs alloc-free ingress cost
  in isolation, cleanly separated from the egress bug — composes the
  existing `ws_parse` + `jwt_validate` + `convert` benches into one path.
- **Concurrent-WS scaling harness (does NOT fit Criterion — open-loop,
  I/O):** extend `ws_order_latency.rs` to sweep connection count
  (1 → 100 → 1k → 10k), reporting accept+handshake rate, per-conn RSS,
  and egress latency distribution. This is what exposes / regresses the
  reactor-starvation bug and would validate the tile-split when it lands.
- **Naive baseline (mirrors `compare_naive_bench.rs`):** the existing
  thread-per-connection blocking WS client (15 µs) IS the honest
  baseline — no egress starvation, which is exactly why it beats the
  shared reactor's 11.5 ms today.
- **Cannot measure single-box:** real WAN client RTT, NIC IRQ coalescing,
  DPDK/AF_XDP swap (future). Loopback removes the wire.

## Traps

- **THE trap: quoting 11.5 ms as "gateway latency".** It is a known
  egress scheduling bug, not the floor — ingress is sub-µs, the
  in-process round-trip is 7.5 µs. Any external comparison must state
  which number and why they differ.
- **Protocol/I/O-layer mismatch.** Nasdaq's 14–40 µs is binary OUCH over
  kernel-bypass/FPGA; rsx-gateway is WS-JSON over monoio/io_uring.
  Directional universe check only, never a ratio.
- **What's-in-the-number mismatch.** Crypto's 5–10 ms includes their
  entire matching engine; our 11.5 ms is only gateway egress scheduling.
  Same magnitude, completely different cause.
- **Loopback vs WAN.** All RSX gateway numbers are single-box loopback;
  crypto 5–10 ms includes internet + full stack. Don't cross them.
- **Connection-scaling is a claim, not yet measured.** The "io_uring for
  100k+ connections" rationale has no 10k-conn run yet (WS bench maxes
  ~100 conns, slab-bounded). Label it aspirational.
