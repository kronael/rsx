use std::time::Duration;
use std::time::SystemTime;

/// Total time a client has to deliver a complete HTTP upgrade request to the
/// gateway or marketdata.
pub const WS_UPGRADE_DEADLINE: Duration = Duration::from_secs(5);

#[inline]
pub fn time_ns() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

#[inline]
pub fn time_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[inline]
pub fn time() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
