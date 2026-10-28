use crate::client_ids::ClientOrderIds;
use crate::client_ids::WINDOW_NS;

fn cid(n: u8) -> [u8; 20] {
    [n; 20]
}

// bindings made at 100 and 200, swept at 100 + window + 1 -> only the first leaves, map and queue shrink together
#[test]
fn expire_drops_only_bindings_older_than_the_window() {
    let mut ids = ClientOrderIds::default();
    ids.insert(1, cid(1), [1; 16], 100);
    ids.insert(1, cid(2), [2; 16], 200);

    ids.expire(100 + WINDOW_NS);
    assert_eq!(ids.len(), 2);
    ids.expire(100 + WINDOW_NS + 1);
    assert_eq!(ids.get(1, &cid(1)), None);
    assert_eq!(ids.get(1, &cid(2)), Some([2; 16]));
    assert_eq!(ids.len(), 1);
    ids.expire(200 + WINDOW_NS + 1);
    assert!(ids.is_empty());
    assert!(ids.inserted.is_empty());
}

// same cid for two users -> two bindings
#[test]
fn bindings_are_per_user() {
    let mut ids = ClientOrderIds::default();
    ids.insert(1, cid(1), [1; 16], 100);
    ids.insert(2, cid(1), [2; 16], 100);
    assert_eq!(ids.get(1, &cid(1)), Some([1; 16]));
    assert_eq!(ids.get(2, &cid(1)), Some([2; 16]));
}

// released at 100, rebound at 150 -> the first queue entry expires at 100 + window and leaves the rebinding
#[test]
fn a_released_cid_rebinds_and_the_old_queue_entry_leaves_it_alone() {
    let mut ids = ClientOrderIds::default();
    ids.insert(1, cid(1), [1; 16], 100);
    ids.release(1, &cid(1), &[1; 16]);
    assert_eq!(ids.get(1, &cid(1)), None);

    ids.insert(1, cid(1), [2; 16], 150);
    ids.expire(100 + WINDOW_NS + 1);
    assert_eq!(ids.get(1, &cid(1)), Some([2; 16]));
    ids.expire(150 + WINDOW_NS + 1);
    assert!(ids.is_empty());
}

// release names an order id -> a binding held by another order stays
#[test]
fn release_keeps_a_binding_held_by_another_order() {
    let mut ids = ClientOrderIds::default();
    ids.insert(1, cid(1), [1; 16], 100);
    ids.release(1, &cid(1), &[9; 16]);
    assert_eq!(ids.get(1, &cid(1)), Some([1; 16]));
}
