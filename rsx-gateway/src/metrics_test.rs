use crate::circuit::State;
use crate::metrics::GatewayMetrics;
use rsx_health::LoadGauges;
use rsx_types::FailureReason;
use std::sync::atomic::Ordering;

fn counter(snapshot: &rsx_health::HealthSnapshot, name: &str) -> u64 {
    snapshot
        .counters
        .iter()
        .find(|counter| counter.name == name)
        .unwrap_or_else(|| panic!("no counter {name}"))
        .value
}

// rejects of two reasons and orders_processed -> each shows under its own name, rejects_total is the sum
#[test]
fn snapshot_reports_counters_by_name() {
    let metrics = GatewayMetrics::default();
    metrics.orders_processed.add(5);
    metrics.count_reject(FailureReason::RateLimit);
    metrics.count_reject(FailureReason::RateLimit);
    metrics.count_reject(FailureReason::Timeout);

    let snapshot = metrics.health_snapshot(&LoadGauges::new(), 100);

    assert_eq!(counter(&snapshot, "orders_processed"), 5);
    assert_eq!(counter(&snapshot, "rejects_rate_limit"), 2);
    assert_eq!(counter(&snapshot, "rejects_timeout"), 1);
    assert_eq!(counter(&snapshot, "rejects_overloaded"), 0);
    assert_eq!(counter(&snapshot, "rejects_total"), 3);
}

// every FailureReason code -> a counter named for it, so the table cannot drift from the enum
#[test]
fn every_failure_reason_has_its_own_counter() {
    let metrics = GatewayMetrics::default();
    for code in 0..=FailureReason::OrderTooSmall as u8 {
        metrics.count_reject_code(code);
    }
    let snapshot = metrics.health_snapshot(&LoadGauges::new(), 100);

    let per_reason: Vec<u64> = snapshot
        .counters
        .iter()
        .filter(|counter| counter.name.starts_with("rejects_") && counter.name != "rejects_total")
        .map(|counter| counter.value)
        .collect();
    assert_eq!(per_reason.len(), FailureReason::OrderTooSmall as usize + 2);
    assert_eq!(&per_reason[..15], &[1; 15]);
    assert_eq!(per_reason[15], 0, "rejects_other");
    assert_eq!(counter(&snapshot, "rejects_total"), 15);
    assert_eq!(counter(&snapshot, "rejects_order_too_small"), 1);
}

// a code beyond FailureReason -> counted under rejects_other
#[test]
fn unknown_failure_code_counts_as_other() {
    let metrics = GatewayMetrics::default();
    metrics.count_reject_code(200);

    let snapshot = metrics.health_snapshot(&LoadGauges::new(), 100);

    assert_eq!(counter(&snapshot, "rejects_other"), 1);
    assert_eq!(counter(&snapshot, "rejects_total"), 1);
}

// circuit states -> 0 closed, 1 open, 2 half-open
#[test]
fn circuit_state_is_exported_as_a_number() {
    let metrics = GatewayMetrics::default();
    let load = LoadGauges::new();
    for (state, code) in [(State::Closed, 0), (State::Open, 1), (State::HalfOpen, 2)] {
        metrics.set_circuit_state(state);
        let snapshot = metrics.health_snapshot(&load, 100);
        assert_eq!(counter(&snapshot, "circuit_state"), code);
    }
}

// pending 90 of 100 -> not ready; connections and pending come from the load gauges
#[test]
fn snapshot_reads_load_gauges_and_turns_unready_at_ninety_percent() {
    let metrics = GatewayMetrics::default();
    let load = LoadGauges::new();
    load.live.store(true, Ordering::Relaxed);
    load.connections.store(4, Ordering::Relaxed);
    load.pending_orders.store(89, Ordering::Relaxed);

    let snapshot = metrics.health_snapshot(&load, 100);
    assert!(snapshot.live && snapshot.ready);
    assert_eq!(counter(&snapshot, "connections"), 4);

    load.pending_orders.store(90, Ordering::Relaxed);
    let snapshot = metrics.health_snapshot(&load, 100);
    assert!(snapshot.live && !snapshot.ready);
    assert_eq!(snapshot.queues[0].used, 90);
    assert_eq!(snapshot.queues[0].cap, 100);
}
