//! Match latency by order type. Each order type runs against a freshly
//! built fixture (via `iter_batched`, so setup is untimed and every type
//! is measured the same way) and the timed body is one
//! `process_new_order` call. This exposes the cost of each type's
//! distinct path through the matcher:
//!
//! - `gtc_full_cross` — GTC taker fully filled by one big resting maker.
//! - `ioc`            — IOC taker, one fill then OrderDone (residual 0).
//! - `fok`            — FOK taker, liquidity check then fill.
//! - `post_only_rest` — post-only that does NOT cross, so it rests
//!                      (OrderInserted).
//! - `reduce_only`    — reduce-only sell for a user already long,
//!                      matching a resting bid (a real reducing fill).
//!
//! Reported as `match_by_order_type/<type>`.

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

/// Two-sided book with a resting ask at `MID+1` and bid at `MID-1`
/// (both `BIG_QTY`), plus user 7 made long `qty` by a prior GTC buy.
/// Used by the reduce-only case so it has a real position to reduce and
/// a resting bid to match against.
fn positioned_book(qty: i64) -> Orderbook {
    let mut book = Orderbook::new(harness::config(), 1_024, harness::MID);
    book.insert_resting(
        harness::MID + 1,
        harness::BIG_QTY,
        Side::Sell,
        0,
        200,
        false,
        1,
        0,
        2_000,
    );
    book.insert_resting(
        harness::MID - 1,
        harness::BIG_QTY,
        Side::Buy,
        0,
        201,
        false,
        1,
        0,
        2_001,
    );
    // User 7 buys to establish a long position of `qty`.
    let mut buy = harness::order(
        harness::MID + 1,
        qty,
        Side::Buy,
        TimeInForce::GTC,
        7,
        9_000,
    );
    process_new_order(&mut book, &mut buy);
    book
}

fn bench_by_type(c: &mut Criterion) {
    harness::pin();
    let mut group = c.benchmark_group("match_by_order_type");

    group.bench_function("gtc_full_cross", |b| {
        b.iter_batched(
            || harness::single_ask(harness::BIG_QTY),
            |mut book| {
                let mut o = harness::order(
                    harness::MID + 1,
                    1,
                    Side::Buy,
                    TimeInForce::GTC,
                    1,
                    1,
                );
                process_new_order(black_box(&mut book), black_box(&mut o));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("ioc", |b| {
        b.iter_batched(
            || harness::single_ask(harness::BIG_QTY),
            |mut book| {
                let mut o = harness::order(
                    harness::MID + 1,
                    1,
                    Side::Buy,
                    TimeInForce::IOC,
                    1,
                    1,
                );
                process_new_order(black_box(&mut book), black_box(&mut o));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("fok", |b| {
        b.iter_batched(
            || harness::single_ask(harness::BIG_QTY),
            |mut book| {
                let mut o = harness::order(
                    harness::MID + 1,
                    1,
                    Side::Buy,
                    TimeInForce::FOK,
                    1,
                    1,
                );
                process_new_order(black_box(&mut book), black_box(&mut o));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("post_only_rest", |b| {
        b.iter_batched(
            || harness::single_ask(harness::BIG_QTY),
            |mut book| {
                // Below best ask -> does not cross -> rests.
                let mut o = harness::order(
                    harness::MID - 100,
                    1,
                    Side::Buy,
                    TimeInForce::GTC,
                    1,
                    1,
                );
                o.post_only = true;
                process_new_order(black_box(&mut book), black_box(&mut o));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("reduce_only", |b| {
        b.iter_batched(
            || positioned_book(10),
            |mut book| {
                // User 7 is long 10; reduce-only sell hits resting bid.
                let mut o = harness::order(
                    harness::MID - 1,
                    10,
                    Side::Sell,
                    TimeInForce::GTC,
                    7,
                    1,
                );
                o.reduce_only = true;
                process_new_order(black_box(&mut book), black_box(&mut o));
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = harness::criterion();
    targets = bench_by_type
}
criterion_main!(benches);
