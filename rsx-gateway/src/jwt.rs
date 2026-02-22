use jsonwebtoken::decode;
use jsonwebtoken::Algorithm;
use jsonwebtoken::DecodingKey;
use jsonwebtoken::Validation;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Deserialize, Serialize)]
pub struct Claims {
    pub sub: String,
    pub exp: u64,
    #[serde(default)]
    pub aud: Option<String>,
    #[serde(default)]
    pub iss: Option<String>,
}

pub fn validate_jwt(
    token: &str,
    secret: &str,
) -> Result<u32, String> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    validation.set_audience(&["rsx-gateway"]);
    validation.set_issuer(&["rsx-auth"]);

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|e| format!("jwt decode failed: {e}"))?;

    let user_id = token_data
        .claims
        .sub
        .parse::<u32>()
        .map_err(|e| format!("invalid user_id: {e}"))?;

    Ok(user_id)
}
