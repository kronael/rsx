use rsx_gateway::jwt::validate_jwt;
use rsx_messages::mint_jwt;
use rsx_messages::Claims;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const SECRET: &str = "test-secret-padded-to-32-bytes-minlen!";

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn test_validate_jwt_valid() {
    let user_id = 12345u32;
    let claims = Claims::for_user(user_id, now_secs() + 3600);
    let token = mint_jwt(&claims, SECRET).unwrap();

    let result = validate_jwt(&token, SECRET);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), user_id);
}

#[test]
fn test_validate_jwt_expired() {
    let claims = Claims::for_user(12345, now_secs().saturating_sub(3600));
    let token = mint_jwt(&claims, SECRET).unwrap();

    let result = validate_jwt(&token, SECRET);
    assert!(result.is_err());
    let err = result.unwrap_err();
    eprintln!("Error message: {}", err);
    assert!(err.contains("expired") || err.contains("ExpiredSignature"));
}

#[test]
fn test_validate_jwt_invalid_secret() {
    let claims = Claims::for_user(12345, now_secs() + 3600);
    let token = mint_jwt(&claims, SECRET).unwrap();

    let result = validate_jwt(&token, "wrong-secret");
    assert!(result.is_err());
}

#[test]
fn test_validate_jwt_wrong_audience() {
    let claims = Claims {
        aud: Some("rsx-marketdata".to_string()),
        ..Claims::for_user(12345, now_secs() + 3600)
    };
    let token = mint_jwt(&claims, SECRET).unwrap();

    let err = validate_jwt(&token, SECRET).unwrap_err();
    assert!(err.contains("InvalidAudience"), "{err}");
}

#[test]
fn test_validate_jwt_wrong_issuer() {
    let claims = Claims {
        iss: Some("someone-else".to_string()),
        ..Claims::for_user(12345, now_secs() + 3600)
    };
    let token = mint_jwt(&claims, SECRET).unwrap();

    let err = validate_jwt(&token, SECRET).unwrap_err();
    assert!(err.contains("InvalidIssuer"), "{err}");
}

#[test]
fn test_validate_jwt_invalid_user_id() {
    let claims = Claims {
        sub: "not-a-number".to_string(),
        user_id: None,
        ..Claims::for_user(0, now_secs() + 3600)
    };
    let token = mint_jwt(&claims, SECRET).unwrap();

    let result = validate_jwt(&token, SECRET);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("invalid user_id"));
}

#[test]
fn test_validate_jwt_malformed() {
    let result = validate_jwt("not-a-jwt", SECRET);
    assert!(result.is_err());
}

#[test]
fn test_validate_jwt_rejects_nbf_in_future() {
    let claims = Claims {
        nbf: Some(now_secs() + 3600),
        ..Claims::for_user(7, now_secs() + 3600)
    };
    let token = mint_jwt(&claims, SECRET).unwrap();

    let result = validate_jwt(&token, SECRET);
    assert!(
        result.is_err(),
        "nbf in the future must reject token: {result:?}"
    );
}
