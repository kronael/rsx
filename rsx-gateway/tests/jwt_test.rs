use jsonwebtoken::encode;
use jsonwebtoken::Algorithm;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::Header;
use rsx_gateway::jwt::validate_jwt;
use rsx_gateway::jwt::Claims;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn test_validate_jwt_valid() {
    let secret = "test-secret";
    let user_id = 12345u32;
    let exp = now_secs() + 3600;

    let claims = Claims {
        sub: user_id.to_string(),
        exp,
        aud: Some("rsx-gateway".to_string()),
        iss: Some("rsx-auth".to_string()),
    };

    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap();

    let result = validate_jwt(&token, secret);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), user_id);
}

#[test]
fn test_validate_jwt_expired() {
    let secret = "test-secret";
    let user_id = 12345u32;
    let exp = now_secs().saturating_sub(3600);

    let claims = Claims {
        sub: user_id.to_string(),
        exp,
        aud: Some("rsx-gateway".to_string()),
        iss: Some("rsx-auth".to_string()),
    };

    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap();

    let result = validate_jwt(&token, secret);
    assert!(result.is_err());
    let err = result.unwrap_err();
    eprintln!("Error message: {}", err);
    assert!(err.contains("expired") || err.contains("ExpiredSignature"));
}

#[test]
fn test_validate_jwt_invalid_secret() {
    let secret = "test-secret";
    let wrong_secret = "wrong-secret";
    let user_id = 12345u32;
    let exp = now_secs() + 3600;

    let claims = Claims {
        sub: user_id.to_string(),
        exp,
        aud: Some("rsx-gateway".to_string()),
        iss: Some("rsx-auth".to_string()),
    };

    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap();

    let result = validate_jwt(&token, wrong_secret);
    assert!(result.is_err());
}

#[test]
fn test_validate_jwt_invalid_user_id() {
    let secret = "test-secret";
    let exp = now_secs() + 3600;

    let claims = Claims {
        sub: "not-a-number".to_string(),
        exp,
        aud: Some("rsx-gateway".to_string()),
        iss: Some("rsx-auth".to_string()),
    };

    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap();

    let result = validate_jwt(&token, secret);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("invalid user_id")
    );
}

#[test]
fn test_validate_jwt_malformed() {
    let secret = "test-secret";
    let result = validate_jwt("not-a-jwt", secret);
    assert!(result.is_err());
}
