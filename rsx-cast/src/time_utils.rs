//! `time_ns`: wall-clock helper. Local copy — `rsx-cast` has no workspace deps.

use std::time::SystemTime;
use std::time::UNIX_EPOCH;

#[inline]
pub fn time_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
