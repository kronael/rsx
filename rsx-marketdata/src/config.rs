pub struct MarketDataConfig {
    pub listen_addr: String,
    pub max_symbols: usize,
    pub snapshot_depth: u32,
    pub spsc_ring_size: usize,
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

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub fn load_marketdata_config() -> MarketDataConfig {
    MarketDataConfig {
        listen_addr: env_str(
            "RSX_MD_LISTEN",
            "0.0.0.0:8081",
        ),
        max_symbols: env_usize(
            "RSX_MD_MAX_SYMBOLS",
            64,
        ),
        snapshot_depth: env_u32(
            "RSX_MD_SNAPSHOT_DEPTH",
            10,
        ),
        spsc_ring_size: env_usize(
            "RSX_MD_RING_SIZE",
            8192,
        ),
    }
}
