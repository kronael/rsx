use rsx_gateway::pending::PendingOrder;
use rsx_gateway::pending::PendingOrders;

fn make_order(id_byte: u8) -> PendingOrder {
    let mut order_id = [0u8; 16];
    order_id[0] = id_byte;
    PendingOrder {
        order_id,
        user_id: 1,
        symbol_id: 1,
        client_order_id: [id_byte; 20],
        timestamp_ns: id_byte as u64 * 1_000_000,
    }
}

#[test]
fn pending_push_back_new_order() {
    let mut p = PendingOrders::new(100);
    assert!(p.push(make_order(1)));
    assert_eq!(p.len(), 1);
}

#[test]
fn pending_pop_back_lifo_match() {
    let mut p = PendingOrders::new(100);
    p.push(make_order(1));
    p.push(make_order(2));
    p.push(make_order(3));
    let mut id = [0u8; 16];
    id[0] = 3;
    let removed = p.remove(&id).unwrap();
    assert_eq!(removed.order_id[0], 3);
    assert_eq!(p.len(), 2);
}

#[test]
fn pending_linear_scan_on_mismatch() {
    let mut p = PendingOrders::new(100);
    p.push(make_order(1));
    p.push(make_order(2));
    p.push(make_order(3));
    let mut id = [0u8; 16];
    id[0] = 1;
    let removed = p.remove(&id).unwrap();
    assert_eq!(removed.order_id[0], 1);
    assert_eq!(p.len(), 2);
}

#[test]
fn pending_remove_by_order_id() {
    let mut p = PendingOrders::new(100);
    p.push(make_order(10));
    p.push(make_order(20));
    let mut id = [0u8; 16];
    id[0] = 10;
    assert!(p.remove(&id).is_some());
    assert!(p.remove(&id).is_none());
}

#[test]
fn pending_empty_after_all_removed() {
    let mut p = PendingOrders::new(100);
    p.push(make_order(1));
    p.push(make_order(2));
    let mut id1 = [0u8; 16];
    id1[0] = 1;
    let mut id2 = [0u8; 16];
    id2[0] = 2;
    p.remove(&id1);
    p.remove(&id2);
    assert!(p.is_empty());
}

#[test]
fn pending_multiple_orders_same_user() {
    let mut p = PendingOrders::new(100);
    let mut o1 = make_order(1);
    o1.user_id = 42;
    let mut o2 = make_order(2);
    o2.user_id = 42;
    p.push(o1);
    p.push(o2);
    assert_eq!(p.len(), 2);
}

#[test]
fn backpressure_accepts_under_10k() {
    let mut p = PendingOrders::new(10_000);
    for i in 0..9_999u16 {
        let mut order_id = [0u8; 16];
        order_id[0] = (i & 0xFF) as u8;
        order_id[1] = (i >> 8) as u8;
        assert!(p.push(PendingOrder {
            order_id,
            user_id: 1,
            symbol_id: 1,
            client_order_id: [0; 20],
            timestamp_ns: i as u64,
        }));
    }
    assert!(!p.is_full());
}

#[test]
fn backpressure_rejects_at_10k_overloaded() {
    let mut p = PendingOrders::new(10_000);
    for i in 0..10_000u16 {
        let mut order_id = [0u8; 16];
        order_id[0] = (i & 0xFF) as u8;
        order_id[1] = (i >> 8) as u8;
        assert!(p.push(PendingOrder {
            order_id,
            user_id: 1,
            symbol_id: 1,
            client_order_id: [0; 20],
            timestamp_ns: i as u64,
        }));
    }
    assert!(p.is_full());
    assert!(!p.push(make_order(99)));
}

#[test]
fn pending_timeout_removes_stale_order() {
    let mut p = PendingOrders::new(100);
    let mut o1 = make_order(1);
    o1.timestamp_ns = 1_000_000_000; // 1s
    let mut o2 = make_order(2);
    o2.timestamp_ns = 5_000_000_000; // 5s
    let mut o3 = make_order(3);
    o3.timestamp_ns = 15_000_000_000; // 15s
    p.push(o1);
    p.push(o2);
    p.push(o3);
    // Remove orders older than 10s
    let stale = p.remove_stale(10_000_000_000);
    assert_eq!(stale.len(), 2);
    assert_eq!(p.len(), 1);
    // Remaining order is the one at 15s
    let mut id3 = [0u8; 16];
    id3[0] = 3;
    assert!(p.remove(&id3).is_some());
}

#[test]
fn backpressure_resumes_after_drain() {
    let mut p = PendingOrders::new(3);
    p.push(make_order(1));
    p.push(make_order(2));
    p.push(make_order(3));
    assert!(!p.push(make_order(4)));
    let mut id = [0u8; 16];
    id[0] = 3;
    p.remove(&id);
    assert!(p.push(make_order(5)));
}

#[test]
fn find_by_client_order_id() {
    let mut pending = PendingOrders::new(10);
    let mut cid = [0u8; 20];
    cid[..5].copy_from_slice(b"test1");
    let order = PendingOrder {
        order_id: [1u8; 16],
        user_id: 42,
        symbol_id: 0,
        client_order_id: cid,
        timestamp_ns: 100,
    };
    pending.push(order);

    assert!(pending.find_by_client_order_id(&cid).is_some());
    assert_eq!(
        pending.find_by_client_order_id(&cid).unwrap().user_id,
        42,
    );

    let mut other_cid = [0u8; 20];
    other_cid[..5].copy_from_slice(b"test2");
    assert!(
        pending.find_by_client_order_id(&other_cid).is_none()
    );
}
