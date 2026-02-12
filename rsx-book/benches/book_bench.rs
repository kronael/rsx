use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_book::CompressionMap;
use rsx_book::Orderbook;
use rsx_book::OrderSlot;
use rsx_book::Slab;
use rsx_book::matching::IncomingOrder;
use rsx_book::matching::process_new_order;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 1,
        lot_size: 1,
    }
}

fn bench_slab_alloc_free(c: &mut Criterion) {
    // Bump-path alloc
    c.bench_function("slab_alloc_bump", |b| {
        b.iter_custom(|iters| {
            let mut s: Slab<OrderSlot> = Slab::new(
                iters.max(1024) as u32,
            );
            let start = std::time::Instant::now();
            for _ in 0..iters {
                s.alloc();
            }
            start.elapsed()
        });
    });

    // Free-list path: pre-fill, free all, then bench alloc
    c.bench_function("slab_alloc_from_freelist", |b| {
        let mut slab: Slab<OrderSlot> = Slab::new(100_000);
        let handles: Vec<u32> =
            (0..100_000).map(|_| slab.alloc()).collect();
        for &h in &handles {
            slab.free(h);
        }
        b.iter(|| {
            let h = slab.alloc();
            slab.free(h);
        });
    });

    // Free bench
    c.bench_function("slab_free", |b| {
        b.iter_custom(|iters| {
            let n = iters.max(1024) as u32;
            let mut s: Slab<OrderSlot> = Slab::new(n);
            let handles: Vec<u32> =
                (0..n).map(|_| s.alloc()).collect();
            let start = std::time::Instant::now();
            for &h in &handles {
                s.free(h);
            }
            start.elapsed()
        });
    });
}

fn bench_compression_map(c: &mut Criterion) {
    let map = CompressionMap::new(50000, 1);
    c.bench_function("compression_price_to_index_near", |b| {
        b.iter(|| black_box(map.price_to_index(50010)));
    });
    c.bench_function("compression_price_to_index_far", |b| {
        b.iter(|| black_box(map.price_to_index(65000)));
    });
    c.bench_function("compression_new", |b| {
        b.iter(|| black_box(CompressionMap::new(50000, 1)));
    });
}

fn bench_insert_resting(c: &mut Criterion) {
    c.bench_function("insert_resting_order", |b| {
        let mut book =
            Orderbook::new(config(), 1_000_000, 50000);
        let mut price = 49000_i64;
        b.iter(|| {
            let h = book.insert_resting(
                price, 100, Side::Buy, 0, 1, false,
                1000, 0, price as u64,
            );
            book.cancel_order(h);
            price += 1;
            if price > 49999 {
                price = 49000;
            }
        });
    });
}

fn bench_cancel_order(c: &mut Criterion) {
    c.bench_function("cancel_order", |b| {
        b.iter_custom(|iters| {
            let n = iters.min(100_000) as i64;
            let mut book =
                Orderbook::new(config(), n as u32 + 1, 50000);
            let handles: Vec<u32> = (0..n)
                .map(|i| {
                    book.insert_resting(
                        49000 + (i % 1000), 100, Side::Buy,
                        0, 1, false, 1000, 0, i as u64,
                    )
                })
                .collect();
            let start = std::time::Instant::now();
            for &h in &handles {
                black_box(book.cancel_order(h));
            }
            start.elapsed()
        });
    });
}

fn bench_match_single_fill(c: &mut Criterion) {
    c.bench_function("match_single_fill", |b| {
        let mut book =
            Orderbook::new(config(), 1_000_000, 50000);
        // Pre-fill asks
        for i in 0..1000 {
            book.insert_resting(
                50001 + i, 100, Side::Sell, 0, 2, false,
                1000, 0, i as u64,
            );
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
            book.insert_resting(
                50001, 100, Side::Sell, 0, 2, false,
                1000, 0, seq,
            );
            seq += 1;
        });
    });
}

fn bench_match_deep_book(c: &mut Criterion) {
    c.bench_function("match_sweep_10_levels", |b| {
        let mut book =
            Orderbook::new(config(), 1_000_000, 50000);
        let mut seq = 0_u64;
        b.iter(|| {
            // Build 10 levels of asks
            for i in 0..10 {
                book.insert_resting(
                    50001 + i, 100, Side::Sell, 0, 2,
                    false, 1000, 0, seq,
                );
                seq += 1;
            }
            // Aggressive buy sweeps all 10
            let mut order = IncomingOrder {
                price: 50011,
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
        });
    });
}

fn bench_insert_order_zone_3(c: &mut Criterion) {
    c.bench_function("insert_order_zone_3", |b| {
        let mut book =
            Orderbook::new(config(), 1_000_000, 50000);
        // Zone 3 = 30-50% away = 65000-75000
        let mut price = 70000_i64;
        b.iter(|| {
            let h = book.insert_resting(
                price, 100, Side::Sell, 0, 1, false,
                1000, 0, price as u64,
            );
            black_box(h);
            book.cancel_order(h);
            price += 1;
            if price > 74000 {
                price = 70000;
            }
        });
    });
}

fn bench_match_smooshed_level_100(c: &mut Criterion) {
    c.bench_function("match_smooshed_level_100", |b| {
        let mut book =
            Orderbook::new(config(), 1_000_000, 50000);
        let mut seq = 0_u64;
        b.iter(|| {
            // 100 orders at same price (smooshed level)
            for _ in 0..100 {
                book.insert_resting(
                    50001, 10, Side::Sell, 0, 2, false,
                    1000, 0, seq,
                );
                seq += 1;
            }
            // Sweep all 100
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
        });
    });
}

fn bench_price_to_index_bisection(c: &mut Criterion) {
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
    c.bench_function("recenter_10k_orders", |b| {
        b.iter_custom(|iters| {
            let mut total =
                std::time::Duration::from_secs(0);
            for _ in 0..iters {
                let mut book = Orderbook::new(
                    config(), 20_000, 50000,
                );
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
    c.bench_function("recenter_lazy_per_access", |b| {
        let mut book =
            Orderbook::new(config(), 20_000, 50000);
        for i in 0..1000_i64 {
            book.insert_resting(
                49000 + i, 100, Side::Buy, 0, 1, false,
                1000, 0, i as u64,
            );
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
    c.bench_function("event_buffer_drain_100", |b| {
        let mut book =
            Orderbook::new(config(), 1_000_000, 50000);
        let mut seq = 0_u64;
        b.iter(|| {
            // Insert 100 asks then sweep to generate
            // ~100 fill events
            for _ in 0..100 {
                book.insert_resting(
                    50001, 10, Side::Sell, 0, 2, false,
                    1000, 0, seq,
                );
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
    c.bench_function(
        "best_bid_scan_after_cancel",
        |b| {
            let mut book = Orderbook::new(
                config(), 1_000_000, 50000,
            );
            // Insert orders at consecutive bid prices
            let mut handles = Vec::with_capacity(1000);
            for i in 0..1000_i64 {
                let h = book.insert_resting(
                    49000 + i, 100, Side::Buy, 0, 1,
                    false, 1000, 0, i as u64,
                );
                handles.push(h);
            }
            // Cancel from best bid downward, forcing
            // scan each time
            let mut idx = handles.len() - 1;
            b.iter(|| {
                if idx > 0 {
                    black_box(
                        book.cancel_order(handles[idx]),
                    );
                    idx -= 1;
                } else {
                    // Rebuild for next round
                    book = Orderbook::new(
                        config(), 1_000_000, 50000,
                    );
                    handles.clear();
                    for i in 0..1000_i64 {
                        let h = book.insert_resting(
                            49000 + i, 100, Side::Buy,
                            0, 1, false, 1000, 0,
                            i as u64,
                        );
                        handles.push(h);
                    }
                    idx = handles.len() - 1;
                }
            });
        },
    );
}

fn bench_modify_order_price_change(c: &mut Criterion) {
    c.bench_function("modify_order_price_change", |b| {
        let mut book =
            Orderbook::new(config(), 1_000_000, 50000);
        let mut handle = book.insert_resting(
            49500, 100, Side::Buy, 0, 1, false,
            1000, 0, 1,
        );
        let mut price = 49500_i64;
        let mut seq = 100_u64;
        b.iter(|| {
            price += 1;
            if price > 49999 {
                price = 49000;
            }
            seq += 1;
            handle = book.modify_order_price(
                handle, price, Side::Buy, 0, 1, false,
                1000, 0, seq,
            );
            black_box(handle);
        });
    });
}

fn bench_modify_order_qty_down(c: &mut Criterion) {
    c.bench_function("modify_order_qty_down", |b| {
        b.iter_custom(|iters| {
            let mut book = Orderbook::new(
                config(), 1_000_000, 50000,
            );
            // Large initial qty so we can reduce many
            // times without exhausting
            let n = iters.min(500_000) as i64;
            let handle = book.insert_resting(
                49500, n + 1, Side::Buy, 0, 1, false,
                1000, 0, 1,
            );
            let start = std::time::Instant::now();
            for i in 0..n {
                black_box(book.modify_order_qty_down(
                    handle, n - i,
                ));
            }
            start.elapsed()
        });
    });
}

fn bench_match_10_fills_same_level(c: &mut Criterion) {
    c.bench_function("match_10_fills_same_level", |b| {
        let mut book =
            Orderbook::new(config(), 1_000_000, 50000);
        let mut seq = 0_u64;
        b.iter(|| {
            // 10 orders at same price
            for _ in 0..10 {
                book.insert_resting(
                    50001, 100, Side::Sell, 0, 2, false,
                    1000, 0, seq,
                );
                seq += 1;
            }
            // Match all 10
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
        });
    });
}

criterion_group!(
    benches,
    bench_slab_alloc_free,
    bench_compression_map,
    bench_insert_resting,
    bench_cancel_order,
    bench_match_single_fill,
    bench_match_deep_book,
    bench_insert_order_zone_3,
    bench_match_smooshed_level_100,
    bench_price_to_index_bisection,
    bench_recenter_10k_orders,
    bench_recenter_lazy_per_access,
    bench_event_buffer_drain_100,
    bench_best_bid_scan_after_cancel,
    bench_modify_order_price_change,
    bench_modify_order_qty_down,
    bench_match_10_fills_same_level
);
criterion_main!(benches);
