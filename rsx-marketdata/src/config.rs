pub struct MarketDataConfig {
    pub listen_addr: String,
    pub max_symbols: usize,
    pub snapshot_depth: u32,
    pub spsc_ring_size: usize,
    pub book_capacity: u32,
    pub mid_price: i64,
    pub tick_size: i64,
    pub lot_size: i64,
    pub price_decimals: u8,
    pub qty_decimals: u8,
    pub max_outbound: usize,
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

fn env_i64(key: &str, default: i64) -> i64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u8(key: &str, default: u8) -> u8 {
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
        book_capacity: env_u32(
            "RSX_MD_BOOK_CAPACITY",
            1024,
        ),
        mid_price: env_i64(
            "RSX_MD_BOOK_MID_PRICE",
            50_000,
        ),
        tick_size: env_i64("RSX_MD_TICK_SIZE", 1),
        lot_size: env_i64("RSX_MD_LOT_SIZE", 1),
        price_decimals: env_u8(
            "RSX_MD_PRICE_DECIMALS",
            0,
        ),
        qty_decimals: env_u8("RSX_MD_QTY_DECIMALS", 0),
        max_outbound: env_usize(
            "RSX_MD_MAX_OUTBOUND",
            1024,
        ),
    }
}
