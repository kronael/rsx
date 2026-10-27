# Benchmark index

This file maps each benchmark to the production path it measures. Results live
in dated [`reports/`](../reports/) files so hardware, commit, workload, p50,
p99, and limitations stay together.

Do not add a latency table with only a mean or median. A published latency
table requires p50 and p99 from the same run and workload.

## Running benchmarks

```bash
cargo bench -p <crate> --bench <bench>
make perf
make perf-gate
make perf-load
```

Criterion output under `target/criterion/` is transient. A run worth keeping
gets a dated report before its numbers are quoted elsewhere.

## Benchmark families

| Family | Measures | Use |
|---|---|---|
| component path | one production component or route | attribution and regression gates |
| primitive | one operation inside a component | local optimization and regression diagnosis |
| load | an offered-rate curve through a real route | throughput, queueing, and tail behavior |

Primitive measurements do not add up to an end-to-end latency result. Route
measurements own route claims.

## `rsx-types`

| Benchmark | Measures |
|---|---|
| `types_bench` | fixed-point arithmetic, validation, and notional calculation |

## `rsx-messages`

| Benchmark | Measures |
|---|---|
| `encode_bench` | exchange record encode, decode, and CRC work |

## `rsx-cast`

| Benchmark | Measures |
|---|---|
| `compare_udp` | raw UDP loopback baseline |
| `cast_one_way_bench` | one casting send and receive |
| `cast_rtt_bench` | casting echo round trip |
| `cast_send_breakdown_bench` | framing, copy, CRC, and socket-send attribution |
| `wal_bench` | in-memory WAL append, sequential read, and replay |
| `wal_fsync_bench` | append plus explicit disk flush |
| `wal_random_read_bench` | random sequence lookup in a populated WAL |
| `cast_bench` | control-record primitives |

## `rsx-book`

| Benchmark | Measures |
|---|---|
| `book_bench` | slab reuse, price mapping, construction, insert, match, and cancel |
| `match_depth` | one fixed-work fill while preloaded order count changes |
| `deep_flat_match` | fixed-work fill at large preloaded depths |
| `deep_match_tail` | same-run p50 and p99 for the fixed-work depth sweep |
| `tail_bench` | operation distributions, including touch-clearing paths |
| `compare_all_bench` | shared operation stream across supported contenders |

`compare_all_bench` does not yet collect p99 for every contender, so its exact
median tables are not published in the comparison index.

## `rsx-matching`

| Benchmark | Measures |
|---|---|
| `process_order_bench` | deduplication, acceptance WAL work, book transition, and event WAL work |
| `me_tail_bench` | same-run p50 and p99 of `process_order` with cast sends, and of the checkpoint capture pause |
| `match_n_levels_bench` | incoming orders that consume different numbers of levels |
| `wal_replay_bench` | matching WAL replay |
| `matching_bench` | matching-process primitives |

## `rsx-gateway`

| Benchmark | Measures |
|---|---|
| `ws_parse_bench` | WebSocket order and cancel parsing |
| `ws_encode_bench` | WebSocket fill and order-update encoding |
| `jwt_validate_bench` | JWT validation |
| `gateway_bench` | gateway primitives |
| `gw_tail_bench` | same-run p50 and p99 of a WebSocket order frame to the casting datagram at risk, and of a casting record at the gateway to the `U` or `F` frame on the client socket |

## `rsx-marketdata`

| Benchmark | Measures |
|---|---|
| `shadow_book_apply_bench` | insert, fill, cancel, and BBO updates in the shadow book |
| `marketdata_bench` | L2 snapshots, deltas, serialization, and event throughput |

## `rsx-mark`

| Benchmark | Measures |
|---|---|
| `aggregator_bench` | multi-source mark aggregation |
| `mark_bench` | source-to-publish work inside the aggregator |

## `rsx-risk`

| Benchmark | Measures |
|---|---|
| `validate_bench` | portfolio pre-trade validation |
| `risk_bench` | index updates, BBO handling, liquidation enqueue, and escalation |

## Route programs

| Program | Measures |
|---|---|
| `bench-match-rt` | in-process casting, matching, and WAL route with per-stage timing |
| `rsx-cli stress --mode probe` | client-observed WebSocket order round trip through a running cluster |
| `make perf-load` | sustained external route (`rsx-cli stress --mode load`) with accepted rate and latency distribution |

## Evidence chain

1. The benchmark source defines the workload.
2. A dated `reports/YYYYMMDD_*.md` file records the commit, environment,
   sample shape, p50, p99, and limitations.
3. Summary docs link that report instead of copying an incomplete point
   estimate.
4. Regression gates compare compatible workloads only.

The current fixed-work order-book distribution is in
[`reports/20260727_book-deep-match-tail.md`](../reports/20260727_book-deep-match-tail.md).
The current in-process route distribution is in
[`reports/20260708_bench-rerun.md`](../reports/20260708_bench-rerun.md).
