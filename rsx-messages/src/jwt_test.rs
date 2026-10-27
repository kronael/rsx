use crate::jwt::mint_jwt;
use crate::jwt::Claims;
use crate::jwt::JWT_AUDIENCE;
use crate::jwt::JWT_ISSUER;
use jsonwebtoken::decode;
use jsonwebtoken::Algorithm;
use jsonwebtoken::DecodingKey;
use jsonwebtoken::Validation;
use serde_json::Value;

const SECRET: &str = "test-secret-padded-to-32-bytes-minlen!";

fn decode_claims(token: &str, secret: &str) -> Result<Claims, String> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&[JWT_AUDIENCE]);
    validation.set_issuer(&[JWT_ISSUER]);
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|error| error.to_string())
}

#[test]
fn test_mint_round_trips_claims() {
    let claims = Claims::for_user(42, 4_000_000_000);
    let token = mint_jwt(&claims, SECRET).unwrap();

    let decoded = decode_claims(&token, SECRET).unwrap();
    assert_eq!(decoded.user_id, Some(42));
    assert_eq!(decoded.sub, "42");
    assert_eq!(decoded.exp, 4_000_000_000);
    assert_eq!(decoded.aud.as_deref(), Some(JWT_AUDIENCE));
    assert_eq!(decoded.iss.as_deref(), Some(JWT_ISSUER));
}

#[test]
fn test_mint_is_signed_with_the_secret() {
    let token = mint_jwt(&Claims::for_user(1, 4_000_000_000), SECRET).unwrap();
    assert!(decode_claims(&token, "another-secret-padded-to-32-bytes!").is_err());
}

#[test]
fn test_absent_claims_are_left_out_of_the_token() {
    let claims = Claims {
        user_id: None,
        aud: None,
        iss: None,
        ..Claims::for_user(1, 4_000_000_000)
    };
    let token = mint_jwt(&claims, SECRET).unwrap();

    let mut validation = Validation::new(Algorithm::HS256);
    validation.insecure_disable_signature_validation();
    validation.validate_aud = false;
    let payload = decode::<Value>(&token, &DecodingKey::from_secret(b""), &validation)
        .unwrap()
        .claims;
    let mut keys: Vec<&str> = payload
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(keys, ["exp", "sub"]);
}
