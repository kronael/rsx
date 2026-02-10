use rsx_gateway::rate_limit::per_instance;
use rsx_gateway::rate_limit::per_ip;
use rsx_gateway::rate_limit::per_user;
use rsx_gateway::rate_limit::RateLimiter;
use std::thread;
use std::time::Duration;

#[test]
fn rate_limit_allows_under_threshold() {
    let mut rl = RateLimiter::new(5, 5);
    for _ in 0..3 {
        assert!(rl.try_consume());
    }
}

#[test]
fn rate_limit_rejects_at_threshold() {
    let mut rl = RateLimiter::new(5, 5);
    for _ in 0..5 {
        assert!(rl.try_consume());
    }
    assert!(!rl.try_consume());
}

#[test]
fn rate_limit_refills_over_time() {
    let mut rl = RateLimiter::new(5, 5);
    for _ in 0..5 {
        assert!(rl.try_consume());
    }
    assert!(!rl.try_consume());
    thread::sleep(Duration::from_millis(250));
    assert!(rl.try_consume());
}

#[test]
fn rate_limit_per_user_independent() {
    let mut a = per_user();
    let mut b = per_user();
    for _ in 0..10 {
        assert!(a.try_consume());
    }
    assert!(!a.try_consume());
    assert!(b.try_consume());
}

#[test]
fn rate_limit_per_ip_independent() {
    let mut a = per_ip();
    let mut b = per_ip();
    for _ in 0..100 {
        assert!(a.try_consume());
    }
    assert!(!a.try_consume());
    assert!(b.try_consume());
}

#[test]
fn rate_limit_10_per_sec_per_user() {
    let mut rl = per_user();
    for _ in 0..10 {
        assert!(rl.try_consume());
    }
    assert!(!rl.try_consume());
}

#[test]
fn rate_limit_100_per_sec_per_ip() {
    let mut rl = per_ip();
    for _ in 0..100 {
        assert!(rl.try_consume());
    }
    assert!(!rl.try_consume());
}

#[test]
fn rate_limit_1000_per_sec_per_instance() {
    let mut rl = per_instance();
    for _ in 0..1000 {
        assert!(rl.try_consume());
    }
    assert!(!rl.try_consume());
}
