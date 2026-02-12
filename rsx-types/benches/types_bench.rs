use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::SymbolConfig;
use rsx_types::validate_order;

fn bench_price_arithmetic(c: &mut Criterion) {
    let a = Price(50000);
    let b = Price(100);
    c.bench_function("price_add", |bench| {
        bench.iter(|| Price(a.0 + b.0));
    });
    c.bench_function("price_mul_qty", |bench| {
        let qty = Qty(100);
        bench.iter(|| a.0.checked_mul(qty.0));
    });
}

fn bench_validate_order(c: &mut Criterion) {
    let config = SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 100,
        lot_size: 10,
    };
    c.bench_function("validate_order_pass", |bench| {
        bench.iter(|| {
            validate_order(&config, Price(50000), Qty(100))
        });
    });
    c.bench_function("validate_order_fail_tick", |bench| {
        bench.iter(|| {
            validate_order(&config, Price(50001), Qty(100))
        });
    });
}

fn bench_notional_overflow_check(c: &mut Criterion) {
    c.bench_function("notional_checked_mul", |bench| {
        let price = 50000_i64;
        let qty = 1_000_000_i64;
        bench.iter(|| price.checked_mul(qty));
    });
    c.bench_function("notional_i128_mul", |bench| {
        let price = 50000_i64;
        let qty = 1_000_000_i64;
        bench.iter(|| (price as i128) * (qty as i128));
    });
}

criterion_group!(
    benches,
    bench_price_arithmetic,
    bench_validate_order,
    bench_notional_overflow_check
);
criterion_main!(benches);
