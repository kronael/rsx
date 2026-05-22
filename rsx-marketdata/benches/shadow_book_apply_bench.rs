//! Feed real CMP wire records through the marketdata
//! shadow-book apply path. This is the consumer-side cost
//! of every CMP packet ME sends to MD: parse + look up the
//! order in the shadow's `order_map` + apply to the
//! internal `Orderbook` + bump seq.
//!
//! The private `handle_insert / handle_cancel / handle_fill`
//! in `rsx-marketdata/src/main.rs` ultimately call exactly
//! the pub methods we hit here: `apply_insert_by_id`,
//! `apply_cancel_by_order_id`, `apply_fill_by_order_id`.
//! Those methods are the production code path; the wrapper
//! functions only dispatch and broadcast. Broadcast cost is
//! out of scope for "isolated shadow-book apply".

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::BatchSize;
use criterion::Criterion;
use rsx_marketdata::shadow::ShadowBook;
use rsx_types::SymbolConfig;

const SYMBOL_ID: u32 = 1;

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: SYMBOL_ID,
        price_decimals: 2,
        qty_decimals: 4,
        tick_size: 1,
        lot_size: 1,
    }
}

fn populated_book(n_resting: usize) -> ShadowBook {
    let mut book = ShadowBook::new(config(), 65_536, 100_000);
    for i in 0..n_resting {
        book.apply_insert_by_id(
            99_500 - i as i64,
            100,
            0, // buy
            1000 + i as u32,
            1_700_000_000_000 + i as u64,
            0,
            10_000 + i as u64,
        );
        book.apply_insert_by_id(
            100_500 + i as i64,
            100,
            1, // sell
            2000 + i as u32,
            1_700_000_000_000 + i as u64,
            0,
            20_000 + i as u64,
        );
    }
    book
}

/// Insert path — fresh order ID each iter; mirrors the
/// per-record cost when ME emits OrderInserted. Insert is
/// paired with cancel inside iter_batched so the slab
/// doesn't exhaust over Criterion's many-iter campaigns.
fn bench_apply_insert(c: &mut Criterion) {
    c.bench_function("shadow_apply_insert", |b| {
        let mut book = populated_book(50);
        let mut counter: u64 = 100_000;
        b.iter(|| {
            counter += 1;
            let oid_lo = counter;
            book.apply_insert_by_id(
                black_box(99_400),
                black_box(10),
                black_box(0),
                black_box(1),
                black_box(1_700_000_000_000),
                black_box(0),
                black_box(oid_lo),
            );
            // Drop the order so slab capacity stays bounded.
            // Cancel cost is benched separately below.
            book.apply_cancel_by_order_id(
                0,
                oid_lo,
                1_700_000_000_001,
            );
        });
    });
}

/// Fill path: maker order is looked up by oid, qty is
/// decremented (or order removed if filled). The hot path
/// every trade hits.
fn bench_apply_fill(c: &mut Criterion) {
    c.bench_function("shadow_apply_fill", |b| {
        b.iter_batched(
            || {
                let mut book = populated_book(10);
                // Pre-insert a known fillable order.
                book.apply_insert_by_id(
                    99_490,
                    1_000_000_000,
                    0,
                    9_999,
                    1_700_000_000_000,
                    0,
                    99_999,
                );
                book
            },
            |mut book| {
                let r = book.apply_fill_by_order_id(
                    black_box(0),
                    black_box(99_999),
                    black_box(1),
                    black_box(1_700_000_000_001),
                );
                black_box(r);
            },
            BatchSize::SmallInput,
        );
    });
}

/// Cancel path: full removal of a resting order.
fn bench_apply_cancel(c: &mut Criterion) {
    c.bench_function("shadow_apply_cancel", |b| {
        b.iter_batched(
            || {
                let mut book = populated_book(10);
                book.apply_insert_by_id(
                    99_490, 100, 0, 9_999,
                    1_700_000_000_000, 0, 99_999,
                );
                book
            },
            |mut book| {
                let r = book.apply_cancel_by_order_id(
                    black_box(0),
                    black_box(99_999),
                    black_box(1_700_000_000_001),
                );
                black_box(r);
            },
            BatchSize::SmallInput,
        );
    });
}

/// Derive BBO after apply — the production loop emits this
/// to all subscribers. Already covered by marketdata_bench,
/// but reproduced here against a populated book to give the
/// integration cost when chained after apply_*.
fn bench_apply_insert_then_derive_bbo(
    c: &mut Criterion,
) {
    c.bench_function(
        "shadow_apply_insert_then_bbo",
        |b| {
            let mut book = populated_book(20);
            let mut counter: u64 = 500_000;
            b.iter(|| {
                counter += 1;
                let oid_lo = counter;
                book.apply_insert_by_id(
                    99_400, 10, 0, 1,
                    1_700_000_000_000, 0, oid_lo,
                );
                let bbo = book.derive_bbo();
                black_box(bbo);
                book.apply_cancel_by_order_id(
                    0, oid_lo, 1_700_000_000_001,
                );
            });
        },
    );
}

criterion_group!(
    benches,
    bench_apply_insert,
    bench_apply_fill,
    bench_apply_cancel,
    bench_apply_insert_then_derive_bbo,
);
criterion_main!(benches);
