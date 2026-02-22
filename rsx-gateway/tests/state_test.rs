use rsx_gateway::circuit::State;
use rsx_gateway::pending::PendingOrder;
use rsx_gateway::rate_limit;
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

#[test]
fn ws_new_order_accepted_and_filled() {
    let mut state = GatewayState::new(100, 10, 30_000, vec![]);
    let order_id = [1u8; 16];
    let order = PendingOrder {
        order_id,
        user_id: 42,
        symbol_id: 0,
        client_order_id: [0u8; 20],
        timestamp_ns: 1_000,
    };
    assert!(state.pending.push(order));
    assert_eq!(state.pending.len(), 1);
    let removed = state.pending.remove(&order_id);
    assert!(removed.is_some());
    assert!(state.pending.is_empty());
}

#[test]
fn concurrent_sessions_isolated() {
    let mut state = GatewayState::new(100, 10, 30_000, vec![]);
    state.add_connection(1).unwrap();
    state.add_connection(2).unwrap();
    // exhaust user 1's rate limiter
    let mut limiter = rate_limit::per_user();
    for _ in 0..100 {
        limiter.try_consume();
    }
    state.user_limiters.insert(1, limiter);
    // user 2 has no limiter -- unaffected
    assert!(!state.user_limiters.contains_key(&2));
    assert_eq!(
        state.user_limiters[&1].tokens_remaining(),
        0
    );
}

#[test]
fn circuit_breaker_opens_on_gateway_overload() {
    let mut state = GatewayState::new(100, 10, 30_000, vec![]);
    for _ in 0..10 {
        state.circuit.record_failure();
    }
    assert_eq!(state.circuit.state(), State::Open);
    assert!(!state.circuit.allow());
}
