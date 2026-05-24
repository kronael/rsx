//! WS-frame JSON encode path. Times the production
//! `protocol::serialize` call that route_fill / route_update /
//! the heartbeat broadcaster all use to format outbound
//! frames. Each bench uses a realistic record shape (32-char
//! UUID order ids, real fee, real ms timestamp).

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_gateway::records::serialize;
use rsx_gateway::records::WsFrame;

fn oid32() -> String {
    "0123456789abcdef0123456789abcdef".to_string()
}

fn fill_frame() -> WsFrame {
    WsFrame::Fill {
        taker_order_id: oid32(),
        maker_order_id: oid32(),
        price: 5_000_000,
        qty: 100_000,
        timestamp_ns: 1_700_000_000_000_000_000,
        fee: 250,
    }
}

fn order_update_frame() -> WsFrame {
    WsFrame::OrderUpdate {
        order_id: oid32(),
        status: 1,
        filled_qty: 50_000,
        remaining_qty: 50_000,
        reason: 0,
    }
}

fn heartbeat_frame() -> WsFrame {
    WsFrame::Heartbeat {
        timestamp_ms: 1_700_000_000_000,
    }
}

fn bench_encode_fill(c: &mut Criterion) {
    let frame = fill_frame();
    c.bench_function("ws_encode_fill", |b| {
        b.iter(|| {
            let s = serialize(black_box(&frame));
            black_box(s);
        });
    });
}

fn bench_encode_order_update(c: &mut Criterion) {
    let frame = order_update_frame();
    c.bench_function("ws_encode_order_update", |b| {
        b.iter(|| {
            let s = serialize(black_box(&frame));
            black_box(s);
        });
    });
}

fn bench_encode_heartbeat(c: &mut Criterion) {
    let frame = heartbeat_frame();
    c.bench_function("ws_encode_heartbeat", |b| {
        b.iter(|| {
            let s = serialize(black_box(&frame));
            black_box(s);
        });
    });
}

criterion_group!(
    benches,
    bench_encode_fill,
    bench_encode_order_update,
    bench_encode_heartbeat,
);
criterion_main!(benches);
