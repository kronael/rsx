// Bench doc-comments aren't rendered to docs; skip the markdown lint.
#![allow(clippy::doc_lazy_continuation)]
//! Full ME accept path + throughput, via the shared `harness::Me`
//! fixture. `Me::accept()` runs everything the matching-engine main loop
//! runs between `me_in` and `me_out` (sans cast send / latency probes):
//! dedup check + `OrderAcceptedRecord` WAL append + `process_new_order`
//! + `write_events_to_wal` + order-index update — all on real production
//! code (real Orderbook seeded with resting liquidity, real WalWriter,
//! real DedupTracker, real FxHashMap index).
//!
//! Two views of the same call:
//! - `me_accept_path/full` — per-order latency (p50).
//! - `me_throughput/orders` — orders/s (each accept does one fill, so
//!   fills/s == orders/s here).

use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use criterion::Throughput;

#[path = "harness.rs"]
mod harness;

/// Depth of resting asks the accept path matches into. 50 keeps the book
/// non-trivial while the best level stays `BIG_QTY` (non-draining).
const DEPTH: u64 = 50;

fn bench_accept_path(c: &mut Criterion) {
    harness::pin();
    let mut group = c.benchmark_group("me_accept_path");
    group.bench_function("full", |b| {
        let mut me = harness::Me::new(DEPTH);
        b.iter(|| me.accept());
    });
    group.finish();
}

fn bench_throughput(c: &mut Criterion) {
    harness::pin();
    let mut group = c.benchmark_group("me_throughput");
    group.throughput(Throughput::Elements(1));
    group.bench_function("orders", |b| {
        let mut me = harness::Me::new(DEPTH);
        b.iter(|| me.accept());
    });
    group.finish();
}

criterion_group! {
    name = benches;
    config = harness::criterion();
    targets = bench_accept_path, bench_throughput
}
criterion_main!(benches);
