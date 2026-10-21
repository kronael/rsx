use rsx_gateway::state::GatewayState;

#[test]
fn rate_limiter_created_on_demand() {
    let mut state = GatewayState::new(100, 10, 30_000, vec![]);
    assert!(state.user_limiters.is_empty());
    state.add_connection(42).unwrap();
    assert!(state.user_limiters.is_empty());
}

// fresh state -> the instance limiter holds 1000 orders, the spec's per-instance rate
#[test]
fn instance_rate_limit_defaults_to_1000_orders_per_second() {
    let state = GatewayState::new(100, 10, 30_000, vec![]);
    assert_eq!(state.instance_limiter.tokens_remaining(), 1000);
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
fn reap_idle_connections_removes_only_the_stale() {
    let mut state = GatewayState::new(100, 10, 30_000, vec![]);
    let c1 = state.add_connection(1).unwrap();
    let c2 = state.add_connection(2).unwrap();
    state.touch_connection(c1, 1_000);
    state.touch_connection(c2, 50_000);
    assert_eq!(state.reap_idle_connections(10_000), 1);
    assert!(!state.connections.contains_key(&c1));
    assert!(state.connections.contains_key(&c2));
}

#[test]
fn push_to_user_removes_a_connection_whose_outbound_is_full() {
    let mut state = GatewayState::new(100, 10, 30_000, vec![]);
    state.max_outbound = 2;
    let full = state.add_connection(1).unwrap();
    let drained = state.add_connection(1).unwrap();
    let other_user = state.add_connection(2).unwrap();
    state.push_to_user(1, "a".into());
    state.push_to_user(1, "b".into());
    state.drain_outbound(drained);

    state.push_to_user(1, "c".into());

    assert!(!state.connections.contains_key(&full));
    assert_eq!(state.drain_outbound(drained).len(), 1);
    assert!(state.connections.contains_key(&other_user));
}

#[test]
fn broadcast_heartbeat_removes_a_connection_whose_outbound_is_full() {
    let mut state = GatewayState::new(100, 10, 30_000, vec![]);
    state.max_outbound = 1;
    let full = state.add_connection(1).unwrap();
    let empty = state.add_connection(2).unwrap();
    state.push_to_user(1, "a".into());

    state.broadcast_heartbeat(5);

    assert!(!state.connections.contains_key(&full));
    assert_eq!(state.drain_outbound(empty).len(), 1);
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

#[test]
fn connection_limit_rejects_sixth() {
    let mut state = GatewayState::new(100, 10, 30_000, vec![]);
    for _ in 0..5 {
        assert!(state.add_connection(1).is_ok());
    }
    assert_eq!(state.add_connection(1), Err("max connections per user"));
}
