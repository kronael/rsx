use crate::metrics::MatchingMetrics;
use rsx_book::event::Event;
use rsx_book::event::OrderFailureReason;
use rsx_health::LoadGauges;
use std::sync::atomic::Ordering;

fn failed(reason: OrderFailureReason) -> Event {
    Event::OrderFailed {
        user_id: 1,
        reason,
        order_id_hi: 0,
        order_id_lo: 1,
    }
}

fn counter(metrics: &MatchingMetrics, name: &str) -> u64 {
    let load = LoadGauges::new();
    let snapshot = metrics.health_snapshot(&load);
    snapshot
        .counters
        .iter()
        .find(|counter| counter.name == name)
        .unwrap_or_else(|| panic!("no counter {name}"))
        .value
}

/// ORDER_FAILED events land in the counter of their own reason; other events
/// are ignored; the total is their sum.
#[test]
fn rejects_are_counted_by_reason() {
    let metrics = MatchingMetrics::default();
    metrics.count_rejects(&[
        failed(OrderFailureReason::Validation),
        Event::BBO {
            bid_px: rsx_types::Price(1),
            bid_qty: rsx_types::Qty(1),
            ask_px: rsx_types::Price(2),
            ask_qty: rsx_types::Qty(1),
        },
        failed(OrderFailureReason::FillOrKill),
        failed(OrderFailureReason::FillOrKill),
    ]);
    metrics.count_reject(OrderFailureReason::ReduceOnly);
    metrics.count_reject(OrderFailureReason::Duplicate);
    metrics.count_reject(OrderFailureReason::Duplicate);
    metrics.count_reject(OrderFailureReason::Duplicate);

    assert_eq!(counter(&metrics, "rejects_validation"), 1);
    assert_eq!(counter(&metrics, "rejects_reduce_only"), 1);
    assert_eq!(counter(&metrics, "rejects_fill_or_kill"), 2);
    assert_eq!(counter(&metrics, "rejects_duplicate"), 3);
    assert_eq!(counter(&metrics, "rejects_total"), 7);
}

/// The snapshot carries drops, the WAL and checkpoint positions and the
/// config version under their documented names, and no queue gauge: the
/// dedup set has no capacity to report.
#[test]
fn snapshot_exports_drops_wal_checkpoint_and_config_positions() {
    let metrics = MatchingMetrics::default();
    metrics.orders_processed.add(5);
    metrics.publishes.add(4);
    metrics.drops.add(3);
    metrics.dedup_entries.set(17);
    metrics.wal_last_seq.set(120);
    metrics.wal_flushed_seq.set(118);
    metrics.checkpoint_wal_seq.set(100);
    metrics.config_version.set(2);
    let load = LoadGauges::new();
    load.live.store(true, Ordering::Relaxed);
    load.ready.store(true, Ordering::Relaxed);
    load.state_idx.store(4, Ordering::Relaxed);

    let snapshot = metrics.health_snapshot(&load);

    assert!(snapshot.live && snapshot.ready);
    assert_eq!(snapshot.state, "running");
    assert!(snapshot.queues.is_empty());
    let counters: Vec<(&str, u64)> = snapshot
        .counters
        .iter()
        .map(|counter| (counter.name, counter.value))
        .collect();
    assert_eq!(
        counters,
        [
            ("orders_processed", 5),
            ("publishes", 4),
            ("drops", 3),
            ("dedup_entries", 17),
            ("rejects_validation", 0),
            ("rejects_reduce_only", 0),
            ("rejects_fill_or_kill", 0),
            ("rejects_duplicate", 0),
            ("rejects_total", 0),
            ("wal_last_seq", 120),
            ("wal_flushed_seq", 118),
            ("checkpoint_wal_seq", 100),
            ("config_version", 2),
        ]
    );
}
