# Benchmark Index

Every measurement program in the repo: what it measures,
which production code path it isolates, and how to run it.
Cross-reference: `.ship/18-COMPONENT-BENCHES/LANDSCAPE.md`
has the most recent numbers + attribution against the
production GW→ME→GW p50.

## How to run

```
# One Criterion bench
cargo bench --bench <bench_name>

# Quick smoke (10 samples, 3s measurement):
cargo bench --bench <bench_name> -- --sample-size 10 \
  --warm-up-time 1 --measurement-time 3

# All Criterion benches (a few minutes):
cargo bench --workspace

# In-process matching round-trip binary (not Criterion):
cargo build --release --bin bench-match-rt
./target/release/bench-match-rt --n 10000 --warmup 500
```

Criterion writes per-bench results to
`target/criterion/<bench>/` (HTML + JSON). The bench-match-rt
binary prints to stdout.

## Structure

Each crate owns its `benches/` directory. Two families:

| Family | Purpose | Example |
|---|---|---|
| **Component isolation** | One leg of the production path in tight loop, real production code | `cast_one_way_bench`, `bench-match-rt` |
| **Micro-op** | A single primitive in isolation | `crc32_compute_128b`, `price_add` |

Component-isolation benches were added in `.ship/18-COMPONENT-BENCHES`
(2026-05-22 → 2026-05-23) and are the load-bearing ones for
latency attribution. Micro-op benches are older; they're
useful for regression detection on specific primitives but
don't compose to a production p50 on their own.

## Per-crate index

### rsx-types (foundation: primitive ops)

| Bench | Measures | Notes |
|---|---|---|
| `types_bench` | `Price` add, `Price * Qty`, order validation, `notional` mul (i128 vs checked) | Pure CPU, no IO |

### rsx-messages (wire records)

| Bench | Measures | Notes |
|---|---|---|
| `encode_bench` | `FillRecord` encode/decode, CRC32C over 128 B, `WalHeader` encode/decode | The `fill_record_encode` (23 ns) + `header_encode` (3 ns) appear in CHANGELOG; also see `cast_send_breakdown_bench` for the same numbers in send-path context |

### rsx-cast (transport: WAL + casting + UDP)

| Bench | Measures | Production leg it isolates |
|---|---|---|
| `compare_udp` | Raw UDP loopback round-trip, 64-byte payload, two non-blocking sockets spinning. **Absolute floor.** | none — baseline |
| `cast_one_way_bench` | `CastSender::send` → `CastReceiver::try_recv` one direction | `gateway_in → risk_in`, `risk_out → gateway_cast_recv` |
| `cast_rtt_bench` | casting echo round-trip (A → B → A) with two pairs | the full `risk → ME → risk` triangle |
| `cast_send_breakdown_bench` | Each step inside `CastSender::send` separately: CRC, header build, buf pack, sendto, NAK ring copy | Attributes the 3.9 µs `send` body — **99 % is sendto** |
| `wal_bench` | `WalWriter::prepare` + `append_framed` in-memory, flush+fsync 64 KB, sequential read 10 K, replay 100 K | Pre-fsync append is 31 ns; fsync is in the fsync-specific bench below |
| `wal_fsync_bench` | `WalWriter::prepare` + `append_framed` + explicit flush + fsync to disk | Durability cost — **651 µs p50**, 20 000× higher than the in-memory append |
| `wal_random_read_bench` | `read_record_at_seq(random)` over a pre-populated WAL | Cold-tier NAK retransmit path; O(n) at 23.5 ms @ 10 K records |
| `cast_bench` | Protocol record encode/decode (NAK, Heartbeat) | Wire-level primitives only — not on the per-packet send path |

### rsx-book (orderbook)

| Bench | Measures | Notes |
|---|---|---|
| `book_bench` | slab alloc/free, `CompressionMap::price_to_index` near + far, `CompressionMap::new`, single-order insert + match + cancel | The "54 ns single fill" Criterion bench lives here; that's the **inner match only**, surrounding plumbing adds ~3 µs in production (see `process_order_bench`) |

### rsx-matching (matching engine plumbing)

| Bench | Measures | Production leg it isolates |
|---|---|---|
| `process_order_bench` | Full `me_in → me_out` cycle: dedup + WAL accept + `process_new_order` + `write_events_to_wal` + index update | `me_in → me_out` in production (the 158 µs leg). Production now goes through `publish_events` (one-CRC fan-out) instead; the bench uses `write_events_to_wal` directly because it isolates the WAL leg without the cast send-path noise. |
| `match_n_levels_bench` | One incoming order sweeping 1, 5, 20, 100 resting levels | Algorithmic complexity check; n=1 here is 6.8 µs (includes book setup), pure-match is 54 ns |
| `wal_replay_bench` | `WalReader::next` over a pre-written 30 K-record WAL | Cold-start recovery / snapshot reload — 228 ms @ 30 K records |
| `matching_bench` | dedup duplicate check, single-slot alloc/free, cancel | Hot-path primitives |

### rsx-gateway (WS ingress + JWT)

| Bench | Measures | Production leg it isolates |
|---|---|---|
| `ws_parse_bench` | `parse()` of N (new-order), C (cancel), Heartbeat JSON frames | `gw_in` after WS frame read |
| `ws_encode_bench` | `serialize(WsFrame::Fill / OrderUpdate / Heartbeat)` | `gw_out` before WS write |
| `jwt_validate_bench` | `validate_jwt_with_claims` with and without `jti` | One-shot per WS handshake (~6 µs); amortized away for order flow |
| `jti_tracker_bench` | `JtiTracker::record` steady state + duplicate | Per-handshake replay defence — sub-µs |
| `gateway_bench` | UUID v7 generation, rate-limit check, fee extraction, frame parse/serialize, backpressure reject | Pre-existing micro-ops — overlaps with the per-stage benches above |

### rsx-marketdata (shadow book)

| Bench | Measures | Notes |
|---|---|---|
| `shadow_book_apply_bench` | `apply_insert_by_id`, `apply_fill`, `apply_cancel`, `apply_insert_then_bbo` paths in real `ShadowBook` | The marketdata consumer side; sub-µs |
| `marketdata_bench` | Inserts/fills + BBO derivation + L2 snapshot at 10/50 levels + L2 delta gen + WS serialize BBO + sustained event throughput | Pre-existing broader-coverage bench |

### rsx-mark (mark price aggregator)

| Bench | Measures | Notes |
|---|---|---|
| `aggregator_bench` | Binance + Coinbase tick aggregation (2-source, 5-source) | 78 ns — essentially free |
| `mark_bench` | Source-to-publish end-to-end inside the aggregator, source-mask computation | Pre-existing |

### rsx-risk (risk shard)

| Bench | Measures | Notes |
|---|---|---|
| `validate_bench` | Risk shard order validation (margin check) in isolation | 281 ns; risk's algorithm is essentially free, the leg's cost is plumbing |
| `risk_bench` | Index price calculation, pre-trade check latency, BBO processing, liquidation enqueue, round escalation | Pre-existing broader bench |

### rsx-cli (binary; not Criterion)

| Program | Measures | How to run |
|---|---|---|
| `bench-probe` | E2E probe via Python aiohttp WS — measures the full GW→ME→GW round-trip from outside | `./target/release/bench-probe` (needs live cluster) |
| `bench-match-rt` | **In-process matching round-trip** with per-stage timing. Single binary, two threads, real casting + Orderbook + WAL. The algorithmic floor. | `./target/release/bench-match-rt --n 10000 --warmup 500` |

## Most informative single number

| Layer | p50 | p99 |
|---|---:|---:|
| Matching algorithm only (dedup + match + WAL) | **~410 ns** | — |
| In-process round-trip (`bench-match-rt`, `--n 20000`) | **7.82 µs** | **22.3 µs** |
| Cross-process production (`SPEED-OFFHOT.md`) | 1 128 µs | — |
| Cross-process via Python probe (`bench-baseline.json`) | 11 878 µs | — |

Fresh `bench-match-rt` (this box, `--n 20000 --warmup 1000`) per stage p50/p99 ns:
`gw_send` 3196/5480, `me_dedup` 70/1623, `me_wal_accept` 110/2104, `me_match`
110/401, `me_wal_events` 120/2896, `me_send` 3276/11011, TOTAL 7824/22291. The
two `~3.2 µs` send legs are `std::net::UdpSocket` sendto (bench uses std, not
monoio); the compute stages sum to ~410 ns.

`bench-match-rt` is the load-bearing measurement: it
exercises every real production code path **except** the
inter-process boundaries (separate processes, monoio's
100 µs sleep, tokio reactor schedule, PG write-behind).
Its 7.82 µs floor vs the cross-process 1 128 µs tells us
**~99 % of production latency is inter-process overhead**,
not algorithm or transport framing.

## CastSender::send sub-attribution (`cast_send_breakdown_bench`)

Per `dfe2ef4`:

| Sub-step | p50 |
|---|---:|
| `crc32_128b` | 16 ns |
| `header_build` | 4.4 ns |
| `buf_pack_144b` (two memcpys → buf) | 4.4 ns |
| **`sendto_144b_loopback`** | **3 846 ns** ← 99 % |
| `ring_cache_copy_144b` | 3 ns |
| **Sum** | **3 874 ns** ≈ bench-match-rt `gw_send` |

If you eliminated **every line of Rust code in `CastSender::send`**,
you'd save 28 ns out of 3 874 ns — **0.7 % improvement**.
The remaining 99.3 % is the `sendto` syscall, which is
kernel code we don't own. To reduce it: io_uring SQE
submission (gateway production already does this; the bench
uses `std::net::UdpSocket` and overstates by ~2 µs), or
sendmmsg batching, or kernel bypass (DPDK/AF_XDP).

## Caveats and gotchas

- **Legacy quirk: casting flow control closed around iter
  65 536 without periodic `tick()`.** `cast_one_way` and
  `cast_rtt` were silently hanging for the first iteration of
  these benches until we added `tick()` every 1024 sends. The
  StatusMessage/flow-control path was removed in `87b223e`
  (and `CastReceiver::tick` no-op removed in `dc2a6a6`), so
  modern benches do not need this workaround. The bench source
  documents this. `CastSender::tick()` is still called from
  production sender loops to emit idle-stream heartbeats.
- **Both sockets binding the same port + `SO_REUSEPORT`
  hash-distributes incoming traffic.** `bench-match-rt`
  hit this: gw_sender and gw_receiver originally shared
  `gw_bind`; ME's replies landed on the sender socket half
  the time and vanished. Each CastSender / CastReceiver
  needs its own port (`18cfb00`).
- **`set_read_timeout` setsockopt inside a hot loop adds
  ~µs per iteration.** Why udp_rtt was reading as 29 µs
  for weeks before the fix in `245bd03`.
- **Criterion includes setup cost in `iter_batched` low-n
  cases.** `match_n_levels n=1` showed 6.8 µs (most of
  which is `Orderbook::new`). The "real" pure-match cost
  is the 54 ns single-fill bench in `book_bench`. Use
  `process_order_bench` (3.5 µs) for the true ME critical
  section cost.
- **`bench-match-rt` is `std::net::UdpSocket`, not monoio.**
  So `gw_send` 3.77 µs is the classical-sendto cost.
  Production gateway uses monoio (io_uring) and should be
  meaningfully cheaper. To get the io_uring number we'd
  need to port `CastSender` itself to monoio — logged as a
  deferred refactor.
- **WAL fsync 651 µs is amortised in production**: the
  writer flushes every 10 ms, not per record. As long as
  ≥ 10 orders share one fsync, per-order cost is ≪ 651 µs.

## Where the numbers live

| Doc | Purpose |
|---|---|
| `docs/benches.md` (this file) | What each bench measures, how to run |
| `.ship/18-COMPONENT-BENCHES/LANDSCAPE.md` | Current numbers + production-leg attribution |
| `.ship/19-SLEEP-AUDIT/SLEEPS.md` | Sleep / timeout audit (57 sites, 2 bugs) |
| `bench-baseline.json` | Rolling Criterion numbers (per `make perf`) |
| `bench-reference.json` | Sealed reference for the CI bench gate |
| `CHANGELOG.md` | Historical numbers cited as part of releases |

## Adding a new bench

1. New `.rs` file under `<crate>/benches/`. Use the
   `cast_one_way_bench.rs` shape as a template (docstring
   first, then bench).
2. Wire it in `<crate>/Cargo.toml`:
   ```toml
   [[bench]]
   name = "your_bench"
   harness = false
   ```
3. Document it: header docstring + a row in this file.
4. Mention it in `.ship/18-COMPONENT-BENCHES/LANDSCAPE.md`
   if it's a component-isolation bench (one that
   attributes to a production leg).
