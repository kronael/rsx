use rsx_marketdata::config::load_marketdata_config;

#[test]
fn config_defaults() {
    let config = load_marketdata_config();
    assert_eq!(config.listen_addr, "0.0.0.0:8081");
    assert_eq!(config.max_symbols, 64);
    assert_eq!(config.snapshot_depth, 10);
    assert_eq!(config.spsc_ring_size, 8192);
}
