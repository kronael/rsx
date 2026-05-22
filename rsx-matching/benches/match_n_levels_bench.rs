//! `process_new_order` with an aggressive order that sweeps
//! N resting price levels. Exposes how the matching loop
//! scales — the existing 54ns single-fill baseline only
//! covers k=1, but production traders sweep many levels at
//! once.
//!
//! Each level has exactly one resting order of qty 1 at a
//! distinct price; the taker is sized to consume all of them
//! exactly (qty == n).
//!
//! Reported as `book_match_n_levels/n` for each n in
//! {1, 5, 20, 100}.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::BatchSize;
use criterion::Criterion;
use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 4,
        tick_size: 1,
        lot_size: 1,
    }
}

fn make_order(
    price: i64,
    qty: i64,
    side: Side,
    user_id: u32,
    oid: u64,
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
        order_id_lo: oid,
    }
}

fn book_with_resting_asks(n: usize) -> Orderbook {
    let mut book = Orderbook::new(config(), 65_536, 100_000);
    // n distinct price levels, one resting ask per level,
    // qty 1 each. Aggressive bid will sweep all of them.
    for i in 0..n {
        let mut ask = make_order(
            100_001 + i as i64,
            1,
            Side::Sell,
            200 + i as u32,
            2_000 + i as u64,
        );
        process_new_order(&mut book, &mut ask);
    }
    book
}

fn bench_match_n_levels(c: &mut Criterion) {
    let mut group = c.benchmark_group("book_match_n_levels");
    for n in [1usize, 5, 20, 100] {
        group.bench_function(format!("n={n}"), |b| {
            b.iter_batched(
                || book_with_resting_asks(n),
                |mut book| {
                    // Aggressor sweeps exactly n levels.
                    let mut bid = make_order(
                        100_000 + n as i64,
                        n as i64,
                        Side::Buy,
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

criterion_group!(benches, bench_match_n_levels);
criterion_main!(benches);
