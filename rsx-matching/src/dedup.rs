use rustc_hash::FxHashMap;
use std::collections::VecDeque;
use std::time::Duration;
use std::time::Instant;

const DEDUP_WINDOW: Duration = Duration::from_secs(300);
const DEDUP_CLEANUP_INTERVAL: Duration =
    Duration::from_secs(10);

/// Dedup key: (user_id, order_id_hi, order_id_lo)
type Key = (u32, u64, u64);

pub struct DedupTracker {
    seen: FxHashMap<Key, ()>,
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
            seen: FxHashMap::default(),
            pruning_queue: VecDeque::new(),
            last_cleanup: Instant::now(),
        }
    }

    /// Returns true if duplicate (already seen).
    pub fn check_and_insert(
        &mut self,
        user_id: u32,
        order_id_hi: u64,
        order_id_lo: u64,
    ) -> bool {
        let key = (user_id, order_id_hi, order_id_lo);
        if self.seen.contains_key(&key) {
            return true;
        }
        self.seen.insert(key, ());
        self.pruning_queue
            .push_back((key, Instant::now()));
        false
    }

    /// Prune entries older than 5 minutes.
    /// Call periodically (every 10s).
    pub fn maybe_cleanup(&mut self) {
        if self.last_cleanup.elapsed()
            < DEDUP_CLEANUP_INTERVAL
        {
            return;
        }
        let cutoff = Instant::now() - DEDUP_WINDOW;
        while let Some(&(key, ts)) =
            self.pruning_queue.front()
        {
            if ts >= cutoff {
                break;
            }
            self.pruning_queue.pop_front();
            self.seen.remove(&key);
        }
        self.last_cleanup = Instant::now();
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    /// Force cleanup with custom cutoff (bench/test).
    pub fn cleanup_with_cutoff(
        &mut self,
        cutoff: Instant,
    ) {
        while let Some(&(key, ts)) =
            self.pruning_queue.front()
        {
            if ts >= cutoff {
                break;
            }
            self.pruning_queue.pop_front();
            self.seen.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_order_not_duplicate() {
        let mut d = DedupTracker::new();
        assert!(!d.check_and_insert(1, 0, 1));
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn same_order_is_duplicate() {
        let mut d = DedupTracker::new();
        assert!(!d.check_and_insert(1, 0, 1));
        assert!(d.check_and_insert(1, 0, 1));
    }

    #[test]
    fn different_user_not_duplicate() {
        let mut d = DedupTracker::new();
        assert!(!d.check_and_insert(1, 0, 1));
        assert!(!d.check_and_insert(2, 0, 1));
    }

    #[test]
    fn different_order_id_not_duplicate() {
        let mut d = DedupTracker::new();
        assert!(!d.check_and_insert(1, 0, 1));
        assert!(!d.check_and_insert(1, 0, 2));
    }

    #[test]
    fn cleanup_removes_old_entries() {
        let mut d = DedupTracker::new();
        assert!(!d.check_and_insert(1, 0, 1));
        assert_eq!(d.len(), 1);
        // Force cleanup with future cutoff
        d.cleanup_with_cutoff(
            Instant::now() + Duration::from_secs(1),
        );
        assert_eq!(d.len(), 0);
        // No longer duplicate after pruning
        assert!(!d.check_and_insert(1, 0, 1));
    }

    #[test]
    fn cleanup_preserves_recent() {
        let mut d = DedupTracker::new();
        assert!(!d.check_and_insert(1, 0, 1));
        // Cutoff in the past: nothing to prune
        d.cleanup_with_cutoff(
            Instant::now() - Duration::from_secs(1),
        );
        assert_eq!(d.len(), 1);
        assert!(d.check_and_insert(1, 0, 1));
    }
}
