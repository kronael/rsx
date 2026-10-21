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

// same cid for two users -> two bindings; a rebind keeps one queue entry
#[test]
fn bindings_are_per_user_and_a_rebind_adds_no_queue_entry() {
    let mut ids = ClientOrderIds::default();
    ids.insert(1, cid(1), [1; 16], 100);
    ids.insert(2, cid(1), [2; 16], 100);
    assert_eq!(ids.get(1, &cid(1)), Some([1; 16]));
    assert_eq!(ids.get(2, &cid(1)), Some([2; 16]));

    ids.insert(1, cid(1), [3; 16], 150);
    assert_eq!(ids.get(1, &cid(1)), Some([3; 16]));
    assert_eq!(ids.inserted.len(), 2);
}
