//! Time helpers local to rsx-cast.
//!
//! `rsx-cast` cannot depend on `rsx-types` (zero-dep transport
//! invariant; see crate-root CLAUDE.md). So we keep a tiny
//! local `time_ns` rather than reaching across.

use std::time::SystemTime;
use std::time::UNIX_EPOCH;

#[inline]
pub fn time_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
