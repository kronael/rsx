//! Sweep-N-levels: one aggressive taker consumes N resting price levels
//! in a single `process_new_order`. Each level holds one resting order
//! of qty 1 at a distinct tick; the taker is sized to consume all of
//! them exactly (qty == n). This exposes how the match loop scales with
//! fills — the single-fill baseline (~54ns, see
//! reports/20260530_component-benches.md) only covers n=1, but real
//! takers sweep many levels at once.
//!
//! Reported as `sweep_n_levels/n={n}` for n in {1, 5, 20, 100}.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::BatchSize;
use criterion::Criterion;
use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_types::Side;
use rsx_types::TimeInForce;

#[path = "harness.rs"]
mod harness;

/// `n` distinct ask levels, one resting order of qty 1 per level, from
/// `MID+1` up. An aggressive bid at `MID+n` sweeps all of them.
fn book_with_ladder(n: usize) -> Orderbook {
    let mut book =
        Orderbook::new(harness::config(), 65_536, harness::MID);
    for i in 0..n {
        book.insert_resting(
            harness::MID + 1 + i as i64,
            1,
            Side::Sell,
            0,
            200 + i as u32,
            false,
            1,
            0,
            2_000 + i as u64,
        );
    }
    book
}

fn bench_sweep_n_levels(c: &mut Criterion) {
    harness::pin();
    let mut group = c.benchmark_group("sweep_n_levels");
    for n in [1usize, 5, 20, 100] {
        group.bench_function(format!("n={n}"), |b| {
            b.iter_batched(
                || book_with_ladder(n),
                |mut book| {
                    let mut bid = harness::order(
                        harness::MID + n as i64,
                        n as i64,
                        Side::Buy,
                        TimeInForce::GTC,
                        1,
                        9_999,
                    );
                    process_new_order(
                        black_box(&mut book),
                        black_box(&mut bid),
                    );
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = harness::criterion();
    targets = bench_sweep_n_levels
}
criterion_main!(benches);
