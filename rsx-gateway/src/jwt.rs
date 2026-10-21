use jsonwebtoken::decode;
use jsonwebtoken::Algorithm;
use jsonwebtoken::DecodingKey;
use jsonwebtoken::Validation;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Deserialize, Serialize)]
pub struct Claims {
    pub sub: String,
    #[serde(default)]
    pub user_id: Option<u32>,
    pub exp: u64,
    /// "Not before" — token rejected if `now < nbf`. Optional
    /// in the spec; when present, we enforce it via `Validation`.
    #[serde(default)]
    pub nbf: Option<u64>,
    #[serde(default)]
    pub aud: Option<String>,
    #[serde(default)]
    pub iss: Option<String>,
}

/// Validate a JWT and return the user_id it carries.
///
/// A token stays valid for every connection until `exp`; no per-token
/// state is kept. Enforces:
/// - HS256 signature with the supplied secret
/// - `exp` (expiry)
/// - `nbf` (not-before, when present in the token)
/// - `aud == "rsx-gateway"`
/// - `iss == "rsx-auth"`
pub fn validate_jwt(token: &str, secret: &str) -> Result<u32, String> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    validation.validate_nbf = true;
    validation.set_audience(&["rsx-gateway"]);
    validation.set_issuer(&["rsx-auth"]);

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
