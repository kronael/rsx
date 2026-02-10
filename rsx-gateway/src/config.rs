pub struct GatewayConfig {
    pub listen_addr: String,
    pub risk_addr: String,
    pub max_pending: usize,
    pub order_timeout_ms: u64,
    pub heartbeat_interval_ms: u64,
    pub heartbeat_timeout_ms: u64,
    pub rate_limit_per_user: u32,
    pub rate_limit_per_ip: u32,
    pub rate_limit_per_instance: u32,
    pub circuit_threshold: u32,
    pub circuit_cooldown_ms: u64,
}

fn env_str(key: &str, default: &str) -> String {
    std::env::var(key)
        .unwrap_or_else(|_| default.to_string())
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

pub fn load_gateway_config() -> GatewayConfig {
    GatewayConfig {
        listen_addr: env_str(
            "RSX_GW_LISTEN",
            "0.0.0.0:8080",
        ),
        risk_addr: env_str(
            "RSX_GW_RISK_ADDR",
            "127.0.0.1:9090",
        ),
        max_pending: env_usize(
            "RSX_GW_MAX_PENDING",
            10_000,
        ),
        order_timeout_ms: env_u64(
            "RSX_GW_ORDER_TIMEOUT_MS",
            10_000,
        ),
        heartbeat_interval_ms: env_u64(
            "RSX_GW_HB_INTERVAL_MS",
            5_000,
        ),
        heartbeat_timeout_ms: env_u64(
            "RSX_GW_HB_TIMEOUT_MS",
            10_000,
        ),
        rate_limit_per_user: env_u32(
            "RSX_GW_RL_USER",
            10,
        ),
        rate_limit_per_ip: env_u32(
            "RSX_GW_RL_IP",
            100,
        ),
        rate_limit_per_instance: env_u32(
            "RSX_GW_RL_INSTANCE",
            1000,
        ),
        circuit_threshold: env_u32(
            "RSX_GW_CB_THRESHOLD",
            10,
        ),
        circuit_cooldown_ms: env_u64(
            "RSX_GW_CB_COOLDOWN_MS",
            30_000,
        ),
    }
}
