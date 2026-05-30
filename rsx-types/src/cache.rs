//! Cache-line utilities — ONE place for the false-sharing primitives every
//! pinned tile reuses. See `notes/hot-path.md` "False sharing" for the why and
//! for how to *find* it (`perf c2c`, `pahole`).
//!
//! False sharing: two threads write two *different* variables that happen to
//! land on the same cache line. Nothing is actually shared, but MESI doesn't
//! know that — every write invalidates the other core's copy, so the line
//! ping-pongs across the interconnect on every iteration. The fix is purely
//! layout: give each independently-written datum its own line(s).

use std::ops::Deref;
use std::ops::DerefMut;

/// A real cache line is 64 B on x86-64 and aarch64. Use this for *layout*
/// reasoning — which fields land on the same line.
pub const LINE: usize = 64;

/// Padding/alignment used to AVOID false sharing — 128 B, not 64. Intel's
/// adjacent-line (spatial) prefetcher pulls lines in pairs, so the unit of
/// destructive interference is two lines, not one. crossbeam-utils aligns its
/// `CachePadded` to the same 128 on x86-64/aarch64. Over-aligning wastes a few
/// bytes; under-aligning costs a coherence bounce on every write — pick the
/// cheap mistake.
pub const PAD: usize = 128;

/// Wrap a value so it occupies its own padding span (`PAD` bytes), guaranteeing
/// nothing else aligned the same way shares its cache line(s). Reach for it
/// when two threads each write their OWN datum and those would otherwise land
/// on one line: a producer-owned tail vs a consumer-owned head, two per-thread
/// counters, a flag set by one thread and polled by another.
///
/// `align(128)` is a literal because `repr` can't take a const; the
/// `align_matches_pad` test keeps it locked to [`PAD`].
///
/// ```
/// use rsx_types::cache::Padded;
/// let head = Padded::new(0u64);
/// let tail = Padded::new(0u64);
/// assert_eq!(core::mem::align_of_val(&head), rsx_types::cache::PAD);
/// assert_eq!(*head, 0);
/// let _ = tail;
/// ```
#[derive(Clone, Copy, Default, Debug)]
#[repr(align(128))]
pub struct Padded<T>(pub T);

impl<T> Padded<T> {
    pub const fn new(v: T) -> Self {
        Padded(v)
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Padded<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Padded<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

#[cfg(test)]
#[path = "cache_test.rs"]
mod cache_test;
