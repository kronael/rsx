use rsx_gateway::state::GatewayState;

#[test]
fn rate_limiter_created_on_demand() {
    let mut state = GatewayState::new(100, 10, 30_000);
    assert!(state.user_limiters.is_empty());
    state.add_connection(42);
    assert!(state.user_limiters.is_empty());
}

#[test]
fn circuit_breaker_default_closed() {
    let state = GatewayState::new(100, 10, 30_000);
    assert!(std::mem::size_of_val(&state.circuit) > 0);
}

#[test]
fn broadcast_heartbeat_adds_to_outbound() {
    let mut state =
        GatewayState::new(100, 10, 30_000);
    let c1 = state.add_connection(1);
    let c2 = state.add_connection(2);
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
        GatewayState::new(100, 10, 30_000);
    let c1 = state.add_connection(1);
    let c2 = state.add_connection(2);
    // c1 active long ago, c2 recent
    state.touch_connection(c1, 1_000);
    state.touch_connection(c2, 50_000);
    let stale = state.stale_connections(10_000);
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0], c1);
}
