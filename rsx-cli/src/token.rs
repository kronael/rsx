use crate::output::Failure;
use crate::output::Outcome;
use rsx_messages::mint_jwt;
use rsx_messages::Claims;
use serde_json::json;
use serde_json::Value;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

#[cfg(test)]
#[path = "token_test.rs"]
mod token_test;

/// A minted gateway token and the claims a caller needs to report.
pub struct Minted {
    pub token: String,
    pub user_id: u32,
    pub exp: u64,
}

/// Mints a gateway JWT for `user_id`, valid `ttl_secs` from now, and prints
/// it: the bare token in text mode, `minted_json` under `--json`. The secret
/// is the gateway's `RSX_GW_JWT_SECRET`; an absent or empty one is `Misuse`.
pub fn mint(user_id: u32, ttl_secs: u64, secret: Option<&str>, is_json: bool) -> Outcome {
    let minted = mint_for(user_id, ttl_secs, secret, now_secs()?)?;
    if is_json {
        println!("{}", minted_json(&minted));
    } else {
        println!("{}", minted.token);
    }
    Ok(())
}

/// The JSON object `--json` prints: `token`, `user_id`, `exp` (Unix seconds).
pub fn minted_json(minted: &Minted) -> Value {
    json!({
        "token": minted.token,
        "user_id": minted.user_id,
        "exp": minted.exp,
    })
}

/// Mints at `now` (Unix seconds), so a caller can fix the clock.
pub fn mint_for(
    user_id: u32,
    ttl_secs: u64,
    secret: Option<&str>,
    now: u64,
) -> Result<Minted, Failure> {
    let secret = match secret {
        Some(secret) if !secret.is_empty() => secret,
        _ => {
            return Err(Failure::misuse(
                "no JWT secret: set RSX_GW_JWT_SECRET or pass --secret",
            ))
        }
    };
    let exp = now
        .checked_add(ttl_secs)
        .ok_or_else(|| Failure::misuse(format!("--ttl {ttl_secs} overflows the expiry")))?;
    let token = mint_jwt(&Claims::for_user(user_id, exp), secret).map_err(Failure::runtime)?;
    Ok(Minted {
        token,
        user_id,
        exp,
    })
}

fn now_secs() -> Result<u64, Failure> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|error| Failure::runtime(format!("the clock is before 1970: {error}")))
}
