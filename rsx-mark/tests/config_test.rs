use rsx_mark::config::load_mark_config;
use std::env;

fn clear_env() {
    for key in &[
        "RSX_MARK_LISTEN_ADDR",
        "RSX_MARK_WAL_DIR",
        "RSX_MARK_STREAM_ID",
        "RSX_MARK_STALENESS_NS",
        "RSX_MARK_SOURCE_BINANCE_WS_URL",
        "RSX_MARK_SOURCE_BINANCE_ENABLED",
        "RSX_MARK_SOURCE_BINANCE_RECONNECT_BASE_MS",
        "RSX_MARK_SOURCE_BINANCE_RECONNECT_MAX_MS",
        "RSX_MARK_SOURCE_COINBASE_WS_URL",
        "RSX_MARK_SOURCE_COINBASE_ENABLED",
        "RSX_MARK_SOURCE_COINBASE_RECONNECT_BASE_MS",
        "RSX_MARK_SOURCE_COINBASE_RECONNECT_MAX_MS",
    ] {
        env::remove_var(key);
    }
}

#[test]
fn config_parse_valid_env() {
    clear_env();
    env::set_var(
        "RSX_MARK_SOURCE_BINANCE_ENABLED",
        "1",
    );
    env::set_var(
        "RSX_MARK_SOURCE_BINANCE_WS_URL",
        "wss://example.com/ws",
    );
    let cfg = load_mark_config().unwrap();
    assert_eq!(cfg.listen_addr, "0.0.0.0:9200");
    assert_eq!(cfg.wal_dir, "./wal/mark");
    assert_eq!(cfg.stream_id, 100);
    assert_eq!(cfg.staleness_ns, 10_000_000_000);
    assert_eq!(cfg.sources.len(), 1);
    assert_eq!(cfg.sources[0].name, "binance");
    clear_env();
}

#[test]
fn config_staleness_ns_overrides_default() {
    clear_env();
    env::set_var("RSX_MARK_STALENESS_NS", "5000000000");
    let cfg = load_mark_config().unwrap();
    assert_eq!(cfg.staleness_ns, 5_000_000_000);
    clear_env();
}

#[test]
fn config_source_enabled_false_skipped() {
    clear_env();
    env::set_var(
        "RSX_MARK_SOURCE_BINANCE_ENABLED",
        "0",
    );
    env::set_var(
        "RSX_MARK_SOURCE_COINBASE_ENABLED",
        "0",
    );
    let cfg = load_mark_config().unwrap();
    assert_eq!(cfg.sources.len(), 0);
    clear_env();
}

#[test]
fn config_listen_addr_and_wal_dir() {
    clear_env();
    env::set_var(
        "RSX_MARK_LISTEN_ADDR",
        "127.0.0.1:9300",
    );
    env::set_var("RSX_MARK_WAL_DIR", "/data/wal/mark");
    let cfg = load_mark_config().unwrap();
    assert_eq!(cfg.listen_addr, "127.0.0.1:9300");
    assert_eq!(cfg.wal_dir, "/data/wal/mark");
    clear_env();
}

#[test]
fn config_stream_id_set() {
    clear_env();
    env::set_var("RSX_MARK_STREAM_ID", "42");
    let cfg = load_mark_config().unwrap();
    assert_eq!(cfg.stream_id, 42);
    clear_env();
}

#[test]
fn config_reconnect_base_and_max_ms() {
    clear_env();
    env::set_var(
        "RSX_MARK_SOURCE_BINANCE_ENABLED",
        "1",
    );
    env::set_var(
        "RSX_MARK_SOURCE_BINANCE_RECONNECT_BASE_MS",
        "2000",
    );
    env::set_var(
        "RSX_MARK_SOURCE_BINANCE_RECONNECT_MAX_MS",
        "60000",
    );
    let cfg = load_mark_config().unwrap();
    assert_eq!(cfg.sources[0].reconnect_base_ms, 2000);
    assert_eq!(cfg.sources[0].reconnect_max_ms, 60000);
    clear_env();
}
