use rsx_book::user::get_or_assign_user;
use rsx_book::user::try_reclaim;
use rsx_book::user::UserState;
use rsx_book::user::RECLAIM_GRACE_NS;
use rustc_hash::FxHashMap;

fn setup() -> (
    Vec<UserState>,
    FxHashMap<u32, u16>,
    Vec<u16>,
    u16,
) {
    (Vec::new(), FxHashMap::default(), Vec::new(), 0)
}

#[test]
fn reclaim_idle_user_after_grace() {
    let (mut states, mut map, mut free, mut bump) =
        setup();
    let idx = get_or_assign_user(
        &mut states, &mut map, &mut free, &mut bump, 42,
    );
    assert_eq!(idx, 0);

    let t0 = 1_000_000_000u64;
    states[idx as usize].mark_zero_if_idle(t0);
    assert_eq!(states[idx as usize].zero_since_ns, t0);

    // Before grace period: no reclaim
    let before = t0 + RECLAIM_GRACE_NS - 1;
    assert!(try_reclaim(
        &mut states, &mut map, &mut free, before, false,
    )
    .is_none());

    // After grace period: reclaim
    let after = t0 + RECLAIM_GRACE_NS;
    let uid = try_reclaim(
        &mut states, &mut map, &mut free, after, false,
    );
    assert_eq!(uid, Some(42));
    assert!(!map.contains_key(&42));
    assert_eq!(free.len(), 1);
}

#[test]
fn new_order_resets_zero_mark() {
    let (mut states, mut map, mut free, mut bump) =
        setup();
    let idx = get_or_assign_user(
        &mut states, &mut map, &mut free, &mut bump, 10,
    );
    states[idx as usize].mark_zero_if_idle(500);
    assert_eq!(states[idx as usize].zero_since_ns, 500);

    // Re-lookup clears zero mark
    let idx2 = get_or_assign_user(
        &mut states, &mut map, &mut free, &mut bump, 10,
    );
    assert_eq!(idx, idx2);
    assert_eq!(states[idx as usize].zero_since_ns, 0);
}

#[test]
fn replay_mode_skips_reclamation() {
    let (mut states, mut map, mut free, mut bump) =
        setup();
    get_or_assign_user(
        &mut states, &mut map, &mut free, &mut bump, 7,
    );
    states[0].mark_zero_if_idle(100);

    let after = 100 + RECLAIM_GRACE_NS + 1;
    assert!(try_reclaim(
        &mut states, &mut map, &mut free, after, true,
    )
    .is_none());
}

#[test]
fn non_idle_user_not_reclaimed() {
    let (mut states, mut map, mut free, mut bump) =
        setup();
    let idx = get_or_assign_user(
        &mut states, &mut map, &mut free, &mut bump, 5,
    );
    states[idx as usize].net_qty = 100;
    states[idx as usize].mark_zero_if_idle(100);
    // zero_since_ns should stay 0 because not idle
    assert_eq!(states[idx as usize].zero_since_ns, 0);

    let after = 100 + RECLAIM_GRACE_NS + 1;
    assert!(try_reclaim(
        &mut states, &mut map, &mut free, after, false,
    )
    .is_none());
}

#[test]
fn reclaimed_slot_reused() {
    let (mut states, mut map, mut free, mut bump) =
        setup();
    get_or_assign_user(
        &mut states, &mut map, &mut free, &mut bump, 1,
    );
    get_or_assign_user(
        &mut states, &mut map, &mut free, &mut bump, 2,
    );
    assert_eq!(bump, 2);

    let t0 = 1_000u64;
    states[0].mark_zero_if_idle(t0);
    let after = t0 + RECLAIM_GRACE_NS;
    try_reclaim(
        &mut states, &mut map, &mut free, after, false,
    );

    // New user should reuse slot 0
    let idx = get_or_assign_user(
        &mut states, &mut map, &mut free, &mut bump, 99,
    );
    assert_eq!(idx, 0);
    assert_eq!(bump, 2); // bump didn't increase
    assert_eq!(states[0].user_id, 99);
}
