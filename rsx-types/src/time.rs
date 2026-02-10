use std::time::SystemTime;

#[inline]
pub fn time_ns() -> u64 {
    // SAFETY: SystemTime always >= UNIX_EPOCH
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

#[inline]
pub fn time_us() -> u64 {
    // SAFETY: SystemTime always >= UNIX_EPOCH
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64
}

#[inline]
pub fn time_ms() -> u64 {
    // SAFETY: SystemTime always >= UNIX_EPOCH
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[inline]
pub fn time() -> u64 {
    // SAFETY: SystemTime always >= UNIX_EPOCH
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
