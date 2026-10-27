use crate::state::DEFAULT_MAX_OUTBOUND;
use crate::state::DEFAULT_RATE_LIMIT_PER_INSTANCE;
use std::str::FromStr;
use std::time::Duration;
use tracing::warn;

pub struct GatewayConfig {
    pub listen_addr: String,
    pub max_pending: usize,
    pub max_outbound: usize,
    pub order_timeout_ms: u64,
    pub heartbeat_interval_ms: u64,
    pub heartbeat_timeout_ms: u64,
    pub rate_limit_per_user: u32,
    pub rate_limit_per_ip: u32,
    pub rate_limit_per_instance: u32,
    pub circuit_threshold: u32,
    pub circuit_cooldown_ms: u64,
    pub jwt_secret: String,
    pub database_url: String,
    pub symbol_poll_interval: Duration,
}

/// The env var `key` parsed as `T`; `None` when it is unset or does not
/// parse.
pub fn env_parse<T: FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok()?.parse().ok()
}

fn env_or<T: FromStr>(key: &str, default: T) -> T {
    env_parse(key).unwrap_or(default)
}

/// Core index named by the env var `key`; `None` when it is unset or not a
/// number (the latter is warned about).
pub fn core_id_from_env(key: &str) -> Option<usize> {
    let raw = std::env::var(key).ok()?;
    match raw.parse() {
        Ok(core_id) => Some(core_id),
        Err(_) => {
            warn!("{key}: invalid core id '{raw}', not pinning");
            None
        }
    }
}

/// HS256 with a short secret is brute-forceable from one observed token, so
/// the gateway refuses to start with less.
const JWT_SECRET_MIN_LEN: usize = 32;

pub fn load_gateway_config() -> GatewayConfig {
    let jwt_secret: String = env_or("RSX_GW_JWT_SECRET", String::new());
    if jwt_secret.is_empty() {
        eprintln!("rsx-gateway: RSX_GW_JWT_SECRET must be set");
        std::process::exit(2);
    }
    if jwt_secret.len() < JWT_SECRET_MIN_LEN {
        eprintln!(
            "rsx-gateway: RSX_GW_JWT_SECRET too short \
             ({} bytes; minimum {} for HS256)",
            jwt_secret.len(),
            JWT_SECRET_MIN_LEN,
        );
        std::process::exit(2);
    }

    let database_url: String = env_or("RSX_GW_DATABASE_URL", String::new());
    if database_url.is_empty() {
        eprintln!(
            "rsx-gateway: RSX_GW_DATABASE_URL must be set (symbol config is read from postgres)"
        );
        std::process::exit(2);
    }

    GatewayConfig {
        listen_addr: env_or("RSX_GW_LISTEN", "0.0.0.0:8080".to_string()),
        max_pending: env_or("RSX_GW_MAX_PENDING", 10_000),
        max_outbound: env_or("RSX_GW_MAX_OUTBOUND", DEFAULT_MAX_OUTBOUND),
        order_timeout_ms: env_or("RSX_GW_ORDER_TIMEOUT_MS", 10_000),
        heartbeat_interval_ms: env_or("RSX_GW_HEARTBEAT_INTERVAL_S", 5) * 1000,
        heartbeat_timeout_ms: env_or("RSX_GW_IDLE_TIMEOUT_S", 10) * 1000,
        rate_limit_per_user: env_or("RSX_GW_RL_USER", 10),
        rate_limit_per_ip: env_or("RSX_GW_RL_IP", 100),
        rate_limit_per_instance: env_or("RSX_GW_RL_INSTANCE", DEFAULT_RATE_LIMIT_PER_INSTANCE),
        circuit_threshold: env_or("RSX_GW_CB_THRESHOLD", 10),
        circuit_cooldown_ms: env_or("RSX_GW_CB_COOLDOWN_MS", 30_000),
        jwt_secret,
        database_url,
        symbol_poll_interval: Duration::from_secs(env_or("RSX_GW_SYMBOL_POLL_S", 10)),
    }
}
