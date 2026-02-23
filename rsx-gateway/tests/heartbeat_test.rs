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
fn heartbeat_sent_every_5s() {
    let mut state =
        GatewayState::new(100, 10, 30_000, vec![]);
    let c = state.add_connection(1).unwrap();

    let t0 = 1_000_000_000u64; // 1s in ns
    state.touch_connection(c, t0);

    // At t0, no heartbeat sent yet -> should send
    assert!(state.should_send_heartbeat(c, t0, 5_000));

    // Mark sent at t0
    state.mark_heartbeat_sent(c, t0);

    // 3s later: not yet
    let t3 = t0 + 3_000_000_000;
    assert!(
        !state.should_send_heartbeat(c, t3, 5_000)
    );

    // 5s later: should send again
    let t5 = t0 + 5_000_000_000;
    assert!(state.should_send_heartbeat(c, t5, 5_000));

    // Mark sent at t5
    state.mark_heartbeat_sent(c, t5);

    // 4.9s after t5: not yet
    let t9_9 = t5 + 4_900_000_000;
    assert!(
        !state.should_send_heartbeat(c, t9_9, 5_000)
    );

    // 5s after t5: yes
    let t10 = t5 + 5_000_000_000;
    assert!(
        state.should_send_heartbeat(c, t10, 5_000)
    );
}

#[test]
fn heartbeat_timeout_closes_at_10s() {
    let mut state =
        GatewayState::new(100, 10, 30_000, vec![]);
    let c = state.add_connection(1).unwrap();

    let t0 = 1_000_000_000u64;
    state.touch_connection(c, t0);

    // No heartbeat sent yet -> no timeout
    assert!(!state.is_heartbeat_timeout(c, t0, 10_000));

    // Send heartbeat at t0
    state.mark_heartbeat_sent(c, t0);

    // 5s later, no recv -> not timed out yet
    let t5 = t0 + 5_000_000_000;
    assert!(!state.is_heartbeat_timeout(c, t5, 10_000));

    // 9.9s later -> not timed out
    let t9_9 = t0 + 9_999_000_000;
    assert!(
        !state.is_heartbeat_timeout(c, t9_9, 10_000)
    );

    // 10s later -> timed out
    let t10 = t0 + 10_000_000_000;
    assert!(
        state.is_heartbeat_timeout(c, t10, 10_000)
    );

    // 15s later -> still timed out
    let t15 = t0 + 15_000_000_000;
    assert!(
        state.is_heartbeat_timeout(c, t15, 10_000)
    );
}

#[test]
fn heartbeat_client_response_resets_timer() {
    let mut state =
        GatewayState::new(100, 10, 30_000, vec![]);
    let c = state.add_connection(1).unwrap();

    let t0 = 1_000_000_000u64;
    state.touch_connection(c, t0);

    // Send heartbeat at t0
    state.mark_heartbeat_sent(c, t0);

    // 8s later, no response -> approaching timeout
    let t8 = t0 + 8_000_000_000;
    assert!(!state.is_heartbeat_timeout(c, t8, 10_000));

    // Client responds at t8
    state.heartbeat_recv(c, t8);

    // 10s after t0 -> no timeout because recv at t8
    let t10 = t0 + 10_000_000_000;
    assert!(
        !state.is_heartbeat_timeout(c, t10, 10_000)
    );

    // Send another heartbeat at t10
    state.mark_heartbeat_sent(c, t10);

    // 15s (5s after second send) -> no timeout yet
    let t15 = t10 + 5_000_000_000;
    assert!(
        !state.is_heartbeat_timeout(c, t15, 10_000)
    );

    // 20s (10s after second send, no recv) -> timeout
    let t20 = t10 + 10_000_000_000;
    assert!(
        state.is_heartbeat_timeout(c, t20, 10_000)
    );

    // Client responds at t20 -> resets
    state.heartbeat_recv(c, t20);
    assert!(
        !state.is_heartbeat_timeout(c, t20, 10_000)
    );
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
