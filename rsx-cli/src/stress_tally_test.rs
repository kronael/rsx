use crate::stress_tally::Measure;
use crate::stress_tally::Samples;
use crate::stress_tally::Tracker;
use rsx_messages::WsFrame;
use std::time::Duration;
use tokio::time::Instant;

const OID: &str = "0190000000007000800000000000000a";
const OTHER: &str = "0190000000007000800000000000000b";

fn update(order_id: &str, status: u8, reason: u8, stamps: [u64; 5]) -> WsFrame {
    WsFrame::OrderUpdate {
        order_id: order_id.to_string(),
        status,
        filled_qty: 0,
        remaining_qty: 0,
        reason,
        gw_in_ns: stamps[0],
        risk_in_ns: stamps[1],
        me_in_ns: stamps[2],
        match_done_ns: stamps[3],
        gw_out_ns: stamps[4],
    }
}

fn fill(order_id: &str, stamps: [u64; 5]) -> WsFrame {
    WsFrame::Fill {
        taker_order_id: order_id.to_string(),
        maker_order_id: OTHER.to_string(),
        price: 100,
        qty: 1,
        timestamp_ns: 5000,
        fee: 0,
        gw_in_ns: stamps[0],
        risk_in_ns: stamps[1],
        me_in_ns: stamps[2],
        match_done_ns: stamps[3],
        gw_out_ns: stamps[4],
    }
}

fn error(code: u32) -> WsFrame {
    WsFrame::Error {
        code,
        message: "unknown symbol".to_string(),
        cid_order: None,
    }
}

fn millis(base: Instant, count: u64) -> Instant {
    base + Duration::from_millis(count)
}

const STAMPS: [u64; 5] = [1000, 2000, 4000, 7000, 11_000];

#[test]
fn summary_takes_nearest_rank_percentiles() {
    let mut samples = Samples::default();
    for value in (1..=100).rev() {
        samples.push(value);
    }
    let summary = samples.summary();
    assert_eq!(summary["n"], 100);
    assert_eq!(summary["min"], 1);
    assert_eq!(summary["p50"], 50);
    assert_eq!(summary["p95"], 95);
    assert_eq!(summary["p99"], 99);
    assert_eq!(summary["p99_9"], 100);
    assert_eq!(summary["max"], 100);
}

#[test]
fn summary_of_no_samples_is_null() {
    let summary = Samples::default().summary();
    assert_eq!(summary["n"], 0);
    assert!(summary["p50"].is_null());
    assert!(summary["max"].is_null());
}

#[test]
fn latency_runs_from_the_intended_send_time() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Accepted);
    tracker.record_sent(base, millis(base, 50));
    tracker.accept(&fill(OID, STAMPS), millis(base, 60));
    tracker.accept(&update(OID, 0, 0, STAMPS), millis(base, 80));
    assert_eq!(tracker.tally.send_lag_us.summary()["max"], 50_000);
    assert_eq!(tracker.tally.latency_us.summary()["max"], 80_000);
    assert_eq!(tracker.tally.accepted, 1);
    assert!(tracker.is_idle());
}

#[test]
fn replies_bind_to_orders_in_send_order() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Accepted);
    tracker.record_sent(millis(base, 0), millis(base, 0));
    tracker.record_sent(millis(base, 10), millis(base, 10));
    tracker.accept(&update(OID, 0, 0, STAMPS), millis(base, 20));
    tracker.accept(&update(OTHER, 0, 0, STAMPS), millis(base, 35));
    let summary = tracker.tally.latency_us.summary();
    assert_eq!(summary["min"], 20_000);
    assert_eq!(summary["max"], 25_000);
}

#[test]
fn server_legs_come_from_the_fill_and_the_update() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Accepted);
    tracker.record_sent(base, base);
    tracker.accept(&fill(OID, STAMPS), millis(base, 1));
    tracker.accept(&update(OID, 0, 0, [0; 5]), millis(base, 2));
    let tally = &mut tracker.tally;
    assert_eq!(tally.filled_orders, 1);
    assert_eq!(tally.fills, 1);
    assert!(tally.timestamp_errors.is_empty());
    let legs: Vec<u64> = tally
        .legs_ns
        .iter_mut()
        .map(|samples| samples.summary()["max"].as_u64().unwrap())
        .collect();
    assert_eq!(legs, [10_000, 1000, 2000, 3000, 4000]);
}

#[test]
fn an_update_without_timestamps_is_a_timestamp_error() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Accepted);
    tracker.record_sent(base, base);
    tracker.accept(&update(OID, 0, 0, [0; 5]), millis(base, 1));
    assert_eq!(tracker.tally.accepted, 1);
    assert_eq!(
        tracker.tally.timestamp_errors["missing_completion_timestamp"],
        1
    );
    assert_eq!(tracker.tally.legs_ns[0].len(), 0);
}

#[test]
fn backwards_clocks_are_a_timestamp_error() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Accepted);
    tracker.record_sent(base, base);
    tracker.accept(&update(OID, 0, 0, [9000, 2000, 4000, 7000, 11_000]), base);
    assert_eq!(
        tracker.tally.timestamp_errors["non_monotone_completion_timestamp"],
        1
    );
}

#[test]
fn a_failed_update_is_counted_under_its_reason_name() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Accepted);
    tracker.record_sent(base, base);
    tracker.record_sent(base, base);
    tracker.accept(&update(OID, 3, 14, [0; 5]), millis(base, 1));
    tracker.accept(&update(OTHER, 3, 14, [0; 5]), millis(base, 2));
    assert_eq!(tracker.tally.rejected, 2);
    assert_eq!(tracker.tally.rejected_by_reason["ORDER_TOO_SMALL"], 2);
    assert_eq!(tracker.tally.accepted, 0);
    assert_eq!(tracker.tally.latency_us.len(), 0);
}

#[test]
fn an_error_frame_rejects_the_oldest_order() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Rejected);
    tracker.record_sent(base, base);
    tracker.accept(&error(1007), millis(base, 3));
    assert_eq!(tracker.tally.rejected_by_reason["E1007"], 1);
    assert_eq!(tracker.tally.latency_us.summary()["max"], 3000);
}

#[test]
fn frames_with_no_order_waiting_are_not_ours() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Accepted);
    tracker.accept(&update(OID, 0, 0, STAMPS), base);
    tracker.accept(&fill(OID, STAMPS), base);
    assert_eq!(tracker.tally.accepted, 0);
    assert_eq!(tracker.tally.fills, 0);
    tracker.accept(&error(1007), base);
    assert_eq!(tracker.tally.errors_by_reason["unbound_error"], 1);
}

#[test]
fn unanswered_orders_time_out() {
    let base = Instant::now();
    let timeout = Duration::from_millis(100);
    let mut tracker = Tracker::new(Measure::Accepted);
    tracker.record_sent(base, base);
    tracker.record_sent(millis(base, 90), millis(base, 90));
    tracker.expire(millis(base, 150), timeout);
    assert_eq!(tracker.tally.timeouts, 1);
    assert!(!tracker.is_idle());
    tracker.finish();
    assert_eq!(tracker.tally.timeouts, 2);
    assert!(tracker.is_idle());
}

#[test]
fn a_fill_without_its_update_times_out_at_the_end() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Accepted);
    tracker.record_sent(base, base);
    tracker.accept(&fill(OID, STAMPS), millis(base, 1));
    tracker.finish();
    assert_eq!(tracker.tally.timeouts, 1);
    assert_eq!(tracker.tally.accepted, 0);
}
