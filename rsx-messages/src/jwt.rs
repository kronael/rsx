//! The gateway's JWT claim set and the one HS256 mint. The gateway validates
//! a token against [`JWT_AUDIENCE`] and [`JWT_ISSUER`] with the same
//! [`Claims`] that [`mint_jwt`] signs, so a minted token and its validation
//! cannot drift. A token stays valid for every connection until `exp`; no
//! per-token state is kept.

use jsonwebtoken::encode;
use jsonwebtoken::Algorithm;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::Header;
use serde::Deserialize;
use serde::Serialize;

#[cfg(test)]
#[path = "jwt_test.rs"]
mod jwt_test;

/// The `aud` claim the gateway accepts.
pub const JWT_AUDIENCE: &str = "rsx-gateway";

/// The `iss` claim the gateway accepts.
pub const JWT_ISSUER: &str = "rsx-auth";

/// JWT claims. `exp` and `nbf` are Unix seconds. The user is `user_id` when
/// present, else `sub` parsed as a `u32`. An absent `Option` claim is left
/// out of the token.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Claims {
    pub sub: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<u32>,
    pub exp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nbf: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
}

impl Claims {
    /// The claims the gateway accepts for `user_id` until `exp`.
    pub fn for_user(user_id: u32, exp: u64) -> Self {
        Claims {
            sub: user_id.to_string(),
            user_id: Some(user_id),
            exp,
            nbf: None,
            aud: Some(JWT_AUDIENCE.to_string()),
            iss: Some(JWT_ISSUER.to_string()),
        }
    }
}

/// Signs `claims` as an HS256 JWT with `secret`. The only place in the
/// workspace that builds an `EncodingKey`.
pub fn mint_jwt(claims: &Claims, secret: &str) -> Result<String, String> {
    encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|error| format!("jwt encode failed: {error}"))
}
