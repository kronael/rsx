use crate::user::UserRegistry;
use crate::user::RECLAIM_GRACE_NS;

#[test]
fn reclaim_idle_user_after_grace() {
    let mut reg = UserRegistry::new();
    let idx = reg.get_or_assign(42);
    assert_eq!(idx, 0);

    let t0 = 1_000_000_000u64;
    reg.user_states[idx as usize].mark_zero_if_idle(t0);
    assert_eq!(reg.user_states[idx as usize].zero_since_ns, t0);

    // Before grace period: no reclaim
    let before = t0 + RECLAIM_GRACE_NS - 1;
    assert!(reg.try_reclaim(before, false).is_none());

    // After grace period: reclaim
    let after = t0 + RECLAIM_GRACE_NS;
    let uid = reg.try_reclaim(after, false);
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
    reg.user_states[0].mark_zero_if_idle(100);

    let after = 100 + RECLAIM_GRACE_NS + 1;
    assert!(reg.try_reclaim(after, true).is_none());
}

#[test]
fn non_idle_user_not_reclaimed() {
    let mut reg = UserRegistry::new();
    let idx = reg.get_or_assign(5);
    reg.user_states[idx as usize].net_qty = 100;
    reg.user_states[idx as usize].mark_zero_if_idle(100);
    // zero_since_ns should stay 0 because not idle
    assert_eq!(reg.user_states[idx as usize].zero_since_ns, 0);

    let after = 100 + RECLAIM_GRACE_NS + 1;
    assert!(reg.try_reclaim(after, false).is_none());
}

#[test]
fn reclaimed_slot_reused() {
    let mut reg = UserRegistry::new();
    reg.get_or_assign(1);
    reg.get_or_assign(2);
    assert_eq!(reg.user_bump, 2);

    let t0 = 1_000u64;
    reg.user_states[0].mark_zero_if_idle(t0);
    let after = t0 + RECLAIM_GRACE_NS;
    reg.try_reclaim(after, false);

    // New user should reuse slot 0
    let idx = reg.get_or_assign(99);
    assert_eq!(idx, 0);
    assert_eq!(reg.user_bump, 2); // bump didn't increase
    assert_eq!(reg.user_states[0].user_id, 99);
}
