use jsonwebtoken::decode;
use jsonwebtoken::Algorithm;
use jsonwebtoken::DecodingKey;
use jsonwebtoken::Validation;
use rsx_messages::Claims;
use rsx_messages::JWT_AUDIENCE;
use rsx_messages::JWT_ISSUER;

/// Validate a JWT and return the user_id it carries.
///
/// A token stays valid for every connection until `exp`; no per-token
/// state is kept. Enforces:
/// - HS256 signature with the supplied secret
/// - `exp` (expiry)
/// - `nbf` (not-before, when present in the token)
/// - `aud == JWT_AUDIENCE`
/// - `iss == JWT_ISSUER`
pub fn validate_jwt(token: &str, secret: &str) -> Result<u32, String> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    validation.validate_nbf = true;
    validation.set_audience(&[JWT_AUDIENCE]);
    validation.set_issuer(&[JWT_ISSUER]);

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|e| format!("jwt decode failed: {e}"))?;

    match token_data.claims.user_id {
        Some(user_id) => Ok(user_id),
        None => token_data
            .claims
            .sub
            .parse::<u32>()
            .map_err(|e| format!("invalid user_id: {e}")),
    }
}
