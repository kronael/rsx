//! JWT validation cost (HS256 + aud + iss + exp + jti).
//! This is what `extract_user_and_record_jti` runs once per
//! WS handshake — the gateway path's first heavy operation
//! after the TCP/WS upgrade.
//!
//! Setup mints a real HS256 token via `jsonwebtoken::encode`
//! at bench start; the iter body times `validate_jwt_with_claims`
//! against that token.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use jsonwebtoken::encode;
use jsonwebtoken::Algorithm;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::Header;
use rsx_gateway::jwt::validate_jwt_with_claims;
use rsx_gateway::jwt::Claims;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const SECRET: &str = "bench-secret-padded-to-32-bytes-min!";

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn mint_token(jti: Option<String>) -> String {
    let claims = Claims {
        sub: "github:12345".to_string(),
        user_id: Some(12345),
        exp: now_secs() + 3600,
        nbf: None,
        jti,
        aud: Some("rsx-gateway".to_string()),
        iss: Some("rsx-auth".to_string()),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(SECRET.as_bytes()),
    )
    .expect("encode")
}

/// Validation without jti — the common case for short-lived
/// gateway tokens. Times the HS256 verify + claim parse.
fn bench_validate_no_jti(c: &mut Criterion) {
    let token = mint_token(None);
    c.bench_function("jwt_validate_no_jti", |b| {
        b.iter(|| {
            let r = validate_jwt_with_claims(black_box(&token), black_box(SECRET));
            black_box(r.unwrap());
        });
    });
}

/// Validation with jti — same path plus the jti claim parse.
fn bench_validate_with_jti(c: &mut Criterion) {
    let token = mint_token(Some("01HXYZ1234567890ABCDEF".to_string()));
    c.bench_function("jwt_validate_with_jti", |b| {
        b.iter(|| {
            let r = validate_jwt_with_claims(black_box(&token), black_box(SECRET));
            black_box(r.unwrap());
        });
    });
}

criterion_group!(benches, bench_validate_no_jti, bench_validate_with_jti,);
criterion_main!(benches);
