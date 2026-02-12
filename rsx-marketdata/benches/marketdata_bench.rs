use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_marketdata::shadow::ShadowBook;
use rsx_marketdata::subscription::SubscriptionManager;
use rsx_marketdata::subscription::CHANNEL_BBO;
use rsx_marketdata::subscription::CHANNEL_DEPTH;
use rsx_marketdata::subscription::CHANNEL_TRADES;
use rsx_marketdata::types::BboUpdate;
use rsx_types::SymbolConfig;

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 0,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    }
}

fn new_book() -> ShadowBook {
    ShadowBook::new(config(), 4096, 50000)
}

fn populated_book(levels: usize) -> ShadowBook {
    let mut book = new_book();
    for i in 0..levels {
        book.apply_insert(
            49990 - i as i64, 100, 0, 1, 1000 + i as u64,
        );
        book.apply_insert(
            50010 + i as i64, 100, 1, 2, 2000 + i as u64,
        );
    }
    book
}

/// bench_shadow_book_insert (<500ns)
fn bench_shadow_book_insert(c: &mut Criterion) {
    c.bench_function("shadow_book_insert", |b| {
        let mut book = new_book();
        let mut i = 0u64;
        b.iter(|| {
            i += 1;
            let price = 49000 + (i % 1000) as i64;
            black_box(book.apply_insert(
                black_box(price),
                black_box(100),
                black_box(0),
                black_box(1),
                black_box(i),
            ));
        });
    });
}

/// bench_shadow_book_fill (<500ns)
fn bench_shadow_book_fill(c: &mut Criterion) {
    c.bench_function("shadow_book_fill", |b| {
        b.iter_custom(|iters| {
            let mut total = std::time::Duration::ZERO;
            for batch in 0..iters {
                let mut book = new_book();
                let h = book.apply_insert(
                    49990, 1_000_000_000, 0, 1, 1000,
                );
                let start = std::time::Instant::now();
                book.apply_fill(
                    black_box(h),
                    black_box(1),
                    black_box(0),
                    black_box(2000 + batch),
                );
                total += start.elapsed();
            }
            total
        });
    });
}

/// bench_bbo_derivation (<100ns)
fn bench_bbo_derivation(c: &mut Criterion) {
    let book = populated_book(10);
    c.bench_function("bbo_derivation", |b| {
        b.iter(|| {
            black_box(book.derive_bbo());
        });
    });
}

/// bench_l2_snapshot_10_levels (<1us)
fn bench_l2_snapshot_10_levels(c: &mut Criterion) {
    let book = populated_book(20);
    c.bench_function("l2_snapshot_10_levels", |b| {
        b.iter(|| {
            black_box(book.derive_l2_snapshot(black_box(10)));
        });
    });
}

/// bench_l2_snapshot_50_levels (<5us)
fn bench_l2_snapshot_50_levels(c: &mut Criterion) {
    let book = populated_book(60);
    c.bench_function("l2_snapshot_50_levels", |b| {
        b.iter(|| {
            black_box(
                book.derive_l2_snapshot(black_box(50)),
            );
        });
    });
}

/// bench_l2_delta_generation (<200ns)
fn bench_l2_delta_generation(c: &mut Criterion) {
    let book = populated_book(10);
    c.bench_function("l2_delta_generation", |b| {
        b.iter(|| {
            black_box(
                book.derive_l2_delta(black_box(0), black_box(49990)),
            );
        });
    });
}

/// bench_event_processing_throughput (>100K events/sec)
fn bench_event_processing_throughput(c: &mut Criterion) {
    c.bench_function("event_processing_throughput", |b| {
        b.iter_custom(|iters| {
            let mut book = new_book();
            let start = std::time::Instant::now();
            for i in 0..iters {
                let price = 49000 + (i % 1000) as i64;
                let side = (i % 2) as u8;
                let h = book.apply_insert(
                    price, 100, side, 1, i,
                );
                book.apply_fill(h, 50, side, i + 1);
                let _ = book.derive_bbo();
            }
            start.elapsed()
        });
    });
}

/// bench_ws_serialize_bbo (<500ns)
/// BboUpdate does not derive Serialize; manual JSON
/// formatting matches the wire protocol pattern.
fn bench_ws_serialize_bbo(c: &mut Criterion) {
    let bbo = BboUpdate {
        symbol_id: 1,
        bid_px: 49990,
        bid_qty: 500,
        bid_count: 5,
        ask_px: 50010,
        ask_qty: 300,
        ask_count: 3,
        timestamp_ns: 1_700_000_000_000,
        seq: 42,
    };
    c.bench_function("ws_serialize_bbo", |b| {
        b.iter(|| {
            let json = format!(
                "{{\"B\":[{},{},{},{},{},{},{},{},{}]}}",
                bbo.symbol_id,
                bbo.bid_px,
                bbo.bid_qty,
                bbo.bid_count,
                bbo.ask_px,
                bbo.ask_qty,
                bbo.ask_count,
                bbo.timestamp_ns,
                bbo.seq,
            );
            black_box(json);
        });
    });
}

/// bench_trade_derivation_from_fill (<100ns)
fn bench_trade_derivation_from_fill(c: &mut Criterion) {
    let book = populated_book(5);
    c.bench_function("trade_derivation_from_fill", |b| {
        b.iter(|| {
            black_box(book.make_trade(
                black_box(49990),
                black_box(50),
                black_box(1),
                black_box(1_000_000),
            ));
        });
    });
}

/// bench_event_routing_filter (<50ns)
fn bench_event_routing_filter(c: &mut Criterion) {
    let mut mgr = SubscriptionManager::new();
    mgr.subscribe(1, 1, CHANNEL_BBO | CHANNEL_TRADES, 10);
    mgr.subscribe(2, 1, CHANNEL_DEPTH, 25);
    mgr.subscribe(3, 1, CHANNEL_BBO | CHANNEL_DEPTH, 10);
    mgr.subscribe(4, 2, CHANNEL_BBO, 10);
    c.bench_function("event_routing_filter", |b| {
        b.iter(|| {
            let has = mgr.has_bbo(black_box(1), black_box(1));
            black_box(has);
            let has = mgr.has_depth(black_box(2), black_box(1));
            black_box(has);
            let has =
                mgr.has_trades(black_box(3), black_box(1));
            black_box(has);
        });
    });
}

criterion_group!(
    benches,
    bench_shadow_book_insert,
    bench_shadow_book_fill,
    bench_bbo_derivation,
    bench_l2_snapshot_10_levels,
    bench_l2_snapshot_50_levels,
    bench_l2_delta_generation,
    bench_event_processing_throughput,
    bench_ws_serialize_bbo,
    bench_trade_derivation_from_fill,
    bench_event_routing_filter,
);
criterion_main!(benches);
