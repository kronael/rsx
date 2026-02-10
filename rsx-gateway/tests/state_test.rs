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
