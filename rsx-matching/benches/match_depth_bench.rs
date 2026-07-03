//! Match latency vs book depth. A single qty-1 taker buy crosses the
//! best ask of a book holding `n` resting orders. The maker at best is
//! `BIG_QTY`, so the fill is one non-draining partial fill and the match
//! work is held constant while only the resting depth varies — this
//! isolates whether a fuller book/slab slows a single match (it should
//! not: O(1) best-level access on the compressed index).
//!
//! Reported as `match_by_depth/n={n}` for n in {1, 100, 1k, 10k, 100k}.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_book::matching::process_new_order;
use rsx_types::Side;
use rsx_types::TimeInForce;

#[path = "harness.rs"]
mod harness;

fn bench_match_by_depth(c: &mut Criterion) {
    harness::pin();
    let mut group = c.benchmark_group("match_by_depth");
    for n in [1u64, 100, 1_000, 10_000, 100_000] {
        group.bench_function(format!("n={n}"), |b| {
            let mut book = harness::build_book(n);
            b.iter(|| {
                let mut taker = harness::order(
                    harness::MID + 1,
                    1,
                    Side::Buy,
                    TimeInForce::IOC,
                    1,
                    0,
                );
                process_new_order(
                    black_box(&mut book),
                    black_box(&mut taker),
                );
            });
        });
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = harness::criterion();
    targets = bench_match_by_depth
}
criterion_main!(benches);
