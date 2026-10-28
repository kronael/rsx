use crate::promotion::classify_drain;
use crate::promotion::drain_until_caught_up;
use crate::promotion::Drain;
use crate::promotion::FINAL_DRAIN_ROUNDS;
use std::time::Duration;

#[test]
fn drain_returns_ok_once_the_tip_stops_advancing() {
    let rounds = [(Some(10), 5), (Some(12), 11), (Some(12), 12)];
    let mut next = 0;
    let result = drain_until_caught_up(Duration::ZERO, || {
        let (live_seq, applied_seq) = rounds[next];
        next += 1;
        Ok(classify_drain(live_seq, applied_seq))
    });
    assert!(result.is_ok());
    assert_eq!(next, 3);
}

#[test]
fn drain_errors_when_the_stream_ends_without_a_marker() {
    let mut calls = 0;
    let result = drain_until_caught_up(Duration::ZERO, || {
        calls += 1;
        Ok(classify_drain(None, 7))
    });
    assert!(result.is_err());
    assert_eq!(calls, 1);
}

#[test]
fn drain_gives_up_after_the_round_bound() {
    let mut calls = 0;
    let result = drain_until_caught_up(Duration::ZERO, || {
        calls += 1;
        Ok(Drain::Behind)
    });
    assert!(result.is_err());
    assert_eq!(calls, FINAL_DRAIN_ROUNDS);
}

#[test]
fn drain_propagates_a_stream_error() {
    let result = drain_until_caught_up(Duration::ZERO, || Err(std::io::Error::other("reset")));
    assert!(result.is_err());
}
