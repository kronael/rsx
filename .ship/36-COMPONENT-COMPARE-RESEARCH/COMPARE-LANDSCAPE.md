# Component comparison landscape — recorder, marketdata, gateway

Companion to the two existing exemplars, written in the same honesty
discipline:

- `rsx-book`: `reports/20260704_book-bench.md` +
  `.ship/34-COMPARE-RESEARCH/MATCHING-ALTERNATIVES.md` +
  `rsx-book/benches/compare_naive_bench.rs` (a same-box naive baseline).
- `rsx-cast`: `rsx-cast/compare/` (per-protocol `.md` + `compare_*` benches,
  every number labelled loopback / off-box / cited-only).

Those two got the full treatment: a `compare/` census of the wider field, a
same-box head-to-head against a naive baseline, and a dated `reports/` run with
per-op p50/p99 + caveats. This document assembles the raw material so the same
treatment can be built for the other three RSX processes. It does **not**
propose editing any code (and `rsx-cast` is frozen regardless).

---

## Honest-comparison rules (read first)

Carried over verbatim in spirit from `MATCHING-ALTERNATIVES.md` and
`rsx-cast/compare/README.md`. A published number is only a **head-to-head** if
it is *same-op + same-depth/payload + same-hardware + same-language*. Everything
else is **directional context**, never "N× faster". Concretely, for these three
components:

1. **Label the axis.** recorder = durable-append throughput + durability lag;
   marketdata = book-update fan-out latency/throughput + snapshot cost; gateway
   = order ingress→cast service time + concurrent-WS scaling. A number on the
   wrong axis (e.g. matching latency quoted against fan-out) is disqualified.
2. **Label the path.** marketdata and recorder are **off** the GW→ME→GW
   critical path; gateway is **on** it. Never place an off-path latency next to
   an on-path one without saying so.
3. **Label the venue.** loopback single-box ≠ cross-DC ≠ multicast-over-NIC.
   Our numbers are single-box loopback; most exchange numbers are tuned
   kernel-bypass on isolated cores.
4. **Label the durability/delivery model.** per-record `fsync` (what the
   recorder does) is a different guarantee than page-cache-batched append
   (Kafka) or `tmpfs`-backed queue (Chronicle benchmark harness). Throughput is
   not comparable across durability models.
5. **Label lab-vs-production.** a Criterion microbench is service time on a
   warm cache, not system TPS under load. The gateway's own live figure
   (11.5 ms) vs its in-process floor (7.5 µs) is the canonical cautionary tale
   in this repo (see §3).
6. **"Cited only" vs "measured".** if we did not run it on our box, it is cited
   with a URL + the exact number + its stated HW/caveat, and it never becomes a
   ratio.

RSX in-repo reference points these three sit against (all
`reports/20260530_load-curves.md`, same 6-core Ryzen box):
one casting hop ≈ **3.8 µs** loopback; in-process GW→ME→GW round-trip
**7.5 µs p50 / 16.9 µs p99**; risk shard service ceiling **~8.5M orders/s**
(depth 8); ME match **~60 ns** depth-invariant.

---

## 1. rsx-recorder — archival WAL append consumer

### What it is
A single-purpose archival replication consumer (`rsx-recorder/src/main.rs`, one
`RecorderState` + one `rsx_cast::ReplicationConsumer`). It opens **one
long-lived TCP connection** to a matching engine's replication server, receives
the raw WAL record stream (`RawWalRecord`, header + payload, no transformation),
buffers into a 64 KB `Vec`, and flushes to a date-partitioned archive file
(`{stream_id}/{stream_id}_{date}.wal`). It rotates daily at UTC midnight and
persists a consumption tip for idempotent restart. Runtime is **tokio** — async
file I/O + one socket, no hot loop, no pinning — an explicit trade of latency
(TCP head-of-line blocking, kernel cwnd) for operational simplicity
(`rsx-recorder/ARCHITECTURE.md`).

### The one metric that matters
**Sustained durable-append throughput (records/s) and durability lag** — the
time from "record received on the socket" to "record `fsync`'d to the archive
file". The current design flushes with `file.sync_all()` (an actual fsync)
**every 1000 records** (`main.rs::flush`), so durability lag is bounded by
1000-record batches, and throughput is gated by fsync cost amortized over 1000
records + TCP delivery.

### Honest reference points

| System | Number | HW / caveat | Head-to-head? | Source |
|---|---|---|---|---|
| **Chronicle Queue** | sustained **5M msg/s**; 99%ile **3.69 µs @ 500k/s**; <10 µs 99.9%ile up to 1.4M/s | 2×12-core Xeon E5-2650 v4, **queues on `tmpfs`** (RAM-backed, not disk-durable) | no — tmpfs ≠ fsync; Java; IPC not TCP | [chronicle.software](https://chronicle.software/throughput-benchmarks-upto-5-million-messages-per-second/) |
| **Chronicle Queue** (persisted event) | **~660 ns/event** persisted | commercial, mmap-shared, single host | no — mmap IPC, different durability | [chronicle.software](https://chronicle.software/building-fast-trading-engines-chronicles-approach-to-low-latency-trading/) |
| **Aeron Archive** | records to disk **at full transport rate**; OSS >350k msg/s, Premium >3M msg/s | Archive is a separate process; rsx-cast *fuses* wire=disk | no — separate archive process, Java/C++ | [AWS blog](https://aws.amazon.com/blogs/industries/aeron-performance-enables-capital-markets-to-move-to-the-cloud-on-aws/) |
| **Kafka (LinkedIn)** | **2M writes/s** on 3 machines (2014) | commodity, **page-cache batched, no per-record fsync** | no — batched durability, cluster | [LinkedIn Eng](https://engineering.linkedin.com/kafka/benchmarking-apache-kafka-2-million-writes-second-three-cheap-machines) |
| **Kafka (tuned 3-broker)** | **~1.05M rec/s (100 MB/s)** | `batch.size=131072, linger.ms=10, lz4` | no — batched + compressed | [oneuptime](https://oneuptime.com/blog/post/2026-01-25-tune-kafka-million-messages-per-second/view) |

**The honest reading:** every one of these is a *different durability model*
than rsx-recorder's per-1000-record fsync-to-disk. Chronicle's 5M/s is on
`tmpfs` (RAM); Kafka's 1-2M/s is page-cache batched (durable only on later
flush); Aeron Archive is a dedicated process. The one property rsx-recorder has
that none of them advertise is **wire format = disk format = audit log, no
transformation** (the rsx-cast thesis). So the recorder is not competing on
raw MB/s; it is the trivially-simple end of a `ReplicationConsumer` that already
did the hard part (`rsx-cast`).

### What we could actually build in-repo
**A Criterion microbench does NOT fit** — this is I/O-bound (fsync + TCP), not
CPU-bound, and there are currently **zero benches** in `rsx-recorder/`. The
honest harness is a **throughput + durability-lag loop** (a `tests/`-style
binary or a `harness = false` bench that does its own timing, like
`risk_flood_bench`):

- Feed N synthetic `RawWalRecord`s straight into `RecorderState::write_record`
  (bypass TCP) → measure records/s and bytes/s to (a) a real disk, (b) `tmpfs`,
  to bracket the fsync cost the way Chronicle's tmpfs run does.
- **Durability-lag sweep:** vary the flush batch (per-record / per-100 /
  per-1000) and report p50/p99 lag = time from `write_record` return to fsync
  completion. This is the number that actually matters and no competitor
  publishes it comparably.
- **Naive baseline (mirrors `compare_naive_bench.rs`):** the same record stream
  written with a plain `BufWriter<File>` + `flush()` (no fsync, no rotation, no
  tip) — shows what the durability + rotation + tip machinery costs over a dumb
  append. That is the honest same-box "vs the obvious thing" line.
- **Cannot measure in this harness:** real TCP replication under loss/backpressure,
  cross-DC RTT, disk contention from a colocated ME. Those need the live cluster
  (`playground start-all`) and belong in a `reports/` run, not a bench.

### Traps
- **Durability-model mismatch (the big one).** Quoting Kafka's 2M/s or
  Chronicle's 5M/s next to rsx-recorder is dishonest unless you match the
  durability: their headline numbers are page-cache-batched / tmpfs, ours is
  per-1000-record fsync-to-disk. Compare fsync-to-fsync or say "different
  guarantee".
- **Single TCP consumer vs multicast archive.** Aeron Archive can tap a
  multicast stream; the recorder is one unicast TCP consumer. Different fan-in.
- **Off critical path.** The recorder's latency never enters the GW→ME→GW
  budget — never place its lag next to the 7.5 µs round-trip.

---

## 2. rsx-marketdata — shadow-book + L2/BBO/trade fan-out

### What it is
A separate process (`rsx-marketdata/`, ~2.4k LOC, single monoio/io_uring
reactor, `Rc<RefCell<MarketDataState>>`, no locks) that drains **one
`CastReceiver` per matching engine**, rebuilds a **shadow orderbook per symbol**
(`ShadowBook` wrapping `rsx_book::Orderbook` + an order-id→slab-handle map),
derives L2 depth deltas / BBO / trades, and **fans them out to public WS
subscribers** as JSON envelopes. It is derived-from-events (owns no
authoritative state), does per-symbol sequence-gap detection (gap → resend
snapshot), and is **off the GW→ME→GW critical path** (ME pushes fire-and-forget;
pinned only for keep-up, no `core_affinity`). Cold-start bootstraps from TCP
replication before going live.

### The one metric that matters
**Book-update fan-out latency and sustained throughput** — how fast one ME
event (`OrderInserted`/`Fill`/`Cancel`) becomes a delivered delta on *every*
subscribed WS client, and how many events/s one reactor sustains before
per-client backpressure forces snapshot-resync. Secondary: **depth-snapshot
cost** (the `derive_l2_snapshot(N)` sent on subscribe + on gap).

### Honest reference points

| System | Number | HW / caveat | Head-to-head? | Source |
|---|---|---|---|---|
| **Nasdaq TotalView-ITCH / OPRA** (feed *input* rate) | microbursts **40–75M msg/s**; Apr-2025 peak **>187M msg/s** (23.7M pkt/s over 1 ms) | whole US options tape, FPGA/kernel-bypass consumers | no — that is the *aggregate market*, not one symbol's book | [Databento](https://databento.com/blog/beyond-40-gbps-processing-opra-in-real-time), [Pico](https://www.pico.net/blog/opra-96-line-expansion-the-big-boost-in-latency-and-infrastructure-requirements/) |
| **ITCH book builder** | **~10.8M msg/s @ ~92 ns/msg**; 100M+ msg/s raw parse | single symbol, C, warm cache, no fan-out | directional — book-apply only, no WS fan-out | [aanrv/Order-Book](https://github.com/aanrv/Order-Book) |
| **Aeron** (fan-out transport) | IPC **~830 ns**; multicast one-to-many at NIC | shared-memory / multicast, not per-subscriber JSON | no — binary multicast ≠ per-client JSON clone | `rsx-cast/compare/README.md`, [AWS](https://aws.amazon.com/blogs/industries/aeron-on-aws-2025-performance-benchmark-results/) |
| **MigratoryData** (WS fan-out scale) | **10–12M concurrent connections** on 12 cores | Java/Linux, messaging not market data | directional — subscriber-scale ceiling only | [MigratoryData](https://migratorydata.com/blog/migratorydata-solved-the-c10m-problem/) |
| **Phoenix Channels** | **2M** WS subscribers, broadcast in **~1 s** | Elixir/BEAM chat, not HFT | directional — fan-out shape only | [josephmate](https://josephmate.github.io/2022-04-14-max-connections/) |

**RSX already measures the per-op half** (`rsx-marketdata/benches/marketdata_bench.rs`,
`shadow_book_apply_bench.rs`): shadow-book insert **<500 ns**, `derive_bbo`
**<100 ns**, `l2_snapshot` 10-level **<1 µs** / 50-level **<5 µs**, delta-gen
**<200 ns**, single-book event throughput **>100k events/s**. What is **missing**
is the fan-out axis: the cost of cloning + queueing one delta across *K*
subscribers (the three documented `msg.clone()` sites in
`main.rs::broadcast_updates` / `handle_fill`) and the sustained events/s before
`RSX_MD_MAX_OUTBOUND` overflow triggers snapshot-resync.

### What we could actually build in-repo
Per-op is done; the honest new work is a **fan-out saturation harness** (again a
`harness=false` self-timed bench, not pure Criterion, because it is
loopback-I/O-bound):

- **Fan-out cost sweep:** hold a populated `ShadowBook`, register K synthetic
  subscribers (K = 1, 10, 100, 1k) in a `SubscriptionManager`, drive a fixed
  delta stream through `broadcast_updates`, and report per-subscriber enqueue
  latency + total events/s as K grows. Isolates the per-K `String` clone cost
  (the JSON-broadcast tax the ARCHITECTURE calls out).
- **Loopback WS delivery:** K real loopback WS clients (reuse the gateway
  bench's hand-rolled blocking WS client, `20260530_gateway-ws-rest-latency.md`),
  measure event→delivered p50/p99 and the events/s at which backpressure starts
  resyncing. This is the honest "fan-out latency" number.
- **Naive baseline (mirrors `compare_naive_bench.rs`):** snapshot-every-update
  vs delta+BBO-dedup — quantifies what the shadow-book/delta machinery buys over
  a dumb "resend the whole top-N book on every event" disseminator.
- **Cannot measure single-box:** true multicast fan-out (one NIC write, switch
  replicates) — our model is per-subscriber unicast JSON, fundamentally
  different from Aeron/OPRA multicast. Say so; don't fake it.

### Traps
- **Fan-out model mismatch.** OPRA/Aeron replicate one binary frame at the
  NIC/switch to N receivers; marketdata clones a JSON `String` per subscriber in
  userspace. Comparing our per-subscriber cost to their per-frame cost is
  apples-to-oranges — different by design (JSON public feed per spec).
- **Input-rate mismatch.** 187M msg/s is the *entire US options tape*; one RSX
  ME emits one symbol's events. Never imply we ingest OPRA rates.
- **Off critical path.** Marketdata latency is fire-and-forget downstream of ME;
  it must never be quoted next to the GW→ME→GW 7.5 µs. Its own budget is
  "keep up with the ME firehose without UDP `RcvbufErrors`", not round-trip µs.
- **Per-op ≠ system.** The <500 ns shadow-book insert is warm-cache service
  time for one book; it is not "marketdata does 2M updates/s to 1000 clients".

---

## 3. rsx-gateway — WS order ingress + cast to Risk

### What it is
The client-facing WS/REST edge (`rsx-gateway/`, ~3.6k LOC, single monoio/io_uring
reactor, `Rc<RefCell<GatewayState>>`). Per connection: reads a WS frame (≤4 KB),
UTF-8 + JSON parse, JWT auth (HS256 + jti-replay), two-layer token-bucket rate
limit (per-IP + per-user), circuit breaker, symbol/tick/lot validation, mints a
UUIDv7 oid, and forwards a stack-struct `OrderRequest` to Risk via
`CastSender::send_raw` (**allocation-free** binary path). The return path decodes
Fill/OrderInserted/… off the casting/UDP recv loop and fans out JSON to the
client's per-connection `VecDeque<String>`. It is **on the GW→ME→GW <50 µs
critical path** and holds no durable state (crash = clients reconnect; Risk+WAL
are authoritative).

### The one metric that matters
**Order ingress→cast service time** (frame-in → `send_raw` to Risk) and
**concurrent-WS-connection scaling** (accept + handshake + per-conn egress
latency as connection count climbs). The ingress path is sub-µs and alloc-free;
the *egress* path is where the known problem lives.

### Honest reference points

| System | Number | HW / caveat | Head-to-head? | Source |
|---|---|---|---|---|
| **Nasdaq INET** | round-trip **<40 µs** wire-to-wire; **37 µs** at network boundary (OUCH/10GbE); fastest prod **14 µs** door-to-door; OrderAck emitted inline on receipt | kernel-bypass, user-space net, FPGA-assisted, binary OUCH | no — DPDK/FPGA + binary protocol vs our monoio + WS-JSON | [A-Team](https://a-teaminsight.com/blog/with-six-rollout-nasdaq-omx-pushes-matching-latency-below-40-microseconds/) |
| **arxiv "World's Fastest ME"** (2026) | p50 **376 ns** / p99 **524 ns @ 5M msg/s** | AWS r8g.metal (ARM Graviton4), unreleased | no — full ack path, ARM, not our box | [arxiv 2606.01183](https://arxiv.org/html/2606.01183v1) |
| **Crypto exchanges** (Coinbase/Kraken/Binance) | exchange processing **5–10 ms**; WS transport <50 ms gw→client | production internet-facing, includes full matching | directional — same *protocol class* (public WS), full-stack | [CoinAPI](https://www.coinapi.io/blog/crypto-trading-latency-guide) |
| **AWS "tick-to-trade" digital assets** | **sub-ms** end-to-end under realistic load | optimized cloud arch, tuned | directional — target we are in-universe with (7.5 µs floor) | [AWS](https://aws.amazon.com/blogs/web3/optimize-tick-to-trade-latency-for-digital-assets-exchanges-and-trading-platforms-on-aws/) |
| **C10M / MigratoryData** | **10–12M** concurrent conns / 12 cores; WhatsApp 2M/24 cores | connection-scaling context, not order latency | directional — the io_uring `for 100k+ conns` claim's ceiling | [MigratoryData](https://migratorydata.com/blog/migratorydata-solved-the-c10m-problem/), [C10k wiki](https://en.wikipedia.org/wiki/C10k_problem) |

**RSX in-repo (`reports/20260530_*`):** in-process GW→ME→GW round-trip
**7.5 µs p50 / 16.9 µs p99** (the transport+compute floor); REST `/health` over
the same gateway **131 µs**; hand-rolled WS client floor **15 µs**; **but** live
single-warmed-WS order round-trip **11.5 ms p50** — a **known bug**
(`GATEWAY-LATENCY`, both `20260530_e2e-ws-probe.md` and
`20260530_gateway-ws-rest-latency.md`): the single monoio reactor parks the
per-connection handler in a 10 ms `readable()` timeout while the casting-recv
loop delivers the response — **egress poll-loop starvation, not the wire**
(`me_out → gateway_cast_recv` is essentially the whole 11 ms gap). This is the
baseline for the planned egress-tile-split (pinned busy-spin casting-recv →
SPSC → WS writers; shard reactors).

### What we could actually build in-repo
The gateway already has the most benches of the three
(`gateway_bench`, `ws_parse_bench`, `ws_encode_bench`, `jwt_validate_bench`,
`jti_tracker_bench`, `ws_order_latency`). The honest gaps:

- **Ingress-only service-time Criterion (this one FITS Criterion — it is
  CPU-bound):** frame bytes → parse → validate → rate-limit → `send_raw` into a
  `/dev/null` UDP sink. Reports the true sub-µs cost of the alloc-free ingress
  path in isolation, cleanly separated from the egress bug. Composes the
  existing `ws_parse` + `jwt_validate` + `convert` benches into one path number.
- **Concurrent-WS scaling harness (does NOT fit Criterion — open-loop, I/O):**
  extend `ws_order_latency.rs` to sweep connection count (1 → 100 → 1k → 10k),
  reporting accept+handshake rate, per-conn RSS, and egress latency
  distribution. This is what actually exposes / regresses the reactor-starvation
  bug and would validate the tile-split when it lands.
- **Naive baseline (mirrors `compare_naive_bench.rs`):** the existing
  hand-rolled **thread-per-connection blocking WS client floor (15 µs)** IS the
  honest baseline — one blocking thread per socket has no egress starvation,
  which is exactly why it is 15 µs while the shared monoio reactor is 11.5 ms.
  Framing the monoio single-reactor vs thread-per-conn is the "vs the obvious
  thing" comparison, and it currently *loses* on egress (a real, documented
  finding, not a win to hide).
- **Cannot measure single-box:** real WAN client RTT, real NIC IRQ coalescing,
  DPDK/AF_XDP swap (future). Loopback removes the wire.

### Traps
- **THE trap: quoting 11.5 ms as "gateway latency".** It is a *known egress
  scheduling bug*, not the gateway's floor — the ingress path is sub-µs and the
  in-process round-trip is 7.5 µs. Any external comparison must state which
  number and why they differ (see rule 5). This is the single most dishonest
  thing one could do with these components.
- **Protocol/I/O-layer mismatch.** Nasdaq's 14–40 µs is binary OUCH over
  kernel-bypass/FPGA; rsx-gateway is WS-JSON over monoio/io_uring. Different
  protocol, different I/O layer — directional universe check only, never a ratio.
- **What's-in-the-number mismatch.** Crypto's 5–10 ms includes their *entire*
  matching engine; our 11.5 ms is *only* gateway egress scheduling (ME+risk are
  sub-ms within it). Same magnitude, completely different cause.
- **Loopback vs WAN.** All RSX gateway numbers are single-box loopback; crypto
  5–10 ms includes internet + their full stack. Don't cross them.
- **Connection-scaling is a claim, not yet measured.** The ARCHITECTURE's
  "io_uring for 100k+ connections" is a design rationale; the repo has no
  10k-connection scaling run yet (the WS bench maxes ~100 conns, slab-bounded).
  Label it aspirational until the scaling harness above exists.

---

## Summary — the reference numbers, sourced

1. **Chronicle Queue: 5M msg/s sustained, 99%ile 3.69 µs @500k/s** (2×12-core
   Xeon, **tmpfs** — RAM, not fsync). recorder peer, different durability.
   [chronicle.software](https://chronicle.software/throughput-benchmarks-upto-5-million-messages-per-second/)
2. **Kafka: 2M writes/s / 3 machines** (page-cache batched, not per-record
   fsync). recorder peer, different durability.
   [LinkedIn](https://engineering.linkedin.com/kafka/benchmarking-apache-kafka-2-million-writes-second-three-cheap-machines)
3. **Aeron: OSS >350k, Premium >3M msg/s; Archive records at full transport
   rate; IPC ~830 ns.** recorder + marketdata fan-out peer.
   [AWS](https://aws.amazon.com/blogs/industries/aeron-performance-enables-capital-markets-to-move-to-the-cloud-on-aws/)
4. **OPRA/ITCH feed peak: 40–75M msg/s, Apr-2025 burst >187M msg/s.** marketdata
   *input-rate* context (whole US options tape, not one symbol).
   [Databento](https://databento.com/blog/beyond-40-gbps-processing-opra-in-real-time)
5. **ITCH book builder: ~10.8M msg/s @ ~92 ns/msg** (single symbol, C).
   marketdata book-apply directional peer.
   [aanrv/Order-Book](https://github.com/aanrv/Order-Book)
6. **Nasdaq INET: <40 µs round-trip, 37 µs at boundary (OUCH/10GbE), 14 µs
   fastest.** gateway on-path peer (kernel-bypass/binary — universe check only).
   [A-Team](https://a-teaminsight.com/blog/with-six-rollout-nasdaq-omx-pushes-matching-latency-below-40-microseconds/)
7. **Crypto exchanges: 5–10 ms exchange processing, WS <50 ms gw→client.**
   gateway same-protocol-class (public WS) directional peer.
   [CoinAPI](https://www.coinapi.io/blog/crypto-trading-latency-guide)
8. **C10M: MigratoryData 10–12M conns/12 cores; Phoenix 2M WS broadcast ~1 s.**
   gateway + marketdata connection-scaling ceiling context.
   [MigratoryData](https://migratorydata.com/blog/migratorydata-solved-the-c10m-problem/)
9. **RSX internal anchors** (`reports/20260530_load-curves.md`): casting hop
   **3.8 µs**, in-process GW→ME→GW **7.5 µs p50**, gateway live WS **11.5 ms**
   (**known egress-starvation bug**, not the floor), risk ceiling **~8.5M ord/s**.
10. **Bench feasibility:** gateway ingress = Criterion-fits (CPU-bound); recorder
    throughput/lag, marketdata fan-out, gateway WS-scaling = **do NOT fit
    Criterion** (I/O-bound) → self-timed throughput/saturation harnesses +
    live-cluster `reports/` runs, each with a naive same-box baseline mirroring
    `rsx-book/benches/compare_naive_bench.rs`.
