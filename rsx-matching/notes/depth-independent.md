# Fixed-work match sweep

An order book holds resting bids and asks for one symbol. Its depth is the
number of resting orders. A benchmark that varies depth must keep the work
performed by the measured operation unchanged.

## Problem

A single latency point cannot show whether preloaded book size affects a
fixed-work match. A sweep that consumes more makers as the book grows also
cannot isolate depth from work.

## Fix

`benches/match_depth_bench.rs` preloads different numbers of resting orders,
then measures the same one-lot, non-draining partial fill at each depth.
`rsx-matching` delegates the transition to
`rsx_book::matching::process_new_order`; the compression map and occupancy
summary are documented in `rsx-book/`.

```text
risk order ──► decode ──► fixed-work book transition ──► ordered events
```

The current sweep observes no depth-related latency increase over its tested
range. That is an empirical statement about this workload, not a claim that
all matching work is constant. Consuming `K` makers remains O(K), and a
compressed slot may require inspecting several orders.

## Cost it removes

The sweep detects regressions where preloaded state leaks into a
non-draining match. It does not replace separate benchmarks for level clearing,
deep sweeps, compressed slots, or the full matching process.

## Evidence

Benchmark source: `benches/match_depth_bench.rs`. Dated results live in the
repository-level `reports/` directory. Exact latency tables are published only
when p50 and p99 come from the same run and workload.
