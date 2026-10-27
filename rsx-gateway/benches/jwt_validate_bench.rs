//! JWT validation cost (HS256 + aud + iss + exp).
//! This is what `ws_handshake` runs once per WS handshake — the
//! gateway path's first heavy operation after the TCP/WS upgrade.
//!
//! Setup mints a real HS256 token via `rsx_messages::mint_jwt`
//! at bench start; the iter body times `validate_jwt` against
//! that token.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_gateway::jwt::validate_jwt;
use rsx_messages::mint_jwt;
use rsx_messages::Claims;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const SECRET: &str = "bench-secret-padded-to-32-bytes-min!";

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn mint_token() -> String {
    let claims = Claims::for_user(12345, now_secs() + 3600);
    mint_jwt(&claims, SECRET).expect("encode")
}

/// Times the HS256 verify + claim parse.
fn bench_validate(c: &mut Criterion) {
    let token = mint_token();
    c.bench_function("jwt_validate", |b| {
        b.iter(|| {
            let r = validate_jwt(black_box(&token), black_box(SECRET));
            black_box(r.unwrap());
        });
    });
}

criterion_group!(benches, bench_validate);
criterion_main!(benches);
