use std::ops::Sub;
use std::time::Instant;
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct PerfCounterNanos(Instant);
impl Sub for PerfCounterNanos {
    type Output = u64;
    fn sub(self, rhs: Self) -> u64 {
        (self.0 - rhs.0).as_nanos() as u64
    }
}

#[inline]
pub fn perf_counter_ns() -> PerfCounterNanos {
    PerfCounterNanos(Instant::now())
}

#[derive(Clone, Debug)]
pub struct PerfCounter(Instant);
impl Sub for PerfCounter {
    type Output = u64;
    fn sub(self, rhs: Self) -> u64 {
        (self.0 - rhs.0).as_micros() as u64
    }
}

#[inline]
pub fn perf_counter() -> PerfCounter {
    PerfCounter(Instant::now())
}

#[inline]
pub fn time_ns() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

#[inline]
pub fn time_us() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64
}

#[inline]
pub fn time_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[inline]
pub fn time() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
