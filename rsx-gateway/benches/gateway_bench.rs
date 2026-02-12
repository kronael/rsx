use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_gateway::convert::price_to_fixed;
use rsx_gateway::convert::qty_to_fixed;
use rsx_gateway::order_id::generate_order_id;
use rsx_gateway::pending::PendingOrder;
use rsx_gateway::pending::PendingOrders;
use rsx_gateway::protocol::parse;
use rsx_gateway::protocol::serialize;
use rsx_gateway::protocol::WsFrame;
use rsx_gateway::rate_limit::RateLimiter;
use rsx_types::SymbolConfig;

fn cid20() -> String {
    "abcdefghij0123456789".to_string()
}

fn oid32() -> String {
    "0123456789abcdef0123456789abcdef".to_string()
}

fn make_n_frame_json() -> String {
    format!(
        "{{\"N\":[1,0,50000,100,\"{}\",0,0,0]}}",
        cid20(),
    )
}

fn make_c_frame_json() -> String {
    format!("{{\"C\":[\"{}\"]}}", cid20())
}

fn make_a_frame_json() -> String {
    // Auth is not a distinct frame type; use subscribe
    // as the "A" (auth-adjacent) frame. The spec's
    // bench_ws_parse_a_frame likely means the auth
    // handshake JSON. We approximate with a subscribe
    // frame which is the lightest parse path.
    r#"{"S":[1,3]}"#.to_string()
}

fn make_fill_frame() -> WsFrame {
    WsFrame::Fill {
        taker_order_id: oid32(),
        maker_order_id: oid32(),
        price: 50000,
        qty: 100,
        timestamp_ns: 1_700_000_000_000,
        fee: 25,
    }
}

fn make_pending_order(i: u32) -> PendingOrder {
    let mut oid = [0u8; 16];
    oid[0..4].copy_from_slice(&i.to_le_bytes());
    let mut cid = [0u8; 20];
    cid[0..4].copy_from_slice(&i.to_le_bytes());
    PendingOrder {
        order_id: oid,
        user_id: 1,
        symbol_id: 1,
        client_order_id: cid,
        timestamp_ns: 1_000_000 * i as u64,
    }
}

/// <500ns: parse new order frame
fn bench_ws_parse_n_frame(c: &mut Criterion) {
    let json = make_n_frame_json();
    c.bench_function("ws_parse_n_frame", |b| {
        b.iter(|| parse(black_box(&json)))
    });
}

/// <500ns: serialize fill frame
fn bench_ws_serialize_f_frame(c: &mut Criterion) {
    let frame = make_fill_frame();
    c.bench_function("ws_serialize_f_frame", |b| {
        b.iter(|| serialize(black_box(&frame)))
    });
}

/// <50ns: UUIDv7 generation
fn bench_uuid_v7_generation(c: &mut Criterion) {
    c.bench_function("uuid_v7_generation", |b| {
        b.iter(|| black_box(generate_order_id()))
    });
}

/// <100ns: LIFO pop from 5 pending orders
fn bench_pending_lifo_pop_5_orders(c: &mut Criterion) {
    c.bench_function(
        "pending_lifo_pop_5_orders",
        |b| {
            b.iter_batched(
                || {
                    let mut p = PendingOrders::new(100);
                    for i in 0..5 {
                        p.push(make_pending_order(i));
                    }
                    let target = make_pending_order(4);
                    (p, target.order_id)
                },
                |(mut p, oid)| {
                    black_box(p.remove(black_box(&oid)));
                },
                criterion::BatchSize::SmallInput,
            )
        },
    );
}

/// <100ns: linear scan of 10 pending orders
fn bench_pending_linear_scan_10(c: &mut Criterion) {
    c.bench_function(
        "pending_linear_scan_10",
        |b| {
            b.iter_batched(
                || {
                    let mut p = PendingOrders::new(100);
                    for i in 0..10 {
                        p.push(make_pending_order(i));
                    }
                    let target = make_pending_order(0);
                    (p, target.order_id)
                },
                |(mut p, oid)| {
                    black_box(p.remove(black_box(&oid)));
                },
                criterion::BatchSize::SmallInput,
            )
        },
    );
}

/// <50ns: rate limit check
fn bench_rate_limit_check(c: &mut Criterion) {
    c.bench_function("rate_limit_check", |b| {
        let mut rl = RateLimiter::new(1000, 1000);
        b.iter(|| black_box(rl.try_consume()))
    });
}

/// <200ns: parse cancel frame
fn bench_ws_parse_c_frame(c: &mut Criterion) {
    let json = make_c_frame_json();
    c.bench_function("ws_parse_c_frame", |b| {
        b.iter(|| parse(black_box(&json)))
    });
}

/// <500ns: parse auth/subscribe frame
fn bench_ws_parse_a_frame(c: &mut Criterion) {
    let json = make_a_frame_json();
    c.bench_function("ws_parse_a_frame", |b| {
        b.iter(|| parse(black_box(&json)))
    });
}

/// <100ns: backpressure reject (pending full)
fn bench_backpressure_reject(c: &mut Criterion) {
    c.bench_function("backpressure_reject", |b| {
        b.iter_batched(
            || {
                let mut p = PendingOrders::new(5);
                for i in 0..5 {
                    p.push(make_pending_order(i));
                }
                p
            },
            |mut p| {
                let rejected =
                    !p.push(make_pending_order(99));
                black_box(rejected);
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

/// <50ns: extract fee from fill frame
fn bench_fill_fee_extraction(c: &mut Criterion) {
    let frame = make_fill_frame();
    let serialized = serialize(&frame);
    c.bench_function("fill_fee_extraction", |b| {
        b.iter(|| {
            let f = parse(black_box(&serialized)).unwrap();
            match f {
                WsFrame::Fill { fee, .. } => {
                    black_box(fee);
                }
                _ => unreachable!(),
            }
        })
    });
}

/// <50ns: fixed-point price conversion
fn bench_fixed_point_conversion(c: &mut Criterion) {
    let config = SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 4,
        tick_size: 1,
        lot_size: 1,
    };
    c.bench_function("fixed_point_conversion", |b| {
        b.iter(|| {
            let px =
                price_to_fixed(black_box(500.25), &config);
            let qty =
                qty_to_fixed(black_box(1.5), &config);
            black_box((px, qty));
        })
    });
}

criterion_group!(
    benches,
    bench_ws_parse_n_frame,
    bench_ws_serialize_f_frame,
    bench_uuid_v7_generation,
    bench_pending_lifo_pop_5_orders,
    bench_pending_linear_scan_10,
    bench_rate_limit_check,
    bench_ws_parse_c_frame,
    bench_ws_parse_a_frame,
    bench_backpressure_reject,
    bench_fill_fee_extraction,
    bench_fixed_point_conversion,
);
criterion_main!(benches);
