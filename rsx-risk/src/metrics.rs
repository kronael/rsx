use rsx_health::CounterGauge;
use rsx_health::HealthSnapshot;
use rsx_health::LoadGauges;
use rsx_health::QueueGauge;
use std::sync::atomic::Ordering;

/// Build a `HealthSnapshot` from the shard's load gauges. Passed
/// (wrapped in a named closure) to `rsx_health::spawn_health_server`
/// so the health thread re-reads live atomics on each request.
pub fn health_snapshot(g: &LoadGauges) -> HealthSnapshot {
    let resp_used = g.resp_ring_used.load(Ordering::Relaxed);
    let resp_cap = g.resp_ring_cap.load(Ordering::Relaxed);
    let acc_used = g.accept_ring_used.load(Ordering::Relaxed);
    let acc_cap = g.accept_ring_cap.load(Ordering::Relaxed);
    let persist_used = g.persist_ring_used.load(Ordering::Relaxed);
    let persist_cap = g.persist_ring_cap.load(Ordering::Relaxed);
    let saturation = [
        (resp_used, resp_cap),
        (acc_used, acc_cap),
        (persist_used, persist_cap),
    ]
    .into_iter()
    .filter(|&(_, cap)| cap > 0)
    .map(|(used, cap)| (used as f64) / (cap as f64))
    .fold(0.0f64, f64::max);
    HealthSnapshot {
        live: g.live.load(Ordering::Relaxed),
        ready: g.ready.load(Ordering::Relaxed),
        saturation,
        queues: vec![
            QueueGauge {
                name: "resp_ring",
                used: resp_used,
                cap: resp_cap,
            },
            QueueGauge {
                name: "accept_ring",
                used: acc_used,
                cap: acc_cap,
            },
            QueueGauge {
                name: "persist_ring",
                used: persist_used,
                cap: persist_cap,
            },
        ],
        counters: vec![
            CounterGauge {
                name: "orders_processed",
                value: g.orders_processed.load(Ordering::Relaxed),
            },
            CounterGauge {
                name: "fills_processed",
                value: g.fills_processed.load(Ordering::Relaxed),
            },
            CounterGauge {
                name: "rejects",
                value: g.rejects.load(Ordering::Relaxed),
            },
        ],
        state: g.state_label(),
    }
}
