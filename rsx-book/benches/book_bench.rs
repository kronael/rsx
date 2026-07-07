//! Orderbook micro-ops: slab alloc/free, CompressionMap lookup +
//! recenter, a marketable IOC fill vs a 1k-ask book, modify, BBO
//! scan, event drain.
//! Depth-vs-latency curves and match-by-type live in
//! `deep_book_bench.rs`; both route through the shared `harness`.
//!
//! See `docs/benches.md` for the full bench index +
//! production-leg attribution.

#[path = "harness.rs"]
mod harness;

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_book::CompressionMap;
use rsx_book::OrderSlot;
use rsx_book::Orderbook;
use rsx_book::Slab;
use rsx_types::Side;
use rsx_types::TimeInForce;

fn bench_slab_alloc_free(c: &mut Criterion) {
    harness::pin();
    // Bump-path alloc
    c.bench_function("slab_alloc_bump", |b| {
        b.iter_custom(|iters| {
            let mut s: Slab<OrderSlot> = Slab::new(iters.max(1024) as u32);
            let start = std::time::Instant::now();
            for _ in 0..iters {
                black_box(s.alloc());
            }
            start.elapsed()
        });
    });

    // Free-list path: pre-fill, free all, then bench alloc
    c.bench_function("slab_alloc_from_freelist", |b| {
        let mut slab: Slab<OrderSlot> = Slab::new(100_000);
        let handles: Vec<u32> = (0..100_000).map(|_| slab.alloc()).collect();
        for &h in &handles {
            slab.free(h);
        }
        b.iter(|| {
            let h = black_box(slab.alloc());
            slab.free(black_box(h));
        });
    });

    // Free bench
    c.bench_function("slab_free", |b| {
        b.iter_custom(|iters| {
            let n = iters.max(1024) as u32;
            let mut s: Slab<OrderSlot> = Slab::new(n);
            let handles: Vec<u32> = (0..n).map(|_| s.alloc()).collect();
            let start = std::time::Instant::now();
            for &h in &handles {
                s.free(h);
            }
            start.elapsed()
        });
    });
}

fn bench_compression_map(c: &mut Criterion) {
    harness::pin();
    let map = CompressionMap::new(50000, 1);
    c.bench_function("compression_price_to_index_near", |b| {
        b.iter(|| black_box(map.price_to_index(black_box(50010))));
    });
    c.bench_function("compression_price_to_index_far", |b| {
        b.iter(|| black_box(map.price_to_index(black_box(65000))));
    });
    c.bench_function("compression_new", |b| {
        b.iter(|| black_box(CompressionMap::new(50000, 1)));
    });
}

// Marketable IOC buy against a 1000-level ask book, with the consumed
// level replenished each iteration. NOT an isolated single-fill latency
// number: it times the full `process_new_order` path plus the replenish
// insert against a deep resting book. Named for what it measures.
fn bench_match_ioc_vs_1k_asks(c: &mut Criterion) {
    harness::pin();
    c.bench_function("match_ioc_vs_1k_asks", |b| {
        let mut book = Orderbook::new(harness::config(), 1_000_000, 50000);
        // Pre-fill asks
        for i in 0..1000 {
            book.insert_resting(50001 + i, 100, Side::Sell, 0, 2, false, 1000, 0, i as u64);
        }
        let mut seq = 10000_u64;
        b.iter(|| {
            let mut order = IncomingOrder {
                price: 51000,
                qty: 100,
                remaining_qty: 100,
                side: Side::Buy,
                tif: TimeInForce::IOC,
                user_id: 1,
                reduce_only: false,
                post_only: false,
                timestamp_ns: 1000,
                order_id_hi: 0,
                order_id_lo: seq,
            };
            process_new_order(&mut book, &mut order);
            seq += 1;
            // Replenish the consumed ask
            book.insert_resting(50001, 100, Side::Sell, 0, 2, false, 1000, 0, seq);
            seq += 1;
        });
    });
}

fn bench_price_to_index_bisection(c: &mut Criterion) {
    harness::pin();
    let map = CompressionMap::new(50000, 1);
    c.bench_function("price_to_index_bisection", |b| {
        let mut price = 49000_i64;
        b.iter(|| {
            let idx = map.price_to_index(price);
            black_box(idx);
            price += 1;
            if price > 51000 {
                price = 49000;
            }
        });
    });
}

fn bench_recenter_10k_orders(c: &mut Criterion) {
    harness::pin();
    c.bench_function("recenter_10k_orders", |b| {
        b.iter_custom(|iters| {
            let mut total = std::time::Duration::from_secs(0);
            for _ in 0..iters {
                let mut book = Orderbook::new(harness::config(), 20_000, 50000);
                for i in 0..10_000_i64 {
                    book.insert_resting(
                        49000 + (i % 2000),
                        100,
                        Side::Buy,
                        0,
                        1,
                        false,
                        1000,
                        0,
                        i as u64,
                    );
                }
                let start = std::time::Instant::now();
                book.trigger_recenter(50500);
                // Complete migration via batch
                book.migrate_batch(100_000);
                total += start.elapsed();
            }
            total
        });
    });
}

fn bench_recenter_lazy_per_access(c: &mut Criterion) {
    harness::pin();
    c.bench_function("recenter_lazy_per_access", |b| {
        let mut book = Orderbook::new(harness::config(), 20_000, 50000);
        for i in 0..1000_i64 {
            book.insert_resting(49000 + i, 100, Side::Buy, 0, 1, false, 1000, 0, i as u64);
        }
        book.trigger_recenter(50500);
        let mut price = 48500_i64;
        b.iter(|| {
            book.resolve_level(price);
            black_box(price);
            price -= 1;
            if price < 47000 {
                // Reset migration state for continued
                // benching
                price = 48500;
            }
        });
    });
}

fn bench_event_buffer_drain_100(c: &mut Criterion) {
    harness::pin();
    c.bench_function("event_buffer_drain_100", |b| {
        let mut book = Orderbook::new(harness::config(), 1_000_000, 50000);
        let mut seq = 0_u64;
        b.iter(|| {
            // Insert 100 asks then sweep to generate
            // ~100 fill events
            for _ in 0..100 {
                book.insert_resting(50001, 10, Side::Sell, 0, 2, false, 1000, 0, seq);
                seq += 1;
            }
            let mut order = IncomingOrder {
                price: 50001,
                qty: 1000,
                remaining_qty: 1000,
                side: Side::Buy,
                tif: TimeInForce::IOC,
                user_id: 1,
                reduce_only: false,
                post_only: false,
                timestamp_ns: 1000,
                order_id_hi: 0,
                order_id_lo: seq,
            };
            process_new_order(&mut book, &mut order);
            seq += 1;
            // Drain events
            let events = book.events();
            black_box(events.len());
        });
    });
}

fn bench_best_bid_scan_after_cancel(c: &mut Criterion) {
    harness::pin();
    c.bench_function("best_bid_scan_after_cancel", |b| {
        let mut book = Orderbook::new(harness::config(), 1_000_000, 50000);
        // Insert orders at consecutive bid prices
        let mut handles = Vec::with_capacity(1000);
        for i in 0..1000_i64 {
            let h = book.insert_resting(49000 + i, 100, Side::Buy, 0, 1, false, 1000, 0, i as u64);
            handles.push(h);
        }
        // Cancel from best bid downward, forcing
        // scan each time
        let mut idx = handles.len() - 1;
        b.iter(|| {
            if idx > 0 {
                black_box(book.cancel_order(handles[idx]));
                idx -= 1;
            } else {
                // Rebuild for next round
                book = Orderbook::new(harness::config(), 1_000_000, 50000);
                handles.clear();
                for i in 0..1000_i64 {
                    let h = book.insert_resting(
                        49000 + i,
                        100,
                        Side::Buy,
                        0,
                        1,
                        false,
                        1000,
                        0,
                        i as u64,
                    );
                    handles.push(h);
                }
                idx = handles.len() - 1;
            }
        });
    });
}

fn bench_modify_order_price_change(c: &mut Criterion) {
    harness::pin();
    c.bench_function("modify_order_price_change", |b| {
        let mut book = Orderbook::new(harness::config(), 1_000_000, 50000);
        let mut handle = book.insert_resting(49500, 100, Side::Buy, 0, 1, false, 1000, 0, 1);
        let mut price = 49500_i64;
        let mut seq = 100_u64;
        b.iter(|| {
            price += 1;
            if price > 49999 {
                price = 49000;
            }
            seq += 1;
            handle = book.modify_order_price(handle, price, Side::Buy, 0, 1, false, 1000, 0, seq);
            black_box(handle);
        });
    });
}

fn bench_modify_order_qty_down(c: &mut Criterion) {
    harness::pin();
    c.bench_function("modify_order_qty_down", |b| {
        b.iter_custom(|iters| {
            let mut book = Orderbook::new(harness::config(), 1_000_000, 50000);
            // qty must strictly decrease each call, so seed the order
            // with qty = iters+1 and step it down exactly `iters`
            // times. Running exactly `iters` ops honors criterion's
            // iter_custom contract — the old `min(500_000)` clamp ran
            // fewer ops than `iters` divided by, reporting ~0 ps.
            let n = iters as i64;
            let handle = book.insert_resting(49500, n + 1, Side::Buy, 0, 1, false, 1000, 0, 1);
            let start = std::time::Instant::now();
            for i in 0..n {
                black_box(book.modify_order_qty_down(black_box(handle), black_box(n - i)));
            }
            start.elapsed()
        });
    });
}

criterion_group! {
    name = benches;
    config = harness::criterion();
    targets =
        bench_slab_alloc_free,
        bench_compression_map,
        bench_match_ioc_vs_1k_asks,
        bench_price_to_index_bisection,
        bench_recenter_10k_orders,
        bench_recenter_lazy_per_access,
        bench_event_buffer_drain_100,
        bench_best_bid_scan_after_cancel,
        bench_modify_order_price_change,
        bench_modify_order_qty_down,
}
criterion_main!(benches);
