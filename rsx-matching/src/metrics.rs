//! Values behind the ME `/metrics` endpoint.
//!
//! Every value is a relaxed `AtomicU64` with one writer. The pinned loop
//! writes them with plain stores (no read-modify-write, no lock, no
//! allocation); the health thread reads them when a request arrives and
//! builds the response there, off the hot path.

use rsx_book::event::Event;
use rsx_book::event::OrderFailureReason;
use rsx_health::CounterGauge;
use rsx_health::HealthSnapshot;
use rsx_health::LoadGauges;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// A `u64` with a single writer thread and relaxed readers. `add` is a
/// load and a store, so it is only correct when no other thread writes.
#[derive(Default)]
pub struct Gauge(AtomicU64);

impl Gauge {
    pub fn set(&self, value: u64) {
        self.0.store(value, Ordering::Relaxed);
    }

    pub fn add(&self, amount: u64) {
        self.set(self.get().wrapping_add(amount));
    }

    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// `/metrics` counter names for ORDER_FAILED by reason, in
/// [`reject_slot`] order.
const REJECT_NAMES: [&str; 4] = [
    "rejects_validation",
    "rejects_reduce_only",
    "rejects_fill_or_kill",
    "rejects_duplicate",
];

/// Index of `reason` in [`MatchingMetrics::rejects`] and [`REJECT_NAMES`].
/// Exhaustive, so a new failure reason must be given a slot.
fn reject_slot(reason: OrderFailureReason) -> usize {
    match reason {
        OrderFailureReason::InvalidTick | OrderFailureReason::InvalidLot => 0,
        OrderFailureReason::ReduceOnly => 1,
        OrderFailureReason::FillOrKill => 2,
        OrderFailureReason::Duplicate => 3,
    }
}

/// What the ME exports. The pinned loop is the only writer of every field
/// except `checkpoint_wal_seq`, whose writer is the checkpoint thread.
#[derive(Default)]
pub struct MatchingMetrics {
    /// Order requests handled, duplicates included.
    pub orders_processed: Gauge,
    /// Order requests whose events were published to WAL, risk and marketdata.
    pub publishes: Gauge,
    /// Order seqs skipped over an unrecoverable gap in the risk stream.
    pub drops: Gauge,
    /// Keys in the dedup window.
    pub dedup_entries: Gauge,
    /// ORDER_FAILED records written, by `OrderFailureReason`.
    rejects: [Gauge; 4],
    /// Seq of the last record appended to the WAL (buffered, maybe not on disk).
    pub wal_last_seq: Gauge,
    /// Seq of the last record the loop has seen flushed to disk.
    pub wal_flushed_seq: Gauge,
    /// WAL seq covered by the newest checkpoint durable on disk; 0 for none.
    pub checkpoint_wal_seq: Arc<Gauge>,
    /// Version of the symbol config the book is running; 0 for the env config.
    pub config_version: Gauge,
}

impl MatchingMetrics {
    /// Count one ORDER_FAILED with `reason`.
    pub fn count_reject(&self, reason: OrderFailureReason) {
        self.rejects[reject_slot(reason)].add(1);
    }

    /// Count the ORDER_FAILED events of one match cycle.
    pub fn count_rejects(&self, events: &[Event]) {
        for event in events {
            if let Event::OrderFailed { reason, .. } = *event {
                self.count_reject(reason);
            }
        }
    }

    /// The `/health`, `/ready` and `/metrics` view: liveness from `load`,
    /// every counter by name. Total rejects are the sum of the four
    /// `rejects_*` counters.
    pub fn health_snapshot(&self, load: &LoadGauges) -> HealthSnapshot {
        let mut counters = vec![
            counter("orders_processed", &self.orders_processed),
            counter("publishes", &self.publishes),
            counter("drops", &self.drops),
            counter("dedup_entries", &self.dedup_entries),
        ];
        let mut rejects_total = 0;
        for (name, gauge) in REJECT_NAMES.into_iter().zip(&self.rejects) {
            rejects_total += gauge.get();
            counters.push(counter(name, gauge));
        }
        counters.push(CounterGauge {
            name: "rejects_total",
            value: rejects_total,
        });
        counters.push(counter("wal_last_seq", &self.wal_last_seq));
        counters.push(counter("wal_flushed_seq", &self.wal_flushed_seq));
        counters.push(counter("checkpoint_wal_seq", &self.checkpoint_wal_seq));
        counters.push(counter("config_version", &self.config_version));
        HealthSnapshot {
            live: load.live.load(Ordering::Relaxed),
            ready: load.ready.load(Ordering::Relaxed),
            saturation: 0.0,
            queues: Vec::new(),
            counters,
            state: load.state_label(),
        }
    }
}

fn counter(name: &'static str, gauge: &Gauge) -> CounterGauge {
    CounterGauge {
        name,
        value: gauge.get(),
    }
}

#[cfg(test)]
#[path = "metrics_test.rs"]
mod metrics_test;
