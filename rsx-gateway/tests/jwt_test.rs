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
        sub: format!("github:{user_id}"),
        user_id: Some(user_id),
        exp,
        aud: Some("rsx-gateway".to_string()),
        iss: Some("rsx-auth".to_string()),
        nbf: None,
        jti: None,
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
        sub: format!("github:{user_id}"),
        user_id: Some(user_id),
        exp,
        aud: Some("rsx-gateway".to_string()),
        iss: Some("rsx-auth".to_string()),
        nbf: None,
        jti: None,
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
        sub: format!("github:{user_id}"),
        user_id: Some(user_id),
        exp,
        aud: Some("rsx-gateway".to_string()),
        iss: Some("rsx-auth".to_string()),
        nbf: None,
        jti: None,
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
        user_id: None,
        exp,
        aud: Some("rsx-gateway".to_string()),
        iss: Some("rsx-auth".to_string()),
        nbf: None,
        jti: None,
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

#[test]
fn test_validate_jwt_rejects_nbf_in_future() {
    use jsonwebtoken::{encode, EncodingKey, Header, Algorithm};
    let secret = "a-secret-that-is-32-chars-long-padpadpad";
    let user_id = 7u32;
    let exp = now_secs() + 3600;
    // nbf 1 hour in the future — token not yet valid.
    let nbf = now_secs() + 3600;
    let claims = Claims {
        sub: format!("github:{user_id}"),
        user_id: Some(user_id),
        exp,
        aud: Some("rsx-gateway".to_string()),
        iss: Some("rsx-auth".to_string()),
        nbf: Some(nbf),
        jti: None,
    };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap();
    let result = validate_jwt(&token, secret);
    assert!(
        result.is_err(),
        "nbf in the future must reject token: {result:?}"
    );
}

#[test]
fn test_jti_tracker_rejects_replay() {
    use rsx_gateway::jwt::JtiTracker;
    let mut t = JtiTracker::new(8);
    assert!(t.record(Some("abc")));
    assert!(t.record(Some("def")));
    assert!(!t.record(Some("abc"))); // replay
    assert!(t.record(None)); // tokens without jti always pass
}

#[test]
fn test_jti_tracker_evicts_oldest_when_full() {
    use rsx_gateway::jwt::JtiTracker;
    let mut t = JtiTracker::new(2);
    assert!(t.record(Some("a")));
    assert!(t.record(Some("b")));
    assert!(t.record(Some("c"))); // evicts "a"
    // "a" is now fresh again because it was evicted.
    assert!(t.record(Some("a")));
    assert_eq!(t.len(), 2);
}
