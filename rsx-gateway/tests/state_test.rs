use rsx_gateway::state::GatewayState;

#[test]
fn rate_limiter_created_on_demand() {
    let mut state = GatewayState::new(100, 10, 30_000, vec![]);
    assert!(state.user_limiters.is_empty());
    state.add_connection(42).unwrap();
    assert!(state.user_limiters.is_empty());
}

#[test]
fn circuit_breaker_default_closed() {
    let state = GatewayState::new(100, 10, 30_000, vec![]);
    assert!(std::mem::size_of_val(&state.circuit) > 0);
}

#[test]
fn broadcast_heartbeat_adds_to_outbound() {
    let mut state =
        GatewayState::new(100, 10, 30_000, vec![]);
    let c1 = state.add_connection(1).unwrap();
    let c2 = state.add_connection(2).unwrap();
    state.broadcast_heartbeat(12345);
    let msgs1 = state.drain_outbound(c1);
    let msgs2 = state.drain_outbound(c2);
    assert_eq!(msgs1.len(), 1);
    assert_eq!(msgs2.len(), 1);
    assert_eq!(msgs1[0], "{\"H\":[12345]}");
    assert_eq!(msgs2[0], "{\"H\":[12345]}");
}

#[test]
fn stale_connections_detected() {
    let mut state =
        GatewayState::new(100, 10, 30_000, vec![]);
    let c1 = state.add_connection(1).unwrap();
    let c2 = state.add_connection(2).unwrap();
    // c1 active long ago, c2 recent
    state.touch_connection(c1, 1_000);
    state.touch_connection(c2, 50_000);
    let stale = state.stale_connections(10_000);
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0], c1);
}

#[test]
fn config_applied_tracks_monotonic_version() {
    let mut state = GatewayState::new(100, 10, 30_000, vec![
        rsx_types::SymbolConfig {
            symbol_id: 0,
            price_decimals: 8,
            qty_decimals: 8,
            tick_size: 1,
            lot_size: 1,
        },
    ]);
    assert!(state.apply_config_applied(0, 5));
    assert_eq!(state.config_versions[0], 5);
    // stale version ignored
    assert!(!state.apply_config_applied(0, 4));
    assert_eq!(state.config_versions[0], 5);
}

#[test]
fn per_user_connection_limit_different_users() {
    let mut state =
        GatewayState::new(100, 10, 30_000, vec![]);
    for _ in 0..5 {
        assert!(state.add_connection(1).is_ok());
    }
    assert!(state.add_connection(1).is_err());
    assert!(state.add_connection(2).is_ok());
}

#[test]
fn config_applied_reloads_symbol_overrides() {
    std::env::set_var("RSX_SYMBOL_0_TICK_SIZE", "5");
    std::env::set_var("RSX_SYMBOL_0_LOT_SIZE", "7");
    let mut state = GatewayState::new(100, 10, 30_000, vec![
        rsx_types::SymbolConfig {
            symbol_id: 0,
            price_decimals: 8,
            qty_decimals: 8,
            tick_size: 1,
            lot_size: 1,
        },
    ]);
    assert!(state.apply_config_applied(0, 1));
    assert_eq!(state.symbol_configs[0].tick_size, 5);
    assert_eq!(state.symbol_configs[0].lot_size, 7);
    std::env::remove_var("RSX_SYMBOL_0_TICK_SIZE");
    std::env::remove_var("RSX_SYMBOL_0_LOT_SIZE");
}
