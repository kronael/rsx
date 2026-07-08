use rustc_hash::FxHashSet;
use std::collections::VecDeque;
use std::time::Duration;
use std::time::Instant;

const DEDUP_WINDOW: Duration = Duration::from_secs(3600);
const DEDUP_CLEANUP_INTERVAL: Duration = Duration::from_secs(10);

/// Dedup key: (user_id, order_id_hi, order_id_lo)
type Key = (u32, u64, u64);

pub struct DedupTracker {
    seen: FxHashSet<Key>,
    pruning_queue: VecDeque<(Key, Instant)>,
    last_cleanup: Instant,
}

impl Default for DedupTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DedupTracker {
    pub fn new() -> Self {
        Self {
            seen: FxHashSet::default(),
            pruning_queue: VecDeque::new(),
            last_cleanup: Instant::now(),
        }
    }

    /// Returns true if duplicate (already seen).
    pub fn check_and_insert(&mut self, user_id: u32, order_id_hi: u64, order_id_lo: u64) -> bool {
        let key = (user_id, order_id_hi, order_id_lo);
        if !self.seen.insert(key) {
            return true;
        }
        self.pruning_queue.push_back((key, Instant::now()));
        false
    }

    /// Seed a key on recovery with the age it was originally inserted,
    /// so the existing Instant-based pruning expires it at the right
    /// time. Used to rebuild the dedup window from the WAL after a
    /// restart (see `wal::rebuild_dedup_window`). Skips keys already
    /// outside the window (`inserted_ago >= DEDUP_WINDOW` — nothing left
    /// to protect) and keys already tracked (idempotent). Callers should
    /// seed in WAL/seq order (oldest records first, i.e. largest
    /// `inserted_ago` first) so `inserted_at` is non-decreasing and
    /// `pruning_queue` stays ordered for cleanup.
    pub fn seed(
        &mut self,
        user_id: u32,
        order_id_hi: u64,
        order_id_lo: u64,
        inserted_ago: Duration,
    ) {
        if inserted_ago >= DEDUP_WINDOW {
            return;
        }
        let key = (user_id, order_id_hi, order_id_lo);
        if !self.seen.insert(key) {
            return;
        }
        // `inserted_ago < DEDUP_WINDOW`, so the subtraction is small; guard
        // it anyway for a host with < DEDUP_WINDOW uptime (monotonic clock
        // can't represent the past instant) — fall back to now, which keeps
        // the key slightly longer (conservative, never a false negative).
        let inserted_at = Instant::now()
            .checked_sub(inserted_ago)
            .unwrap_or_else(Instant::now);
        self.pruning_queue.push_back((key, inserted_at));
    }

    /// Prune entries older than `DEDUP_WINDOW`. Call periodically (every 10s).
    pub fn maybe_cleanup(&mut self) {
        if self.last_cleanup.elapsed() < DEDUP_CLEANUP_INTERVAL {
            return;
        }
        self.evict(Instant::now() - DEDUP_WINDOW);
        self.last_cleanup = Instant::now();
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    /// Evict entries inserted before `cutoff`.
    pub fn evict(&mut self, cutoff: Instant) {
        while let Some(&(key, ts)) = self.pruning_queue.front() {
            if ts >= cutoff {
                break;
            }
            self.pruning_queue.pop_front();
            self.seen.remove(&key);
        }
    }
}

#[cfg(test)]
#[path = "dedup_test.rs"]
mod dedup_test;
