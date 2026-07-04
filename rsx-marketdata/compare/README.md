# rsx-marketdata: fan-out / book-builder alternatives — the wider field

Companion to `rsx-cast/compare/` and `.ship/34-COMPARE-RESEARCH/`.
This is the `niche.md`-style census for the market-data process: where
it sits against ITCH book-builders, multicast fan-out transports, and
the OPRA/ITCH tape rates — and, crucially, which of **two separate
axes** each cited number lives on.

**Read the fairness rule first.** A number is a head-to-head only if it
is *same-op + same-fan-out-model + same-hardware + same-language*.
There are two axes here and they must never be crossed:

- **(a) book-apply per-op** — cost to fold one ME event into the shadow
  book. RSX **already benches this**.
- **(b) K-subscriber fan-out** — cost to deliver one derived delta to
  *every* subscribed WS client. RSX does **NOT** bench this yet.

And the honesty trap to name every time: **marketdata is OFF the
GW→ME→GW critical path.** ME pushes fire-and-forget; marketdata is
pinned only to keep up with the firehose (no `core_affinity`). Its
budget is "drain the ME casting stream without UDP `RcvbufErrors`", not
round-trip µs. **Never** compare its latency against an on-path
matching or gateway number.

## What rsx-marketdata is

A separate process (`rsx-marketdata/`, ~2.4k LOC, single monoio/io_uring
reactor, `Rc<RefCell<MarketDataState>>`, no locks). It drains **one
`CastReceiver` per matching engine**, rebuilds a **shadow orderbook per
symbol** (`ShadowBook` wrapping `rsx_book::Orderbook` + an
order-id→slab-handle map), derives L2 depth deltas / BBO / trades, and
**fans them out to public WS subscribers** as JSON envelopes. It is
derived-from-events (owns no authoritative state), does per-symbol
sequence-gap detection (gap → resend snapshot), and cold-starts from
TCP replication before going live.

## The one metric that matters

**Book-update fan-out latency + sustained throughput** — how fast one ME
event (`OrderInserted`/`Fill`/`Cancel`) becomes a delivered delta on
*every* subscribed WS client, and how many events/s one reactor
sustains before per-client backpressure forces a snapshot resync.
Secondary: **depth-snapshot cost** (`derive_l2_snapshot(N)` sent on
subscribe and on gap).

## Honest reference points

| System | Number | HW / axis caveat | Head-to-head? | Source |
|---|---|---|---|---|
| **Nasdaq TotalView-ITCH / OPRA** (feed *input* rate) | microbursts **40–75M msg/s**; Apr-2025 peak **>187M msg/s** (23.7M pkt/s over 1 ms) | whole US options tape; FPGA/kernel-bypass consumers | no — that is the *aggregate market*, not one symbol's book | [Databento](https://databento.com/blog/beyond-40-gbps-processing-opra-in-real-time), [Pico](https://www.pico.net/blog/opra-96-line-expansion-the-big-boost-in-latency-and-infrastructure-requirements/) |
| **ITCH book builder** (aanrv) | **~10.8M msg/s @ ~92 ns/msg**; 100M+ msg/s raw parse | single symbol, C, warm cache, **no fan-out** | directional (axis a) — book-apply only, no WS clone | [aanrv/Order-Book](https://github.com/aanrv/Order-Book) |
| **charles-cooper/itch-order-book** | **61 ns/tick, 16M msgs/s** | C++, flat arrays, 2012 i7, warm cache, single symbol | directional (axis a) — book-apply only | [repo](https://github.com/charles-cooper/itch-order-book) |
| **Aeron** (fan-out transport) | IPC **~830 ns**; multicast one-to-many at the NIC | shared-memory / multicast, **binary frame, not per-subscriber JSON** | no (axis b) — binary multicast ≠ per-client JSON clone | `rsx-cast/compare/README.md`, [AWS](https://aws.amazon.com/blogs/industries/aeron-on-aws-2025-performance-benchmark-results/) |
| **MigratoryData** (WS fan-out scale) | **10–12M concurrent connections** on 12 cores | Java/Linux, messaging not market data | directional (axis b) — subscriber-scale ceiling only | [MigratoryData](https://migratorydata.com/blog/migratorydata-solved-the-c10m-problem/) |
| **Phoenix Channels** | **2M** WS subscribers, broadcast in **~1 s** | Elixir/BEAM chat, not HFT | directional (axis b) — fan-out *shape* only | [josephmate](https://josephmate.github.io/2022-04-14-max-connections/) |

## Axis (a): per-op — RSX already measures this

`rsx-marketdata/benches/{marketdata_bench,shadow_book_apply_bench}.rs`
(same 6-core Ryzen box as `reports/20260530_load-curves.md`): shadow-book
insert **<500 ns**, `derive_bbo` **<100 ns**, `l2_snapshot` 10-level
**<1 µs** / 50-level **<5 µs**, delta-gen **<200 ns**, single-book event
throughput **>100k events/s**. These are the fair peers to the ITCH
book-builders' 61–92 ns/tick (different HW/language, warm cache — so
directional, not a ratio). **This half is done.**

## Axis (b): fan-out — the honest gap

What is **missing** is the K-subscriber axis: the cost of cloning +
queueing one delta across *K* subscribers (the documented `msg.clone()`
sites in `main.rs::broadcast_updates` / `handle_fill` — a JSON-broadcast
tax the ARCHITECTURE calls out) and the sustained events/s before
`RSX_MD_MAX_OUTBOUND` overflow triggers snapshot-resync. The honest new
harness (self-timed, not pure Criterion — it is loopback-I/O-bound),
mirroring `rsx-book/benches/compare_naive_bench.rs`:

- **Fan-out cost sweep:** hold a populated `ShadowBook`, register K
  synthetic subscribers (K = 1, 10, 100, 1k), drive a fixed delta stream
  through `broadcast_updates`, report per-subscriber enqueue latency +
  total events/s as K grows — isolates the per-K `String` clone cost.
- **Loopback WS delivery:** K real loopback WS clients (reuse the gateway
  bench's blocking WS client), measure event→delivered p50/p99 and the
  events/s at which backpressure starts resyncing. This is the honest
  "fan-out latency" number.
- **Naive baseline:** snapshot-every-update vs delta+BBO-dedup —
  quantifies what the shadow-book/delta machinery buys over a dumb
  "resend the whole top-N book on every event" disseminator.
- **Cannot measure single-box:** true multicast fan-out (one NIC write,
  switch replicates). Our model is per-subscriber unicast JSON —
  fundamentally different from Aeron/OPRA. Say so; don't fake it.

## One-paragraph framing

- **ITCH book builders (aanrv, charles-cooper)** — pure axis-(a) peers:
  parse + book-maintenance, one symbol, no fan-out. The fair rsx line
  next to their 61–92 ns/tick is `apply_*` (<500 ns), not any delivery
  number. Different HW/language — directional.
- **Aeron / OPRA multicast** — axis-(b) peers on a *different fan-out
  model*: replicate one binary frame at the NIC/switch to N receivers.
  marketdata clones a JSON `String` per subscriber in userspace (public
  JSON feed, per spec). Not comparable per-frame; different by design.
- **MigratoryData / Phoenix** — axis-(b) *scale ceiling* context: how
  many WS subscribers a single-box fan-out can hold. Directional target,
  not a latency head-to-head.

## Traps

- **Fan-out model mismatch.** OPRA/Aeron replicate one binary frame at
  the NIC; marketdata clones a JSON `String` per subscriber. Comparing
  our per-subscriber cost to their per-frame cost is apples-to-oranges.
- **Input-rate mismatch.** 187M msg/s is the *entire US options tape*;
  one RSX ME emits one symbol's events. Never imply we ingest OPRA rates.
- **Off the critical path.** marketdata is fire-and-forget downstream of
  ME — its latency must never sit next to the GW→ME→GW 7.5 µs.
- **Per-op ≠ system.** The <500 ns shadow-book insert is warm-cache
  service time for one book; it is not "2M updates/s to 1000 clients".
