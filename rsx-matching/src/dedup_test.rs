use crate::dedup::DedupTracker;
use crate::dedup::DEDUP_WINDOW;
use std::time::Duration;
use std::time::Instant;

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
fn seed_respects_window() {
    let mut d = DedupTracker::new();
    // Seeded inside the window: a later resend is a duplicate.
    d.seed(1, 0, 1, DEDUP_WINDOW - Duration::from_secs(1));
    assert!(d.check_and_insert(1, 0, 1), "in-window seed → duplicate");
    // Seeded at/after the window: skipped, so not a duplicate.
    d.seed(2, 0, 2, DEDUP_WINDOW);
    assert!(
        !d.check_and_insert(2, 0, 2),
        "expired seed → not a duplicate",
    );
}

#[test]
fn evict_removes_old_entries() {
    let mut d = DedupTracker::new();
    assert!(!d.check_and_insert(1, 0, 1));
    assert_eq!(d.len(), 1);
    // Evict with a future cutoff removes everything.
    d.evict(Instant::now() + Duration::from_secs(1));
    assert_eq!(d.len(), 0);
    // No longer duplicate after eviction.
    assert!(!d.check_and_insert(1, 0, 1));
}

#[test]
fn evict_preserves_recent() {
    let mut d = DedupTracker::new();
    assert!(!d.check_and_insert(1, 0, 1));
    // Cutoff in the past: nothing to evict.
    d.evict(Instant::now() - Duration::from_secs(1));
    assert_eq!(d.len(), 1);
    assert!(d.check_and_insert(1, 0, 1));
}
