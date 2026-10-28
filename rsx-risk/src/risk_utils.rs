use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

/// RISK.md §1. Fee with floor division.
/// Uses i128 intermediate to prevent overflow.
pub fn calculate_fee(qty: i64, price: i64, fee_bps: i64) -> i64 {
    let notional = qty as i128 * price as i128;
    let fee_128 = notional * fee_bps as i128;
    let result = fee_128.div_euclid(10_000);
    // Saturate toward the correct sign: a huge rebate (negative
    // fee) must NOT invert into a huge positive cost.
    i64::try_from(result).unwrap_or(if result < 0 { i64::MIN } else { i64::MAX })
}

/// Sleep `total` in 100 ms slices; returns early once `shutdown` is
/// set.
pub fn sleep_unless_shutdown(total: Duration, shutdown: &AtomicBool) {
    let slice = Duration::from_millis(100);
    let deadline = Instant::now() + total;
    while !shutdown.load(Ordering::Relaxed) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        std::thread::sleep(left.min(slice));
    }
}

/// Cheap uniform [0,1) from the clock, for retry jitter. Not
/// cryptographic and deliberately dependency-free -- the only
/// requirement is that two peers backing off do not stay in step.
pub fn rand_jitter() -> f64 {
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(12345);
    (nanos % 1_000_003) as f64 / 1_000_003.0
}

#[cfg(test)]
#[path = "risk_utils_test.rs"]
mod risk_utils_test;
