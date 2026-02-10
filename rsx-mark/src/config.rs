use std::io;

pub struct SourceConfig {
    pub name: String,
    pub ws_url: String,
    pub enabled: bool,
    pub reconnect_base_ms: u64,
    pub reconnect_max_ms: u64,
}

pub struct MarkConfig {
    pub listen_addr: String,
    pub wal_dir: String,
    pub stream_id: u32,
    pub staleness_ns: u64,
    pub price_scale: i64,
    pub symbol_map: crate::types::SymbolMap,
    pub sources: Vec<SourceConfig>,
}

fn env_str(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
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

fn env_i64(key: &str, default: i64) -> i64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| v == "1" || v == "true")
        .unwrap_or(default)
}

fn load_source(name: &str) -> SourceConfig {
    let prefix =
        format!("RSX_MARK_SOURCE_{}", name.to_uppercase());
    SourceConfig {
        name: name.to_lowercase(),
        ws_url: env_str(
            &format!("{}_WS_URL", prefix),
            "",
        ),
        enabled: env_bool(
            &format!("{}_ENABLED", prefix),
            false,
        ),
        reconnect_base_ms: env_u64(
            &format!("{}_RECONNECT_BASE_MS", prefix),
            1000,
        ),
        reconnect_max_ms: env_u64(
            &format!("{}_RECONNECT_MAX_MS", prefix),
            30000,
        ),
    }
}

fn parse_symbol_map(raw: &str) -> crate::types::SymbolMap {
    let mut map = crate::types::SymbolMap::new();
    for pair in raw.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let mut it = pair.split('=');
        let sym = it.next().unwrap_or("").trim();
        let id = it.next().unwrap_or("").trim();
        if sym.is_empty() || id.is_empty() {
            continue;
        }
        if let Ok(parsed) = id.parse::<u32>() {
            map.insert(sym.to_string(), parsed);
        }
    }
    map
}

pub fn load_mark_config() -> io::Result<MarkConfig> {
    let listen_addr = env_str(
        "RSX_MARK_LISTEN_ADDR",
        "0.0.0.0:9200",
    );
    let wal_dir = env_str(
        "RSX_MARK_WAL_DIR",
        "./wal/mark",
    );
    let stream_id = env_u32("RSX_MARK_STREAM_ID", 100);
    let staleness_ns = env_u64(
        "RSX_MARK_STALENESS_NS",
        10_000_000_000,
    );
    let price_scale = env_i64("RSX_MARK_PRICE_SCALE", 1_000_000);
    let symbol_map = parse_symbol_map(
        &env_str("RSX_MARK_SYMBOL_MAP", ""),
    );

    let source_names = ["BINANCE", "COINBASE"];
    let sources: Vec<SourceConfig> = source_names
        .iter()
        .map(|n| load_source(n))
        .filter(|s| s.enabled)
        .collect();

    Ok(MarkConfig {
        listen_addr,
        wal_dir,
        stream_id,
        staleness_ns,
        price_scale,
        symbol_map,
        sources,
    })
}
