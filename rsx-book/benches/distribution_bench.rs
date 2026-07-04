//! Distribution benchmarks: the hot next-best / clear / cancel ops under
//! different order-density SHAPES, to confirm the occupancy bitmap stays
//! O(depth) — not O(slots) — regardless of how the book is populated.
//!
//! Every shape puts a single-order touch one tick inside the book
//! (ask at MID+1, bid at MID-1) with the shape body BEHIND it, so the
//! measured op is always "clear/cancel a 1-order level, then find the
//! next best". What differs is what the find has to traverse:
//! - dense: the next level is adjacent (bits packed in one word).
//! - sparse: the next level is far (find skips empty summary words).
//! - concentrated: the body is one huge wall level (a single set bit).
//!
//! If the bitmap were O(slots) the sparse/concentrated finds would blow
//! up with the gap / book size; O(depth) keeps them flat. Ops:
//! - next_best: read-only `scan_next_ask` with the touch cleared.
//! - match_clears: an IOC taker clears the 1-order touch, then replenish.
//! - cancel_touch: insert a new sole-best, cancel it (empties -> scan).
//! - cancel_deep: insert+cancel an isolated far level (no scan; baseline).
//!
//! Run: cargo bench -p rsx-book --bench distribution_bench

#[path = "harness.rs"]
mod harness;

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::BenchmarkId;
use criterion::Criterion;
use harness::MID;
use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_types::Side;
use rsx_types::TimeInForce;

/// Depths (resting orders per side, excluding the touch) for each shape.
const DEPTHS: [u64; 2] = [1_000, 10_000];

/// A far, always-empty level used by cancel_deep (zone 4 catch-all; no
/// shape here populates beyond ±450k, so it is a fresh slot every time).
const DEEP_ASK: i64 = MID + 2_000_000;

#[derive(Clone, Copy)]
enum Shape {
    Dense,
    Sparse,
    Concentrated,
}

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Shape::Dense => "dense",
            Shape::Sparse => "sparse",
            Shape::Concentrated => "concentrated",
        }
    }
}

fn rest(
    book: &mut Orderbook,
    buy: bool,
    price: i64,
    oid: u64,
) -> u32 {
    let side = if buy { Side::Buy } else { Side::Sell };
    book.insert_resting(price, 10, side, 0, 1, false, 1, 0, oid)
}

/// Build a shape with a 1-order touch at MID±1 and `n` orders per side
/// behind it. Returns the book and the ask-touch handle.
fn build(shape: Shape, n: u64) -> (Orderbook, u32) {
    let cap = (2 * n + 8_192) as u32;
    let mut book = Orderbook::new(harness::config(), cap, MID);
    let ask_touch = rest(&mut book, false, MID + 1, 1);
    rest(&mut book, true, MID - 1, 2);
    let mut oid = 100u64;
    match shape {
        // Contiguous levels behind the touch (offsets 2..=n+1, zone 0).
        Shape::Dense => {
            for i in 2..=(n as i64 + 1) {
                rest(&mut book, false, MID + i, oid);
                oid += 1;
                rest(&mut book, true, MID - i, oid);
                oid += 1;
            }
        }
        // n levels spread across the ±450k range: near-mid spacing is
        // 450k/n (multi-word gaps at low depth), far levels bucket.
        Shape::Sparse => {
            for i in 1..=(n as i64) {
                let off = 2 + (i * 449_998) / n as i64;
                rest(&mut book, false, MID + off, oid);
                oid += 1;
                rest(&mut book, true, MID - off, oid);
                oid += 1;
            }
        }
        // One wall level per side (offset 100): n orders piled on it.
        Shape::Concentrated => {
            for _ in 0..n {
                rest(&mut book, false, MID + 100, oid);
                oid += 1;
                rest(&mut book, true, MID - 100, oid);
                oid += 1;
            }
        }
    }
    (book, ask_touch)
}

fn taker_clear_touch(book: &mut Orderbook, oid: u64) {
    let mut o = IncomingOrder {
        price: MID + 1,
        qty: 10,
        remaining_qty: 10,
        side: Side::Buy,
        tif: TimeInForce::IOC,
        user_id: 1,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1,
        order_id_hi: 0,
        order_id_lo: oid,
    };
    process_new_order(book, &mut o);
}

/// Read-only find cost with the touch cleared: pure `scan_next_ask`.
fn bench_next_best(c: &mut Criterion) {
    harness::pin();
    let mut g = c.benchmark_group("dist_next_best");
    for shape in [Shape::Dense, Shape::Sparse, Shape::Concentrated] {
        for &n in &DEPTHS {
            let (mut book, touch) = build(shape, n);
            book.cancel_order(touch); // clear the touch once
            g.bench_with_input(
                BenchmarkId::new(shape.name(), n),
                &n,
                |b, _| b.iter(|| black_box(book.scan_next_ask(0))),
            );
        }
    }
    g.finish();
}

/// Match-that-clears: IOC taker consumes the 1-order touch (scan fires),
/// then replenish it — net-neutral.
fn bench_match_clears(c: &mut Criterion) {
    harness::pin();
    let mut g = c.benchmark_group("dist_match_clears");
    for shape in [Shape::Dense, Shape::Sparse, Shape::Concentrated] {
        for &n in &DEPTHS {
            let (mut book, _) = build(shape, n);
            let mut oid = 1_000_000u64;
            g.bench_with_input(
                BenchmarkId::new(shape.name(), n),
                &n,
                |b, _| {
                    b.iter(|| {
                        taker_clear_touch(&mut book, oid);
                        oid += 1;
                        rest(&mut book, false, MID + 1, oid);
                        oid += 1;
                    });
                },
            );
        }
    }
    g.finish();
}

/// Cancel-at-touch: insert a new sole-best ask (one tick better), cancel
/// it (empties -> best recompute via scan). Net-neutral.
fn bench_cancel_touch(c: &mut Criterion) {
    harness::pin();
    let mut g = c.benchmark_group("dist_cancel_touch");
    for shape in [Shape::Dense, Shape::Sparse, Shape::Concentrated] {
        for &n in &DEPTHS {
            let (mut book, _) = build(shape, n);
            let mut oid = 2_000_000u64;
            g.bench_with_input(
                BenchmarkId::new(shape.name(), n),
                &n,
                |b, _| {
                    b.iter(|| {
                        let h = rest(&mut book, false, MID, oid);
                        oid += 1;
                        black_box(book.cancel_order(h));
                    });
                },
            );
        }
    }
    g.finish();
}

/// Cancel-deep: insert+cancel an isolated far level (never the best, so
/// no scan). Distribution-independent baseline for the O(1) unlink.
fn bench_cancel_deep(c: &mut Criterion) {
    harness::pin();
    let mut g = c.benchmark_group("dist_cancel_deep");
    for shape in [Shape::Dense, Shape::Sparse, Shape::Concentrated] {
        for &n in &DEPTHS {
            let (mut book, _) = build(shape, n);
            let mut oid = 3_000_000u64;
            g.bench_with_input(
                BenchmarkId::new(shape.name(), n),
                &n,
                |b, _| {
                    b.iter(|| {
                        let h = rest(&mut book, false, DEEP_ASK, oid);
                        oid += 1;
                        black_box(book.cancel_order(h));
                    });
                },
            );
        }
    }
    g.finish();
}

criterion_group! {
    name = benches;
    config = harness::criterion();
    targets =
        bench_next_best,
        bench_match_clears,
        bench_cancel_touch,
        bench_cancel_deep,
}
criterion_main!(benches);
