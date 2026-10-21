use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

#[cfg(test)]
#[path = "gauge_test.rs"]
mod gauge_test;

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
