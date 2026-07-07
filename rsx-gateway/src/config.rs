use rsx_types::SymbolConfig;

pub struct GatewayConfig {
    pub listen_addr: String,
    pub risk_addr: String,
    pub max_pending: usize,
    pub order_timeout_ms: u64,
    pub heartbeat_interval_ms: u64,
    pub heartbeat_timeout_ms: u64,
    pub rate_limit_per_user: u32,
    pub rate_limit_per_ip: u32,
    pub circuit_threshold: u32,
    pub circuit_cooldown_ms: u64,
    pub jwt_secret: String,
    pub symbol_configs: Vec<SymbolConfig>,
}

fn env_str(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn load_symbol_configs() -> Vec<SymbolConfig> {
    let max = env_usize("RSX_MAX_SYMBOLS", 16);
    let tick = env_u64("RSX_DEFAULT_TICK_SIZE", 1) as i64;
    let lot = env_u64("RSX_DEFAULT_LOT_SIZE", 1) as i64;
    (0..max)
        .map(|i| {
            let t = std::env::var(format!("RSX_SYMBOL_{i}_TICK_SIZE"))
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(tick);
            let l = std::env::var(format!("RSX_SYMBOL_{i}_LOT_SIZE"))
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(lot);
            SymbolConfig {
                symbol_id: i as u32,
                price_decimals: 8,
                qty_decimals: 8,
                tick_size: t,
                lot_size: l,
            }
        })
        .collect()
}

/// Minimum JWT secret length. HS256 with a short secret is
/// brute-forceable from a single observed token, so refuse
/// to start with anything weaker. 32 bytes is the floor; 64
/// is recommended for HMAC-SHA256.
pub const JWT_SECRET_MIN_LEN: usize = 32;

pub fn load_gateway_config() -> GatewayConfig {
    let jwt_secret = env_str("RSX_GW_JWT_SECRET", "");
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

    GatewayConfig {
        listen_addr: env_str("RSX_GW_LISTEN", "0.0.0.0:8080"),
        risk_addr: env_str("RSX_GW_RISK_ADDR", "127.0.0.1:9090"),
        max_pending: env_usize("RSX_GW_MAX_PENDING", 10_000),
        order_timeout_ms: env_u64("RSX_GW_ORDER_TIMEOUT_MS", 10_000),
        heartbeat_interval_ms: env_u64("RSX_GW_HEARTBEAT_INTERVAL_S", 5) * 1000,
        heartbeat_timeout_ms: env_u64("RSX_GW_IDLE_TIMEOUT_S", 10) * 1000,
        rate_limit_per_user: env_u32("RSX_GW_RL_USER", 10),
        rate_limit_per_ip: env_u32("RSX_GW_RL_IP", 100),
        circuit_threshold: env_u32("RSX_GW_CB_THRESHOLD", 10),
        circuit_cooldown_ms: env_u64("RSX_GW_CB_COOLDOWN_MS", 30_000),
        jwt_secret,
        symbol_configs: load_symbol_configs(),
    }
}
