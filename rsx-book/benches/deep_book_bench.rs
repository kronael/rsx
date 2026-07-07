//! Depth-parametrized comparable set: how insert / cancel / match
//! latency moves as the resting book grows, and how match latency
//! moves by order type. Every bench routes through `harness` (shared
//! core pin + Criterion config + fat-tailed book fixtures) so the
//! numbers are directly comparable.
//!
//! Book model (verified against rsx-book):
//! - `insert_resting` rests directly (no match path) and emits no
//!   events, so seeding cannot cross-match — `build(n)` reaches exactly
//!   n resting orders, deterministically (fixed PRNG seed).
//! - `process_new_order` resets the event buffer per call, so matching
//!   never accumulates events across iterations.
//! - Each measured op is net-neutral on book size (insert+cancel;
//!   match+replenish), so depth stays ~N across criterion's iterations
//!   — the latency reported is genuinely "at depth N", not a depleting
//!   book. Fat-tailed Student-t seed (see harness).
//!
//! Run: cargo bench -p rsx-book --bench deep_book_bench

#[path = "harness.rs"]
mod harness;

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::BenchmarkId;
use criterion::Criterion;
use criterion::Throughput;
use harness::HALF_SPREAD;
use harness::MID;
use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_types::Side;
use rsx_types::TimeInForce;

/// Depth points for the latency-vs-depth curves.
const SIZES_CURVE: [u64; 5] = [1, 100, 1_000, 10_000, 100_000];
/// Deep points proving match/insert latency stays flat (RAM-bound:
/// 10M resting ~ 1.3 GB). Kept separate so the curve groups stay quick.
const SIZES_DEEP: [u64; 3] = [100_000, 1_000_000, 10_000_000];
/// Fixed depth for the by-order-type comparison.
const TYPE_DEPTH: u64 = 10_000;

fn side(buy: bool) -> Side {
    if buy {
        Side::Buy
    } else {
        Side::Sell
    }
}

/// Insert `consumed` back onto the maker side, spread across `levels`
/// price points starting at the touch, so a sweep doesn't collapse the
/// book into a single level over successive iterations.
fn replenish(book: &mut Orderbook, buy: bool, consumed: i64, levels: i64) {
    if consumed <= 0 {
        return;
    }
    let (base_px, s) = if buy {
        (MID + HALF_SPREAD, Side::Sell)
    } else {
        (MID - HALF_SPREAD, Side::Buy)
    };
    let per = (consumed / levels).max(1);
    let mut left = consumed;
    let mut l = 0_i64;
    while left > 0 && l < levels {
        let q = if l == levels - 1 { left } else { per.min(left) };
        let px = if buy { base_px + l } else { base_px - l };
        book.insert_resting(px, q, s, 0, 1, false, 1, 0, 0);
        left -= q;
        l += 1;
    }
}

/// Run a marketable taker of `qty` past the touch, then replenish the
/// consumed quantity so the book stays ~N deep. `levels` controls how
/// far the replenished quantity is spread.
fn taker_fill(book: &mut Orderbook, tif: TimeInForce, buy: bool, qty: i64, levels: i64) {
    let px = if buy {
        MID + 1_000_000
    } else {
        MID - 1_000_000
    };
    let mut o = IncomingOrder {
        price: px,
        qty,
        remaining_qty: qty,
        side: side(buy),
        tif,
        user_id: 1,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1,
        order_id_hi: 0,
        order_id_lo: 0,
    };
    process_new_order(black_box(book), black_box(&mut o));
    replenish(book, buy, qty - o.remaining_qty, levels);
}

/// A post-only order priced to cross: the engine rejects it on the
/// cross check (nothing consumed, book unchanged). Measures the
/// post-only guard path, net-neutral.
fn taker_postonly_cross(book: &mut Orderbook, buy: bool) {
    let px = if buy {
        MID + 1_000_000
    } else {
        MID - 1_000_000
    };
    let mut o = IncomingOrder {
        price: px,
        qty: 100,
        remaining_qty: 100,
        side: side(buy),
        tif: TimeInForce::GTC,
        user_id: 1,
        reduce_only: false,
        post_only: true,
        timestamp_ns: 1,
        order_id_hi: 0,
        order_id_lo: 0,
    };
    process_new_order(black_box(book), black_box(&mut o));
}

/// Insert+cancel at depth N (net-neutral). Measures the insert path
/// plus the paired cancel against an N-deep book.
fn insert_group(c: &mut Criterion, name: &str, sizes: &[u64]) {
    let mut g = c.benchmark_group(name);
    g.throughput(Throughput::Elements(1));
    for &n in sizes {
        let mut book = harness::build(n);
        let mut rng = harness::Rng::new(0xBEEF ^ n);
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let (p, q, s) = harness::draw_resting(&mut rng);
                let h = book.insert_resting(p, q, s, 0, 1, false, 1, 0, 0);
                black_box(book.cancel_order(h));
            });
        });
    }
    g.finish();
}

/// Pure cancel at depth N. A pool of cancelable handles sits on top of
/// the N-deep book; each timed batch cancels the pool, then the pool is
/// refilled UNTIMED, so only cancel time is measured. Pool is small vs
/// N (clamped) so "depth N" stays meaningful.
fn cancel_group(c: &mut Criterion, name: &str, sizes: &[u64]) {
    let mut g = c.benchmark_group(name);
    g.throughput(Throughput::Elements(1));
    for &n in sizes {
        let pool = (n as usize).clamp(1, 4096);
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter_custom(|iters| {
                let mut book = harness::build(n);
                let mut rng = harness::Rng::new(0xCA9 ^ n);
                let mut handles: Vec<u32> = (0..pool)
                    .map(|_| {
                        let (p, q, s) = harness::draw_resting(&mut rng);
                        book.insert_resting(p, q, s, 0, 1, false, 1, 0, 0)
                    })
                    .collect();
                let mut done = 0_u64;
                let mut elapsed = std::time::Duration::ZERO;
                while done < iters {
                    let batch = ((iters - done) as usize).min(handles.len());
                    let start = std::time::Instant::now();
                    for h in handles.iter().take(batch) {
                        black_box(book.cancel_order(*h));
                    }
                    elapsed += start.elapsed();
                    done += batch as u64;
                    for slot in handles.iter_mut().take(batch) {
                        let (p, q, s) = harness::draw_resting(&mut rng);
                        *slot = book.insert_resting(p, q, s, 0, 1, false, 1, 0, 0);
                    }
                }
                elapsed
            });
        });
    }
    g.finish();
}

/// Match at depth N: an IOC taker (fat-tailed size) sweeps from the
/// touch; the consumed quantity is replenished at the touch. Measures
/// match+replenish against an N-deep book.
fn match_group(c: &mut Criterion, name: &str, sizes: &[u64]) {
    let mut g = c.benchmark_group(name);
    g.throughput(Throughput::Elements(1));
    for &n in sizes {
        let mut book = harness::build(n);
        let mut rng = harness::Rng::new(0xF00D ^ n);
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let buy = rng.next_u64() & 1 == 0;
                let q = (harness::BASE * (1.0 + rng.exp(1.0))).round().max(1.0) as i64;
                taker_fill(&mut book, TimeInForce::IOC, buy, q, 1);
            });
        });
    }
    g.finish();
}

fn bench_insert_cancel_depth(c: &mut Criterion) {
    harness::pin();
    // insert_group times an insert+cancel PAIR (net-neutral to hold
    // depth constant), so the honest name is insert_cancel, not a pure
    // insert latency.
    insert_group(c, "insert_cancel_depth", &SIZES_CURVE);
}

fn bench_cancel_depth(c: &mut Criterion) {
    harness::pin();
    cancel_group(c, "cancel_depth", &SIZES_CURVE);
}

fn bench_match_depth(c: &mut Criterion) {
    harness::pin();
    match_group(c, "match_depth", &SIZES_CURVE);
}

/// Match latency by order type at a fixed depth. Each type gets a
/// fresh N-deep book (build is cheap at 10k) so types don't contaminate
/// each other. All net-neutral.
fn bench_match_by_type(c: &mut Criterion) {
    harness::pin();
    let mut g = c.benchmark_group("match_by_type");
    g.throughput(Throughput::Elements(1));

    g.bench_function("gtc_full_cross", |b| {
        let mut book = harness::build(TYPE_DEPTH);
        b.iter(|| taker_fill(&mut book, TimeInForce::GTC, true, 100, 1));
    });
    g.bench_function("ioc_full", |b| {
        let mut book = harness::build(TYPE_DEPTH);
        b.iter(|| taker_fill(&mut book, TimeInForce::IOC, true, 100, 1));
    });
    g.bench_function("fok_full", |b| {
        let mut book = harness::build(TYPE_DEPTH);
        b.iter(|| taker_fill(&mut book, TimeInForce::FOK, true, 100, 1));
    });
    g.bench_function("post_only_reject", |b| {
        let mut book = harness::build(TYPE_DEPTH);
        b.iter(|| taker_postonly_cross(&mut book, true));
    });
    g.bench_function("sweep_10_levels", |b| {
        let mut book = harness::build(TYPE_DEPTH);
        b.iter(|| taker_fill(&mut book, TimeInForce::IOC, true, 1000, 10));
    });
    g.finish();
}

fn bench_deep_flat_insert(c: &mut Criterion) {
    harness::pin();
    insert_group(c, "deep_flat_insert", &SIZES_DEEP);
}

fn bench_deep_flat_match(c: &mut Criterion) {
    harness::pin();
    match_group(c, "deep_flat_match", &SIZES_DEEP);
}

criterion_group! {
    name = benches;
    config = harness::criterion();
    targets =
        bench_insert_cancel_depth,
        bench_cancel_depth,
        bench_match_depth,
        bench_match_by_type,
        bench_deep_flat_insert,
        bench_deep_flat_match,
}
criterion_main!(benches);
