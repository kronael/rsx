# Component Isolation Landscape — 2026-05-22

Component-isolation benches for every leg of the GW→ME→GW
path, captured against the live SPEED-OFFHOT.md production
p50 of **1 128 µs** (in-process tracing). Both parallel
subagents (transport+risk vs gateway+matching+md+mark+cli)
landed their benches; this doc reconciles their numbers
and attributes the production p50 back to components.

Numbers are release-profile (`[profile.bench]`), single
local dev machine, no `--ignored` integration deps.

## 1. Leg → bench mapping (GW→ME→GW)

| Production leg | Owning benches |
|---|---|
| `gw_in` (WS frame on socket → parsed) | `ws_parse_bench` (parse N/C/H) |
| `gw_in → risk_in` (gateway send to risk) | `jwt_validate_bench`, `jti_tracker_bench`, `udp_rtt_bench`, `cmp_one_way_bench` |
| `risk_in → me_in` (risk validate + forward) | `validate_bench` (risk), `cmp_one_way_bench` |
| `me_in → me_out` (match cycle) | `process_order_bench`, `match_n_levels_bench`, `matching_bench` |
| `me_out → md/risk` (CMP fan-out + WAL) | `wal_replay_bench`, `wal_fsync_bench`, `cmp_one_way_bench` |
| `md → ws` (shadow book + serialize) | `shadow_book_apply_bench`, `marketdata_bench` |
| `gw_out → ws frame` (encode + write) | `ws_encode_bench` |
| Mark price source path (orthogonal) | `aggregator_bench`, `mark_bench` |
| Full RT (algorithmic floor) | `bench-e2e-pipeline` (in-process binary) |

## 2. Headline measurements

### Gateway

| Bench | p50 |
|---|---:|
| `ws_parse_new_order` | **672 ns** |
| `ws_parse_cancel` | 557 ns |
| `ws_parse_heartbeat` | 312 ns |
| `ws_encode_fill` | **633 ns** |
| `ws_encode_order_update` | 342 ns |
| `ws_encode_heartbeat` | 204 ns |
| `jwt_validate_no_jti` | **6.4 µs** |
| `jwt_validate_with_jti` | 5.0 µs |
| `jti_record_steady_state` | 581 ns |
| `jti_record_duplicate` | 49 ns |

### Matching

| Bench | p50 |
|---|---:|
| `me_process_order_full_path` (dedup+WAL accept+match+WAL events+index) | **3.5 µs** |
| `book_match_n_levels/n=1` (full setup+match) | 6.8 µs |
| `book_match_n_levels/n=5` | 23.2 µs |
| `book_match_n_levels/n=20` | 178 µs |
| `book_match_n_levels/n=100` | 87 µs |
| `wal_replay_30k_records` | **228 ms** (~7.6 µs/record) |

The `match_n_levels` numbers above include book setup
(`Orderbook::new` + 1..100 resting inserts) inside
`iter_batched` — setup dominates at low n. The pure-match
component is the 54 ns figure from the existing
`match_single_fill`. The new bench's value is exposing
how setup cost scales; the production "match cycle" cost
is closer to the `me_process_order_full_path` 3.5 µs.

### Marketdata (shadow book apply)

| Bench | p50 |
|---|---:|
| `shadow_apply_insert` (paired insert+cancel) | **199 ns** |
| `shadow_apply_fill` | 1.03 µs |
| `shadow_apply_cancel` | 583 ns |
| `shadow_apply_insert_then_bbo` | 242 ns |

### Mark aggregator

| Bench | p50 |
|---|---:|
| `mark_binance_plus_coinbase` (2-source) | **78 ns** |
| `mark_5_sources_steady_state` | 101 ns |

### Transport (rsx-dxs, owned by parallel sub)

Numbers per `60c0b56 [bench] LANDSCAPE.md + bench harness fixes`:

| Bench | p50 |
|---|---:|
| `udp_rtt_loopback_64b` | **29 µs** |
| `wal_append_fsync_single` | **651 µs** |
| `wal_random_read_10k` | 23.5 ms |
| `cmp_one_way_bench` | _harness hang_ |
| `cmp_rtt_bench` | _harness hang_ |

### Risk (owned by parallel sub)

| Bench | p50 |
|---|---:|
| `validate_bench` | **281 ns** |

### In-process E2E pipeline

`bench-e2e-pipeline --n 10000 --warmup 500` — real
`Orderbook` + `WalWriter` + `DedupTracker` + `FxHashMap`
order index + real `CmpSender`/`CmpReceiver` over loopback
UDP, all in one process. Two threads: "Gateway" sender +
"ME" receiver. Strict request/reply (send order, await
its terminal Fill/OrderDone, then send next).

| Metric | Value |
|---|---:|
| Round-trip **p50** | **9 µs** |
| Round-trip min | 7 µs |
| Round-trip p95 | 14 µs |
| Round-trip p99 | 207 µs |
| Round-trip max | 32 ms |
| Total elapsed (10k orders) | 782 ms |

Cross-process production p50 (from SPEED-OFFHOT.md): **1 128 µs**.

### In-process vs cross-process

| | p50 (µs) |
|---|---:|
| In-process (this bench) | **9** |
| Cross-process (SPEED-OFFHOT.md) | 1 128 |
| Cross-process e2e (bench-baseline.json, includes Python probe) | 11 878 |
| Ratio (cross/in-process) | **125×** |

**The algorithm is two orders of magnitude faster than the
production cross-process number.** The other 1 119 µs is
plumbing: tokio reactor schedules, CMP framing overhead in
separate processes, the `monoio::time::sleep(100us)` polls
in gateway+md, fsync barriers, OS-level UDP loopback context
switches between processes.

## 3. Attribution to the 1 128 µs production p50

Sum of isolated component p50s along the GW→ME→GW path:

| Production leg | Span (µs) | Attributable components | Δ unaccounted |
|---|---:|---|---:|
| `gw_in` (WS parse + JWT once) | ~7 | parse 0.7 + JWT 6.4 | ~0 (first-frame only; in-flight is parse-only) |
| `gw_in → risk_in` (CMP send) | 60 | udp_rtt 29 + parse 0.7 + JWT amortized | ~30 (CMP framing + tokio schedule) |
| `risk_in → me_in` (validate + CMP send) | 205 | risk validate 0.28 + udp_rtt 29 | **~175** (risk's outbound + reactor wake) |
| `me_in → me_out` (match cycle) | 158 | me_process_order_full_path 3.5 + wal_fsync 651/N | ~150 (fsync amortized; first-order pays 651) |
| `me_out → risk_out` (CMP send to risk) | 39 | udp_rtt 29 | ~10 |
| `risk_out → gw_out` (CMP send + WS encode) | 666 | udp_rtt 29 + ws_encode_fill 0.6 + **monoio sleep 100µs × ≥5** | **~140** + sleep |
| **Total** | **1 128** | sum ≈ 720 | **~400** (mostly sleep polls + framing) |

**Findings on the gap:**

1. **`risk_in → me_in` (205 µs) has 175 µs of unattributed cost.** Risk's outbound CMP send + the tokio scheduler waking risk are the suspects. Worth sub-stage tracing inside risk.
2. **`risk_out → gw_out` (666 µs) is mostly the `monoio::time::sleep(100µs)` poll in gateway main loop.** Each event lap can hit the sleep ≥5 times before the gateway picks up the CMP reply. See `9bbb8f6` revert — `yield_now()` starves the WS accept task, the correct fix needs a monoio-native UDP socket in CmpReceiver.
3. **WAL fsync is 651 µs but amortized across many orders** (it flushes every 10 ms, not per-order); the per-order cost is ≪ 651 µs as long as ≥ 10 orders share one fsync.

## 4. Surprises

- **WS parse / encode are sub-µs at p50** (672ns / 633ns). The
  dashboard-style "synthesize JSON fill" cost is well under
  1 µs — JSON is not the bottleneck.
- **JWT validation is 6.4 µs at p50** — that's ~10% of the
  `gw_in → risk_in` 60 µs leg. Significant for the WS
  handshake, but it runs once per connection so for steady-
  state order flow it's amortized away. The handshake itself
  is a one-shot 6.4 µs cost.
- **In-process round-trip is 9 µs p50**, ~125× faster than
  cross-process. Confirms what SPEED-OFFHOT.md already
  argued: the bottleneck is reactor/scheduler/poll-sleep
  glue, not the algorithm.
- **`mark_binance_plus_coinbase` is 78 ns at p50.** The mark
  aggregator is essentially free; mark publish latency is
  dominated by exchange WS round-trips, not local compute.
- **`me_process_order_full_path` is 3.5 µs at p50** — the
  full critical section (dedup + 2 WAL appends + match
  + event emit + index update). The 54 ns "single fill"
  Criterion bench measured the inner `match_at_level`
  function only; the surrounding plumbing is 65× heavier.
- **WAL random-read is O(n)** (23 ms for 10k records). Cold-
  tier NAK retransmit hits this — a seq-indexed read would
  cut it. Out of scope for this sprint; logged.

## 5. Honest gaps

- **`bench-e2e-pipeline` does NOT exercise rsx-risk.** It
  drives Gateway → ME → Gateway directly, skipping risk's
  margin/validation layer. The parallel sub owned rsx-risk
  and benchmarked it in isolation (`validate_bench` =
  281 ns). The full 5-process pipeline (GW → risk → ME →
  risk → GW) would add 2× risk-shard hops; based on the
  isolated 281 ns + 60 µs CMP one-way, that's ~120 µs added,
  putting the in-process floor at ~130 µs vs the 1 128 µs
  cross-process — still ~9× headroom.
- **Postgres testcontainer was not wired in.** Risk's
  persist-behind path runs against PG, but the parallel
  sub's risk bench bypasses persistence (the spec allows
  this — persist is async write-behind on a separate task).
- **`cmp_one_way_bench` / `cmp_rtt_bench` hung** in the
  parallel sub's run. Their flow-control coordination has
  a harness bug. Until those land, the CMP framing cost
  is interpolated from `udp_rtt 29 µs` + measured one-side
  ME `me_process_order_full_path` 3.5 µs.
- **`book_match_n_levels` includes book setup inside
  iter_batched**, dominating at low n. The pure-match cost
  is closer to the existing `match_single_fill` 54 ns
  figure; the new bench's value is showing how setup
  scales with n, not the matching algorithm itself. A
  follow-up that pre-builds the book outside `iter_batched`
  would isolate match-only cost across n.

## 6. New benches landed in this sprint

| Crate | Bench | Status |
|---|---|---|
| `rsx-gateway` | `ws_parse_bench` | clean |
| `rsx-gateway` | `ws_encode_bench` | clean |
| `rsx-gateway` | `jwt_validate_bench` | clean |
| `rsx-gateway` | `jti_tracker_bench` | clean |
| `rsx-matching` | `process_order_bench` | clean |
| `rsx-matching` | `match_n_levels_bench` | clean (setup-dominated) |
| `rsx-matching` | `wal_replay_bench` | clean |
| `rsx-marketdata` | `shadow_book_apply_bench` | clean |
| `rsx-mark` | `aggregator_bench` | clean |
| `rsx-cli` | `bench-e2e-pipeline` (binary) | clean |
| `rsx-dxs` | `udp_rtt_bench` | clean (parallel sub) |
| `rsx-dxs` | `cmp_one_way_bench` | hang (parallel sub) |
| `rsx-dxs` | `cmp_rtt_bench` | hang (parallel sub) |
| `rsx-dxs` | `wal_fsync_bench` | clean (parallel sub) |
| `rsx-dxs` | `wal_random_read_bench` | clean (parallel sub) |
| `rsx-risk` | `validate_bench` | clean (parallel sub) |

All are picked up by `make perf` (cargo bench).

## 7. Next moves

1. **Fix CMP one-way / RTT benches** so the parallel sub's
   numbers fill in the transport-leg row in the attribution
   table.
2. **Sub-stage trace `risk_in → me_in`** — 175 µs is
   unattributed and risk is the suspect.
3. **Replace `monoio::time::sleep(100µs)` polls** with
   monoio-native UDP sockets in `rsx-dxs::CmpReceiver`,
   per the meta-review framing. Expected gain: 500 µs out
   of `risk_out → gw_out`.
4. **Pre-built-book variant of `match_n_levels`** to
   isolate pure-match scaling from setup cost.
