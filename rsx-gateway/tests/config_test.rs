use rsx_gateway::config::load_gateway_config;

#[test]
fn config_defaults() {
    unsafe {
        std::env::set_var("RSX_GW_JWT_SECRET", "test-secret-padded-to-32-bytes-min!");
    }
    let config = load_gateway_config();
    assert_eq!(config.listen_addr, "0.0.0.0:8080");
    assert_eq!(config.max_pending, 10_000);
    assert_eq!(config.order_timeout_ms, 10_000);
    assert_eq!(config.heartbeat_interval_ms, 5_000);
    assert_eq!(config.heartbeat_timeout_ms, 10_000);
    assert_eq!(config.rate_limit_per_user, 10);
    assert_eq!(config.rate_limit_per_ip, 100);
    assert_eq!(config.circuit_threshold, 10);
    assert_eq!(config.circuit_cooldown_ms, 30_000);
    assert!(!config.jwt_secret.is_empty());
}
