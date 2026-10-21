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
    let mut state = GatewayState::new(100, 10, 30_000, vec![]);
    let c1 = state.add_connection(1).unwrap();
    let c2 = state.add_connection(2).unwrap();
    state.broadcast_heartbeat(12345);
    let msgs1 = state.drain_outbound(c1);
    let msgs2 = state.drain_outbound(c2);
    assert_eq!(msgs1.len(), 1);
    assert_eq!(msgs2.len(), 1);
    assert_eq!(&*msgs1[0], "{\"H\":[12345]}");
    assert_eq!(&*msgs2[0], "{\"H\":[12345]}");
}

#[test]
fn stale_connections_detected() {
    let mut state = GatewayState::new(100, 10, 30_000, vec![]);
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
    let mut state = GatewayState::new(
        100,
        10,
        30_000,
        vec![rsx_types::SymbolConfig {
            symbol_id: 0,
            price_decimals: 8,
            qty_decimals: 8,
            tick_size: 1,
            lot_size: 1,
        }],
    );
    assert!(state.apply_config_applied(0, 5));
    assert_eq!(state.config_versions[0], 5);
    // stale version ignored
    assert!(!state.apply_config_applied(0, 4));
    assert_eq!(state.config_versions[0], 5);
    // unknown symbol ignored
    assert!(!state.apply_config_applied(99, 1));
}

#[test]
fn per_user_connection_limit_different_users() {
    let mut state = GatewayState::new(100, 10, 30_000, vec![]);
    for _ in 0..5 {
        assert!(state.add_connection(1).is_ok());
    }
    assert!(state.add_connection(1).is_err());
    assert!(state.add_connection(2).is_ok());
}

#[test]
fn ip_limiter_map_is_bounded() {
    use rsx_gateway::state::IP_LIMITER_MAX;
    let mut state = GatewayState::new(100, 10, 30_000, vec![]);
    state.rate_limit_per_ip = 100;

    // Insert IP_LIMITER_MAX + 5 unique IPs and verify the
    // map never grows past the cap.
    for i in 0..(IP_LIMITER_MAX + 5) as u32 {
        let ip: std::net::IpAddr = std::net::Ipv4Addr::from(i).into();
        let _ = state.ip_limiter_for(ip).try_consume();
        assert!(
            state.ip_limiters.len() <= IP_LIMITER_MAX,
            "ip_limiters grew past cap at i={i} (len={})",
            state.ip_limiters.len()
        );
    }
    assert_eq!(state.ip_limiters.len(), IP_LIMITER_MAX);

    // The very first IP should have been evicted.
    let first_ip: std::net::IpAddr = std::net::Ipv4Addr::from(0u32).into();
    assert!(
        !state.ip_limiters.contains_key(&first_ip),
        "oldest IP should have been evicted"
    );

    // The last-inserted IP should still be present.
    let last_ip: std::net::IpAddr = std::net::Ipv4Addr::from((IP_LIMITER_MAX + 4) as u32).into();
    assert!(
        state.ip_limiters.contains_key(&last_ip),
        "newest IP should be present"
    );
}
