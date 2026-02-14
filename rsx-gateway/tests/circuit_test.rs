use rsx_gateway::circuit::CircuitBreaker;
use rsx_gateway::circuit::State;
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn circuit_closed_allows_orders() {
    let mut cb = CircuitBreaker::new(10, Duration::from_secs(30));
    assert!(cb.allow());
    assert_eq!(cb.state(), State::Closed);
}

#[test]
fn circuit_open_after_10_failures() {
    let mut cb = CircuitBreaker::new(10, Duration::from_secs(30));
    for _ in 0..9 {
        cb.record_failure();
        assert_eq!(cb.state(), State::Closed);
    }
    cb.record_failure();
    assert_eq!(cb.state(), State::Open);
}

#[test]
fn circuit_open_rejects_immediately() {
    let mut cb = CircuitBreaker::new(3, Duration::from_secs(30));
    for _ in 0..3 {
        cb.record_failure();
    }
    assert!(!cb.allow());
}

#[test]
fn circuit_half_open_after_cooldown() {
    let mut cb = CircuitBreaker::new(3, Duration::from_millis(10));
    for _ in 0..3 {
        cb.record_failure();
    }
    assert_eq!(cb.state(), State::Open);
    assert!(!cb.allow());

    let start = Instant::now();
    loop {
        if cb.allow() {
            break;
        }
        if start.elapsed() > Duration::from_millis(100) {
            panic!("timeout waiting for circuit to become half-open");
        }
        thread::sleep(Duration::from_micros(100));
    }
    assert_eq!(cb.state(), State::HalfOpen);
}

#[test]
fn circuit_half_open_success_closes() {
    let mut cb = CircuitBreaker::new(3, Duration::from_millis(10));
    for _ in 0..3 {
        cb.record_failure();
    }

    let start = Instant::now();
    loop {
        if cb.allow() {
            break;
        }
        if start.elapsed() > Duration::from_millis(100) {
            panic!("timeout waiting for circuit to become half-open");
        }
        thread::sleep(Duration::from_micros(100));
    }
    cb.record_success();
    assert_eq!(cb.state(), State::Closed);
}

#[test]
fn circuit_half_open_failure_reopens() {
    let mut cb = CircuitBreaker::new(3, Duration::from_millis(10));
    for _ in 0..3 {
        cb.record_failure();
    }

    let start = Instant::now();
    loop {
        if cb.allow() {
            break;
        }
        if start.elapsed() > Duration::from_millis(100) {
            panic!("timeout waiting for circuit to become half-open");
        }
        thread::sleep(Duration::from_micros(100));
    }
    assert_eq!(cb.state(), State::HalfOpen);
    cb.record_failure();
    assert_eq!(cb.state(), State::Open);
}
