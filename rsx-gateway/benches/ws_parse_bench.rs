//! WS-frame JSON parse path: NewOrder (N), Cancel (C),
//! Heartbeat (H). Drives the real `protocol::parse` —
//! `serde_json::from_str` plus the typed dispatch — with
//! realistic 20-char cids and the same field layout the
//! production gateway hands `parse_new_order`.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_gateway::records::parse;

/// 20-char cid, matches the spec's cid length cap.
fn cid20() -> String {
    "abcdefghij0123456789".to_string()
}

/// N frame: full 8-field shape (sym, side, px, qty, cid,
/// tif, reduce_only, post_only). Sized to match what the WS
/// client actually emits.
fn new_order_frame() -> String {
    format!("{{\"N\":[1,0,5000000,100000,\"{}\",0,0,0]}}", cid20(),)
}

/// C frame: cancel by 20-char client_order_id.
fn cancel_frame() -> String {
    format!("{{\"C\":[\"{}\"]}}", cid20())
}

/// H frame: heartbeat with a 13-digit ms timestamp.
fn heartbeat_frame() -> String {
    "{\"H\":[1700000000000]}".to_string()
}

fn bench_parse_new_order(c: &mut Criterion) {
    let json = new_order_frame();
    c.bench_function("ws_parse_new_order", |b| {
        b.iter(|| {
            let r = parse(black_box(&json));
            black_box(r.unwrap());
        });
    });
}

fn bench_parse_cancel(c: &mut Criterion) {
    let json = cancel_frame();
    c.bench_function("ws_parse_cancel", |b| {
        b.iter(|| {
            let r = parse(black_box(&json));
            black_box(r.unwrap());
        });
    });
}

fn bench_parse_heartbeat(c: &mut Criterion) {
    let json = heartbeat_frame();
    c.bench_function("ws_parse_heartbeat", |b| {
        b.iter(|| {
            let r = parse(black_box(&json));
            black_box(r.unwrap());
        });
    });
}

criterion_group!(
    benches,
    bench_parse_new_order,
    bench_parse_cancel,
    bench_parse_heartbeat,
);
criterion_main!(benches);
