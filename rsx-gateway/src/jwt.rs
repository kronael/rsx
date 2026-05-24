use jsonwebtoken::decode;
use jsonwebtoken::Algorithm;
use jsonwebtoken::DecodingKey;
use jsonwebtoken::Validation;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashSet;
use std::collections::VecDeque;

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
    /// JWT ID — used by [`JtiTracker`] to reject replays of a
    /// given token. Optional; when present, the gateway
    /// remembers it for `WINDOW` seconds.
    #[serde(default)]
    pub jti: Option<String>,
    #[serde(default)]
    pub aud: Option<String>,
    #[serde(default)]
    pub iss: Option<String>,
}

/// Validate a JWT and return the user_id it carries.
/// Convenience wrapper around [`validate_jwt_with_claims`] for
/// callers that don't need the full claim set.
pub fn validate_jwt(
    token: &str,
    secret: &str,
) -> Result<u32, String> {
    validate_jwt_with_claims(token, secret).map(|(uid, _)| uid)
}

/// Validate a JWT and return both user_id and the parsed
/// claims (for callers that need `jti`, `nbf`, etc.).
///
/// Enforces:
/// - HS256 signature with the supplied secret
/// - `exp` (expiry)
/// - `nbf` (not-before, when present in the token)
/// - `aud == "rsx-gateway"`
/// - `iss == "rsx-auth"`
pub fn validate_jwt_with_claims(
    token: &str,
    secret: &str,
) -> Result<(u32, Claims), String> {
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

    let user_id = match token_data.claims.user_id {
        Some(user_id) => user_id,
        None => token_data
            .claims
            .sub
            .parse::<u32>()
            .map_err(|e| format!("invalid user_id: {e}"))?,
    };

    Ok((user_id, token_data.claims))
}

/// Bounded jti replay tracker. Remembers the last `cap` jti
/// values; rejecting any token whose jti is already in the
/// set. FIFO eviction once full.
///
/// This is a lightweight defence against token replay within
/// a single gateway process. It does **not** survive restarts
/// (token replay across a restart is allowed) and is not
/// shared across gateway replicas. For multi-replica replay
/// protection use a centralized cache (Redis) keyed by jti.
pub struct JtiTracker {
    seen: HashSet<String>,
    order: VecDeque<String>,
    cap: usize,
}

impl JtiTracker {
    pub fn new(cap: usize) -> Self {
        Self {
            seen: HashSet::with_capacity(cap),
            order: VecDeque::with_capacity(cap),
            cap,
        }
    }

    /// Returns true if this jti is fresh (and records it);
    /// false if it has been seen before within the window.
    /// Tokens without a `jti` claim always pass — they are
    /// caller-responsibility (typically short-lived `exp`).
    pub fn record(&mut self, jti: Option<&str>) -> bool {
        let Some(jti) = jti else {
            return true;
        };
        if self.seen.contains(jti) {
            return false;
        }
        if self.order.len() >= self.cap {
            if let Some(oldest) = self.order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        let owned = jti.to_string();
        self.order.push_back(owned.clone());
        self.seen.insert(owned);
        true
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    /// Roll back a previously-recorded jti. Used when a handshake
    /// fails AFTER `record` succeeded (e.g. the 101 response write
    /// failed) so the same jti can be retried by the client. See
    /// CTO-REPORT.md R-N5.
    pub fn rollback(&mut self, jti: &str) {
        if self.seen.remove(jti) {
            if let Some(pos) =
                self.order.iter().position(|x| x == jti)
            {
                self.order.remove(pos);
            }
        }
    }
}
