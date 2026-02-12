use rsx_gateway::config::load_gateway_config;
use rsx_gateway::state::GatewayState;

#[test]
fn heartbeat_config_interval_5s() {
    let config = load_gateway_config();
    assert_eq!(config.heartbeat_interval_ms, 5_000);
}

#[test]
fn heartbeat_config_timeout_10s() {
    let config = load_gateway_config();
    assert_eq!(config.heartbeat_timeout_ms, 10_000);
}

#[test]
fn heartbeat_client_response_resets_timer() {
    let mut state =
        GatewayState::new(100, 10, 30_000, vec![]);
    let c = state.add_connection(1).unwrap();
    state.touch_connection(c, 1_000);

    // Before touch: stale
    let stale = state.stale_connections(5_000);
    assert_eq!(stale.len(), 1);

    // Simulate client heartbeat response resetting timer
    state.touch_connection(c, 50_000);
    let stale = state.stale_connections(5_000);
    assert!(stale.is_empty());
}

#[test]
fn connection_limit_rejects_sixth() {
    let mut state =
        GatewayState::new(100, 10, 30_000, vec![]);
    for _ in 0..5 {
        assert!(state.add_connection(1).is_ok());
    }
    let result = state.add_connection(1);
    assert_eq!(
        result,
        Err("max connections per user")
    );
}
