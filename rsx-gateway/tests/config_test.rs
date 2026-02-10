use rsx_gateway::config::load_gateway_config;

#[test]
fn config_defaults() {
    let config = load_gateway_config();
    assert_eq!(config.listen_addr, "0.0.0.0:8080");
    assert_eq!(config.max_pending, 10_000);
    assert_eq!(config.order_timeout_ms, 10_000);
    assert_eq!(config.heartbeat_interval_ms, 10_000);
    assert_eq!(config.heartbeat_timeout_ms, 30_000);
    assert_eq!(config.rate_limit_per_user, 10);
    assert_eq!(config.rate_limit_per_ip, 100);
    assert_eq!(config.rate_limit_per_instance, 1000);
    assert_eq!(config.circuit_threshold, 10);
    assert_eq!(config.circuit_cooldown_ms, 30_000);
}
