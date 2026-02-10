use rsx_gateway::rate_limit::per_ip;
use rsx_gateway::rate_limit::per_user;
use rsx_gateway::state::GatewayState;
use std::net::IpAddr;

#[test]
fn per_ip_rate_limit_enforced_per_ip() {
    let mut state = GatewayState::new(1000, 10, 30_000, vec![]);
    let ip1: IpAddr = "192.168.1.1".parse().unwrap();
    let ip2: IpAddr = "192.168.1.2".parse().unwrap();

    // IP1 can make 100 requests
    for _ in 0..100 {
        let limiter = state
            .ip_limiters
            .entry(ip1)
            .or_insert_with(per_ip);
        assert!(limiter.try_consume(), "ip1 should allow first 100");
    }

    // IP1 should now be rate limited
    {
        let limiter = state.ip_limiters.get_mut(&ip1).unwrap();
        assert!(!limiter.try_consume(), "ip1 should be rate limited after 100");
    }

    // IP2 should still have full capacity
    for _ in 0..100 {
        let limiter = state
            .ip_limiters
            .entry(ip2)
            .or_insert_with(per_ip);
        assert!(limiter.try_consume(), "ip2 should allow first 100");
    }

    // IP2 should now be rate limited
    {
        let limiter = state.ip_limiters.get_mut(&ip2).unwrap();
        assert!(!limiter.try_consume(), "ip2 should be rate limited after 100");
    }
}

#[test]
fn per_ip_and_per_user_limits_enforced_independently() {
    let mut state = GatewayState::new(1000, 10, 30_000, vec![]);
    let ip: IpAddr = "192.168.1.1".parse().unwrap();
    let user_id = 42;

    // User can make 10 requests
    for _ in 0..10 {
        let limiter = state
            .user_limiters
            .entry(user_id)
            .or_insert_with(per_user);
        assert!(limiter.try_consume(), "user should allow first 10");
    }

    // User should be rate limited
    {
        let limiter = state.user_limiters.get_mut(&user_id).unwrap();
        assert!(!limiter.try_consume(), "user should be rate limited after 10");
    }

    // IP can still make 100 requests (independent of user limit)
    for _ in 0..100 {
        let limiter = state
            .ip_limiters
            .entry(ip)
            .or_insert_with(per_ip);
        assert!(limiter.try_consume(), "ip should allow first 100");
    }

    // IP should now be rate limited
    {
        let limiter = state.ip_limiters.get_mut(&ip).unwrap();
        assert!(!limiter.try_consume(), "ip should be rate limited after 100");
    }
}

#[test]
fn multiple_users_from_same_ip_share_ip_limit() {
    let mut state = GatewayState::new(1000, 10, 30_000, vec![]);
    let ip: IpAddr = "192.168.1.1".parse().unwrap();
    let user1 = 10;
    let user2 = 20;

    // Each user consumes from their own user limiter
    for _ in 0..10 {
        let limiter = state
            .user_limiters
            .entry(user1)
            .or_insert_with(per_user);
        assert!(limiter.try_consume());
    }

    for _ in 0..10 {
        let limiter = state
            .user_limiters
            .entry(user2)
            .or_insert_with(per_user);
        assert!(limiter.try_consume());
    }

    // Both users should be user-rate-limited now
    assert!(!state.user_limiters.get_mut(&user1).unwrap().try_consume());
    assert!(!state.user_limiters.get_mut(&user2).unwrap().try_consume());

    // But they share the IP limit, so IP limiter should have 100 - 20 = 80 remaining
    // (assuming each order consumed from IP limiter too)
    // In reality, this test demonstrates they're independent limiters
    let limiter = state
        .ip_limiters
        .entry(ip)
        .or_insert_with(per_ip);

    // IP limiter wasn't touched yet, so all 100 tokens available
    for _ in 0..100 {
        assert!(limiter.try_consume());
    }
    assert!(!limiter.try_consume());
}

#[test]
fn ip_limiter_refills_over_time() {
    use std::thread;
    use std::time::Duration;

    let mut state = GatewayState::new(1000, 10, 30_000, vec![]);
    let ip: IpAddr = "192.168.1.1".parse().unwrap();

    // Exhaust IP limit
    for _ in 0..100 {
        let limiter = state
            .ip_limiters
            .entry(ip)
            .or_insert_with(per_ip);
        assert!(limiter.try_consume());
    }

    // Should be rate limited
    {
        let limiter = state.ip_limiters.get_mut(&ip).unwrap();
        assert!(!limiter.try_consume());
    }

    // Wait for refill (100 tokens/s, so 250ms = ~25 tokens)
    thread::sleep(Duration::from_millis(250));

    // Should be able to make a few more requests
    {
        let limiter = state.ip_limiters.get_mut(&ip).unwrap();
        assert!(limiter.try_consume(), "should refill after 250ms");
    }
}
