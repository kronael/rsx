use criterion::criterion_group;
use criterion::criterion_main;
use criterion::black_box;
use criterion::Criterion;
use rsx_book::book::Orderbook;
use rsx_book::compression::CompressionMap;
use rsx_book::matching::IncomingOrder;
use rsx_book::matching::process_new_order;
use rsx_book::slab::Slab;
use rsx_book::OrderSlot;
use rsx_dxs::wal::WalWriter;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::wal_integration::write_events_to_wal;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use std::path::PathBuf;

fn test_config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 4,
        tick_size: 1,
        lot_size: 1,
    }
}

fn make_book() -> Orderbook {
    Orderbook::new(test_config(), 65536, 100_000)
}

fn make_order(
    price: i64,
    qty: i64,
    side: Side,
    user_id: u32,
) -> IncomingOrder {
    IncomingOrder {
        price,
        qty,
        remaining_qty: qty,
        side,
        tif: TimeInForce::GTC,
        user_id,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1_000_000,
        order_id_hi: 0,
        order_id_lo: user_id as u64,
    }
}

// --- Dedup benchmarks ---

fn bench_dedup_insert(c: &mut Criterion) {
    c.bench_function(
        "dedup_check_and_insert_new",
        |b| {
            let mut dedup = DedupTracker::new();
            let mut id = 0_u64;
            b.iter(|| {
                dedup.check_and_insert(1, 0, id);
                id += 1;
            });
        },
    );
}

fn bench_dedup_duplicate(c: &mut Criterion) {
    let mut dedup = DedupTracker::new();
    dedup.check_and_insert(1, 0, 42);
    c.bench_function("dedup_check_duplicate", |b| {
        b.iter(|| dedup.check_and_insert(1, 0, 42));
    });
}

fn bench_dedup_cleanup(c: &mut Criterion) {
    c.bench_function(
        "dedup_cleanup_10k_entries",
        |b| {
            b.iter(|| {
                let mut dedup = DedupTracker::new();
                for i in 0..10_000_u64 {
                    dedup.check_and_insert(1, 0, i);
                }
                dedup.cleanup_with_cutoff(
                    std::time::Instant::now()
                        + std::time::Duration::from_secs(
                            1,
                        ),
                );
            });
        },
    );
}

// --- Orderbook benchmarks ---

fn bench_process_new_order_insert(c: &mut Criterion) {
    c.bench_function(
        "process_new_order_insert",
        |b| {
            let mut book = make_book();
            let mut seq = 0_u64;
            b.iter(|| {
                seq += 1;
                let px = 99_000 + (seq % 100) as i64;
                let mut order =
                    make_order(px, 10, Side::Buy, 1);
                order.order_id_lo = seq;
                process_new_order(
                    black_box(&mut book),
                    black_box(&mut order),
                );
            });
        },
    );
}

fn bench_process_new_order_match(c: &mut Criterion) {
    c.bench_function(
        "process_new_order_match",
        |b| {
            b.iter_batched(
                || {
                    let mut book = make_book();
                    let mut ask = make_order(
                        100_001, 10, Side::Sell, 2,
                    );
                    process_new_order(&mut book, &mut ask);
                    book
                },
                |mut book| {
                    let mut bid = make_order(
                        100_001, 10, Side::Buy, 1,
                    );
                    process_new_order(
                        black_box(&mut book),
                        black_box(&mut bid),
                    );
                },
                criterion::BatchSize::SmallInput,
            );
        },
    );
}

fn bench_cancel_order(c: &mut Criterion) {
    c.bench_function("cancel_order", |b| {
        b.iter_batched(
            || {
                let mut book = make_book();
                let handle = book.insert_resting(
                    99_500, 10, Side::Buy, 0, 1,
                    false, 1_000_000, 0, 0,
                );
                (book, handle)
            },
            |(mut book, handle)| {
                book.cancel_order(black_box(handle));
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn bench_drain_events_10_fills(c: &mut Criterion) {
    c.bench_function(
        "drain_events_10_fills",
        |b| {
            b.iter_batched(
                || {
                    let mut book = make_book();
                    for i in 0..10 {
                        let mut ask = make_order(
                            100_001 + i,
                            1,
                            Side::Sell,
                            2,
                        );
                        process_new_order(
                            &mut book, &mut ask,
                        );
                    }
                    let mut bid = make_order(
                        100_010, 10, Side::Buy, 1,
                    );
                    process_new_order(
                        &mut book, &mut bid,
                    );
                    book
                },
                |mut book| {
                    let events =
                        black_box(book.events());
                    for e in events {
                        black_box(e);
                    }
                    book.event_len = 0;
                },
                criterion::BatchSize::SmallInput,
            );
        },
    );
}

fn bench_drain_events_100_fills(c: &mut Criterion) {
    c.bench_function(
        "drain_events_100_fills",
        |b| {
            b.iter_batched(
                || {
                    let mut book = make_book();
                    for i in 0..100 {
                        let mut ask = make_order(
                            100_001 + i,
                            1,
                            Side::Sell,
                            2,
                        );
                        process_new_order(
                            &mut book, &mut ask,
                        );
                    }
                    let mut bid = make_order(
                        100_100, 100, Side::Buy, 1,
                    );
                    process_new_order(
                        &mut book, &mut bid,
                    );
                    book
                },
                |mut book| {
                    let events =
                        black_box(book.events());
                    for e in events {
                        black_box(e);
                    }
                    book.event_len = 0;
                },
                criterion::BatchSize::SmallInput,
            );
        },
    );
}

fn bench_wal_append_per_event(c: &mut Criterion) {
    let tmp = PathBuf::from("./tmp/bench_wal");
    let _ = std::fs::create_dir_all(&tmp);
    c.bench_function(
        "wal_append_per_event",
        |b| {
            let mut writer = WalWriter::new(
                99, &tmp, None, 64 * 1024 * 1024, 0,
            )
            .unwrap();
            let mut book = make_book();
            let mut ask = make_order(
                100_001, 10, Side::Sell, 2,
            );
            process_new_order(&mut book, &mut ask);
            let mut bid = make_order(
                100_001, 10, Side::Buy, 1,
            );
            process_new_order(&mut book, &mut bid);

            b.iter(|| {
                write_events_to_wal(
                    black_box(&mut writer),
                    black_box(&book),
                    1,
                    1_000_000,
                )
                .unwrap();
            });
        },
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

fn bench_price_to_index_bisection(c: &mut Criterion) {
    let cmap = CompressionMap::new(100_000, 1);
    c.bench_function(
        "price_to_index_bisection",
        |b| {
            let mut px = 95_000_i64;
            b.iter(|| {
                let idx = cmap.price_to_index(
                    black_box(px),
                );
                black_box(idx);
                px += 1;
                if px > 105_000 {
                    px = 95_000;
                }
            });
        },
    );
}

fn bench_slab_alloc_free(c: &mut Criterion) {
    c.bench_function("slab_alloc_free", |b| {
        let mut slab: Slab<OrderSlot> = Slab::new(1024);
        b.iter(|| {
            let h = slab.alloc();
            slab.free(black_box(h));
        });
    });
}

fn bench_smooshed_tick_match_k_orders(
    c: &mut Criterion,
) {
    let mut group =
        c.benchmark_group("smooshed_tick_match");
    for k in [1, 10, 50, 100] {
        group.bench_function(
            format!("k={}", k),
            |b| {
                b.iter_batched(
                    || {
                        let mut book = make_book();
                        let px = 100_001;
                        for i in 0..k {
                            let mut ask = make_order(
                                px,
                                1,
                                Side::Sell,
                                100 + i as u32,
                            );
                            process_new_order(
                                &mut book, &mut ask,
                            );
                        }
                        book
                    },
                    |mut book| {
                        let mut bid = make_order(
                            100_001,
                            k as i64,
                            Side::Buy,
                            1,
                        );
                        process_new_order(
                            black_box(&mut book),
                            black_box(&mut bid),
                        );
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_dedup_insert,
    bench_dedup_duplicate,
    bench_dedup_cleanup,
    bench_process_new_order_insert,
    bench_process_new_order_match,
    bench_cancel_order,
    bench_drain_events_10_fills,
    bench_drain_events_100_fills,
    bench_wal_append_per_event,
    bench_price_to_index_bisection,
    bench_slab_alloc_free,
    bench_smooshed_tick_match_k_orders,
);
criterion_main!(benches);
