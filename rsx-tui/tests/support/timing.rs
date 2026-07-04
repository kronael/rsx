//! Timing helpers for the speed comparisons (T5): percentile/min/mean
//! over a sample set, a warmup-then-measure loop, a `Timer` for
//! bracketing one submit -> `wait_for(Fill)` round-trip, and JSON/table
//! emitters so `scripts/tui-bench.sh` and the T6 report can consume
//! the same numbers.
//!
//! `warmup_then_measure`/`Timer`/`to_json`/`table_row` are unused until
//! T5 wires up the actual benches; `pctile`/`min`/`mean` already have
//! `timing_test` unit coverage below.
#![allow(dead_code)]

use std::time::Duration;
use std::time::Instant;

/// The `p`th percentile (0..=100) of `samples`, nearest-rank on a
/// sorted copy. Returns 0 for an empty slice (never panics — callers
/// may run this before any sample has landed).
pub fn pctile(samples: &[u64], p: f64) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let last = sorted.len() - 1;
    let idx = ((p / 100.0) * last as f64).round() as usize;
    sorted[idx.min(last)]
}

/// Minimum of `samples`, 0 if empty.
pub fn min(samples: &[u64]) -> u64 {
    samples.iter().copied().min().unwrap_or(0)
}

/// Mean of `samples` (integer division), 0 if empty.
pub fn mean(samples: &[u64]) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    samples.iter().sum::<u64>() / samples.len() as u64
}

/// Run `one` `warmup` times (discarded, lets connections/caches settle)
/// then `iters` times, collecting each call's return value (a
/// nanosecond duration) into the sample set benches/e2e latency report.
pub fn warmup_then_measure<F>(warmup: usize, iters: usize, mut one: F) -> Vec<u64>
where
    F: FnMut() -> u64,
{
    for _ in 0..warmup {
        one();
    }
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        samples.push(one());
    }
    samples
}

/// Brackets one measured operation (e.g. submit -> `wait_for(Fill)`).
/// `elapsed_ns` reads the clock without consuming the timer so a
/// caller can fold it into `warmup_then_measure`'s closure.
pub struct Timer {
    start: Instant,
}

impl Timer {
    pub fn start() -> Self {
        Timer { start: Instant::now() }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    pub fn elapsed_ns(&self) -> u64 {
        self.elapsed().as_nanos() as u64
    }
}

/// One named metric's p50/p99/min/mean/n as a single-line JSON object
/// (no external json dep — this is the only shape `tui-bench.sh`/T6
/// need, and `serde_json` isn't a dev-dep of this crate).
pub fn to_json(name: &str, samples: &[u64]) -> String {
    format!(
        r#"{{"name":"{name}","p50_ns":{},"p99_ns":{},"min_ns":{},"mean_ns":{},"n":{}}}"#,
        pctile(samples, 50.0),
        pctile(samples, 99.0),
        min(samples),
        mean(samples),
        samples.len(),
    )
}

/// One named metric formatted as a fixed-width table row for the
/// terminal report the bench script prints.
pub fn table_row(name: &str, samples: &[u64]) -> String {
    format!(
        "{name:<24} p50={:>10} p99={:>10} min={:>10} mean={:>10} n={}",
        pctile(samples, 50.0),
        pctile(samples, 99.0),
        min(samples),
        mean(samples),
        samples.len(),
    )
}

#[cfg(test)]
mod timing_test {
    use super::mean;
    use super::min;
    use super::pctile;

    #[test]
    fn pctile_min_mean_on_known_samples() {
        let samples = [10_u64, 20, 30, 40, 50];
        assert_eq!(min(&samples), 10);
        assert_eq!(mean(&samples), 30);
        assert_eq!(pctile(&samples, 0.0), 10);
        assert_eq!(pctile(&samples, 50.0), 30);
        assert_eq!(pctile(&samples, 100.0), 50);
    }

    #[test]
    fn empty_samples_never_panic() {
        let samples: [u64; 0] = [];
        assert_eq!(min(&samples), 0);
        assert_eq!(mean(&samples), 0);
        assert_eq!(pctile(&samples, 50.0), 0);
    }
}
