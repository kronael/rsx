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
    /// Coarse clock refreshed by the hot loop (see `refresh_clock`) so the
    /// per-order path never pays `Instant::now()`. Precision to within the
    /// loop's refresh interval is irrelevant against the hour-long window.
    now: Instant,
}

impl Default for DedupTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DedupTracker {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            seen: FxHashSet::default(),
            pruning_queue: VecDeque::new(),
            last_cleanup: now,
            now,
        }
    }

    /// Advance the cached clock. The ME hot loop calls this once every N
    /// spins with a freshly sampled `Instant`; `check_and_insert` and
    /// `maybe_cleanup` read it instead of sampling per call.
    pub fn refresh_clock(&mut self, now: Instant) {
        self.now = now;
    }

    /// Returns true if duplicate (already seen).
    pub fn check_and_insert(&mut self, user_id: u32, order_id_hi: u64, order_id_lo: u64) -> bool {
        let key = (user_id, order_id_hi, order_id_lo);
        if !self.seen.insert(key) {
            return true;
        }
        self.pruning_queue.push_back((key, self.now));
        false
    }

    /// Seed a key on recovery with the age it was originally inserted, so
    /// pruning expires it on time (`wal::rebuild_dedup_window`). Skips keys
    /// outside the window and keys already tracked. Callers seed oldest
    /// first (largest `inserted_ago` first) so `pruning_queue` stays ordered.
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
        // A host with less uptime than `inserted_ago` cannot represent the
        // past instant: fall back to now, which keeps the key slightly
        // longer (never a false negative).
        let inserted_at = Instant::now()
            .checked_sub(inserted_ago)
            .unwrap_or_else(Instant::now);
        self.pruning_queue.push_back((key, inserted_at));
    }

    /// Prune entries older than `DEDUP_WINDOW`. Call periodically (every 10s).
    /// Uses the cached clock (`refresh_clock`).
    pub fn maybe_cleanup(&mut self) {
        if self.now.duration_since(self.last_cleanup) < DEDUP_CLEANUP_INTERVAL {
            return;
        }
        // With less uptime than the window nothing is old enough to evict.
        if let Some(cutoff) = self.now.checked_sub(DEDUP_WINDOW) {
            self.evict(cutoff);
        }
        self.last_cleanup = self.now;
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
