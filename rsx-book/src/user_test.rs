use crate::user::UserRegistry;
use crate::user::RECLAIM_GRACE_NS;
use std::io::Cursor;

#[test]
fn reclaim_idle_user_after_grace() {
    let mut reg = UserRegistry::new();
    let idx = reg.get_or_assign(42);
    assert_eq!(idx, 0);

    let t0 = 1_000_000_000u64;
    reg.maintain_one(t0, false);
    assert_eq!(reg.user_states[idx as usize].zero_since_ns, t0);

    // Before grace period: no reclaim
    let before = t0 + RECLAIM_GRACE_NS - 1;
    assert!(reg.maintain_one(before, false).is_none());

    // After grace period: reclaim
    let after = t0 + RECLAIM_GRACE_NS;
    let uid = reg.maintain_one(after, false);
    assert_eq!(uid, Some(42));
    assert!(!reg.user_map.contains_key(&42));
    assert_eq!(reg.user_free_list.len(), 1);
}

#[test]
fn new_order_resets_zero_mark() {
    let mut reg = UserRegistry::new();
    let idx = reg.get_or_assign(10);
    reg.user_states[idx as usize].mark_zero_if_idle(500);
    assert_eq!(reg.user_states[idx as usize].zero_since_ns, 500);

    // Re-lookup clears zero mark
    let idx2 = reg.get_or_assign(10);
    assert_eq!(idx, idx2);
    assert_eq!(reg.user_states[idx as usize].zero_since_ns, 0);
}

#[test]
fn replay_mode_skips_reclamation() {
    let mut reg = UserRegistry::new();
    reg.get_or_assign(7);
    let after = 100 + RECLAIM_GRACE_NS + 1;
    assert!(reg.maintain_one(100, true).is_none());
    assert_eq!(reg.user_states[0].zero_since_ns, 0);
    reg.maintain_one(100, false);
    assert!(reg.maintain_one(after, true).is_none());
    assert!(reg.contains(7));
}

#[test]
fn non_idle_user_not_reclaimed() {
    let mut reg = UserRegistry::new();
    let idx = reg.get_or_assign(5);
    reg.user_states[idx as usize].net_qty = 100;
    reg.maintain_one(100, false);
    assert_eq!(reg.user_states[idx as usize].zero_since_ns, 0);

    let after = 100 + RECLAIM_GRACE_NS + 1;
    assert!(reg.maintain_one(after, false).is_none());
}

#[test]
fn reclaimed_slot_reused() {
    let mut reg = UserRegistry::new();
    reg.get_or_assign(1);
    reg.get_or_assign(2);
    assert_eq!(reg.user_bump, 2);

    let t0 = 1_000u64;
    reg.maintain_one(t0, false);
    let after = t0 + RECLAIM_GRACE_NS;
    reg.maintain_one(after, false);
    reg.maintain_one(after, false);

    // New user should reuse slot 0
    let idx = reg.get_or_assign(99);
    assert_eq!(idx, 0);
    assert_eq!(reg.user_bump, 2); // bump didn't increase
    assert_eq!(reg.user_states[0].user_id, 99);
}

#[test]
fn live_order_and_position_block_reclamation() {
    let mut reg = UserRegistry::new();
    reg.add_order(1);
    reg.get_or_assign(2);
    reg.user_states[1].net_qty = 10;

    for now in [1, 1 + RECLAIM_GRACE_NS, 1 + 2 * RECLAIM_GRACE_NS] {
        reg.maintain_one(now, false);
        reg.maintain_one(now, false);
    }

    assert!(reg.contains(1));
    assert!(reg.contains(2));
}

#[test]
fn churn_reuses_slots_beyond_u16_history() {
    let mut reg = UserRegistry::new();
    for user_id in 1..=70_000 {
        reg.get_or_assign(user_id);
        reg.maintain_one(1, false);
        assert_eq!(reg.maintain_one(1 + RECLAIM_GRACE_NS, false), Some(user_id));
    }

    assert_eq!(reg.user_bump, 1);
    assert_eq!(reg.user_free_list.len(), 1);
}

#[test]
fn snapshot_preserves_live_and_reclaimed_slots() {
    let mut reg = UserRegistry::new();
    reg.add_order(10);
    reg.get_or_assign(20);
    reg.maintain_one(1, false);
    reg.maintain_one(1, false);
    reg.maintain_one(1 + RECLAIM_GRACE_NS, false);
    assert_eq!(reg.maintain_one(1 + RECLAIM_GRACE_NS, false), Some(20));

    let mut bytes = Vec::new();
    reg.write_snapshot(&mut bytes).unwrap();
    let restored = UserRegistry::read_snapshot(&mut Cursor::new(bytes)).unwrap();

    assert_eq!(restored.order_count(10), Some(1));
    assert!(!restored.contains(20));
    assert_eq!(restored.user_bump, 2);
    assert_eq!(restored.user_free_list, vec![1]);
}
