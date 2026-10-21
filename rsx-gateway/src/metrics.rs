//! Values behind the gateway `/metrics` endpoint.
//!
//! Every value is a relaxed `AtomicU64` with one writer: the monoio reactor
//! thread, which runs the connection handlers, the `route_*` functions and the
//! housekeeping loop. It writes with plain stores (no read-modify-write, no
//! lock, no allocation); the health thread reads when a request arrives and
//! builds the response there, off the hot path.

use crate::circuit::State;
use rsx_health::CounterGauge;
use rsx_health::Gauge;
use rsx_health::HealthSnapshot;
use rsx_health::LoadGauges;
use rsx_health::QueueGauge;
use rsx_types::FailureReason;
use std::sync::atomic::Ordering;

/// Slot for a failure code outside `FailureReason`.
const OTHER_SLOT: usize = FailureReason::FillOrKill as usize + 1;

/// `/metrics` counter names for rejects, indexed by `FailureReason` code,
/// then the slot for codes outside the enum.
const REJECT_NAMES: [&str; OTHER_SLOT + 1] = [
    "rejects_invalid_tick_size",
    "rejects_invalid_lot_size",
    "rejects_symbol_not_found",
    "rejects_duplicate_order_id",
    "rejects_insufficient_margin",
    "rejects_overloaded",
    "rejects_internal_error",
    "rejects_reduce_only_violation",
    "rejects_network_error",
    "rejects_rate_limit",
    "rejects_timeout",
    "rejects_user_in_liquidation",
    "rejects_wrong_shard",
    "rejects_fill_or_kill",
    "rejects_other",
];

/// `/ready` turns false when the pending set is at least this full.
const READY_SATURATION: f64 = 0.9;

/// What the gateway exports besides the connection and pending gauges in
/// `LoadGauges`.
#[derive(Default)]
pub struct GatewayMetrics {
    /// Orders forwarded to risk.
    pub orders_processed: Gauge,
    /// Orders refused or failed, by `FailureReason`: the gateway's own
    /// rejects (rate limit, overload, validation, forward failure, timeout)
    /// and the ORDER_FAILED records risk and the ME send back.
    rejects: [Gauge; OTHER_SLOT + 1],
    /// Circuit breaker state: 0 closed, 1 open, 2 half-open.
    pub circuit_state: Gauge,
}

impl GatewayMetrics {
    /// Count one rejected order.
    pub fn count_reject(&self, reason: FailureReason) {
        self.rejects[reason as usize].add(1);
    }

    /// Count one ORDER_FAILED record by its wire `reason` code.
    pub fn count_reject_code(&self, code: u8) {
        self.rejects[usize::from(code).min(OTHER_SLOT)].add(1);
    }

    /// Rejects counted for `reason`.
    pub fn rejects(&self, reason: FailureReason) -> u64 {
        self.rejects[reason as usize].get()
    }

    /// Rejects counted over every reason.
    pub fn rejects_total(&self) -> u64 {
        self.rejects.iter().map(Gauge::get).sum()
    }

    pub fn set_circuit_state(&self, state: State) {
        self.circuit_state.set(match state {
            State::Closed => 0,
            State::Open => 1,
            State::HalfOpen => 2,
        });
    }

    /// The `/health`, `/ready` and `/metrics` view: liveness and the
    /// connection and pending gauges from `load`, every counter by name.
    /// `/ready` is false while the pending set is 90% full.
    pub fn health_snapshot(&self, load: &LoadGauges, max_pending: u64) -> HealthSnapshot {
        let pending = load.pending_orders.load(Ordering::Relaxed);
        let saturation = if max_pending > 0 {
            pending as f64 / max_pending as f64
        } else {
            0.0
        };
        let live = load.live.load(Ordering::Relaxed);
        let mut counters = vec![
            CounterGauge {
                name: "connections",
                value: load.connections.load(Ordering::Relaxed),
            },
            counter("orders_processed", &self.orders_processed),
        ];
        for (name, gauge) in REJECT_NAMES.into_iter().zip(&self.rejects) {
            counters.push(counter(name, gauge));
        }
        counters.push(CounterGauge {
            name: "rejects_total",
            value: self.rejects_total(),
        });
        counters.push(counter("circuit_state", &self.circuit_state));
        HealthSnapshot {
            live,
            ready: live && saturation < READY_SATURATION,
            saturation,
            queues: vec![QueueGauge {
                name: "pending",
                used: pending,
                cap: max_pending,
            }],
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
