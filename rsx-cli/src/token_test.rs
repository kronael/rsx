use crate::output::Failure;
use crate::token::mint_for;
use crate::token::minted_json;
use serde_json::json;

const SECRET: &str = "test-secret-padded-to-32-bytes-minlen!";

#[test]
fn test_mint_expires_ttl_after_now() {
    let minted = mint_for(7, 300, Some(SECRET), 1_000).unwrap();
    assert_eq!(minted.user_id, 7);
    assert_eq!(minted.exp, 1_300);
    assert_eq!(minted.token.split('.').count(), 3);
}

#[test]
fn test_minted_json_has_token_user_id_and_exp() {
    let minted = mint_for(7, 300, Some(SECRET), 1_000).unwrap();
    assert_eq!(
        minted_json(&minted),
        json!({"token": minted.token, "user_id": 7, "exp": 1_300})
    );
}

#[test]
fn test_missing_or_empty_secret_is_misuse() {
    for secret in [None, Some("")] {
        let failure = mint_for(7, 300, secret, 1_000).err().unwrap();
        assert!(matches!(failure, Failure::Misuse(_)), "{failure:?}");
    }
}

#[test]
fn test_ttl_overflowing_the_expiry_is_misuse() {
    let failure = mint_for(7, u64::MAX, Some(SECRET), 1_000).err().unwrap();
    assert!(matches!(failure, Failure::Misuse(_)), "{failure:?}");
}
