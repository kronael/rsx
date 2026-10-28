use rsx_gateway::pending::PendingOrder;
use rsx_gateway::pending::PendingOrders;
use rsx_gateway::pending::PendingStage;

fn make_order(id_byte: u8) -> PendingOrder {
    let mut order_id = [0u8; 16];
    order_id[0] = id_byte;
    PendingOrder {
        order_id,
        user_id: 1,
        symbol_id: 1,
        client_order_id: [id_byte; 20],
        timestamp_ns: id_byte as u64 * 1_000_000,
        stage: PendingStage::Submitted,
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
    let removed = p.remove(1, &id).unwrap();
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
    let removed = p.remove(1, &id).unwrap();
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
    assert!(p.remove(1, &id).is_some());
    assert!(p.remove(1, &id).is_none());
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
    p.remove(1, &id1);
    p.remove(1, &id2);
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
            stage: PendingStage::Submitted,
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
            stage: PendingStage::Submitted,
        }));
    }
    assert!(p.is_full());
    assert!(!p.push(make_order(99)));
}

#[test]
fn pending_timeout_keeps_stale_orders_tracked_as_timed_out() {
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
    // Time out orders older than 10s
    let stale = p.time_out(10_000_000_000);
    assert_eq!(stale.len(), 2);
    assert_eq!(p.len(), 3);
    assert_eq!((p.in_flight(), p.timed_out()), (1, 2));
    // Remaining submitted order is the one at 15s
    let mut id3 = [0u8; 16];
    id3[0] = 3;
    assert!(p.remove(1, &id3).is_some());
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
    p.remove(1, &id);
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
        stage: PendingStage::Submitted,
    };
    pending.push(order);

    assert!(pending.find_by_client_order_id(42, &cid).is_some());
    assert_eq!(
        pending.find_by_client_order_id(42, &cid).unwrap().user_id,
        42,
    );

    let mut other_cid = [0u8; 20];
    other_cid[..5].copy_from_slice(b"test2");
    assert!(pending.find_by_client_order_id(42, &other_cid).is_none());
}

fn make_user_order(user_id: u32, id_byte: u8, cid: [u8; 20]) -> PendingOrder {
    let mut order = make_order(id_byte);
    order.user_id = user_id;
    order.client_order_id = cid;
    order
}

#[test]
fn find_by_client_order_id_is_scoped_to_the_user() {
    let mut p = PendingOrders::new(10);
    p.push(make_user_order(1, 1, [7; 20]));
    p.push(make_user_order(2, 2, [7; 20]));

    assert_eq!(
        p.find_by_client_order_id(1, &[7; 20]).unwrap().order_id[0],
        1
    );
    assert_eq!(
        p.find_by_client_order_id(2, &[7; 20]).unwrap().order_id[0],
        2
    );
    assert!(p.find_by_client_order_id(3, &[7; 20]).is_none());
}

#[test]
fn find_by_order_id_is_scoped_to_the_user() {
    let mut p = PendingOrders::new(10);
    p.push(make_user_order(1, 1, [1; 20]));
    let mut id = [0u8; 16];
    id[0] = 1;

    assert!(p.find_by_order_id(1, &id).is_some());
    assert!(p.find_by_order_id(2, &id).is_none());
}

#[test]
fn remove_finds_the_order_when_the_user_hint_is_wrong() {
    let mut p = PendingOrders::new(10);
    p.push(make_user_order(1, 1, [1; 20]));
    let mut id = [0u8; 16];
    id[0] = 1;

    assert_eq!(p.remove(2, &id).unwrap().user_id, 1);
    assert!(p.is_empty());
}

#[test]
fn capacity_is_shared_across_users() {
    let mut p = PendingOrders::new(2);
    assert!(p.push(make_user_order(1, 1, [1; 20])));
    assert!(p.push(make_user_order(2, 2, [2; 20])));
    assert!(p.is_full());
    assert!(!p.push(make_user_order(3, 3, [3; 20])));
    assert_eq!(p.len(), 2);
}

#[test]
fn time_out_counts_across_users_and_keeps_fresh_orders() {
    let mut p = PendingOrders::new(10);
    let mut old_a = make_user_order(1, 1, [1; 20]);
    old_a.timestamp_ns = 1;
    let mut old_b = make_user_order(2, 2, [2; 20]);
    old_b.timestamp_ns = 2;
    let mut fresh = make_user_order(2, 3, [3; 20]);
    fresh.timestamp_ns = 100;
    p.push(old_a);
    p.push(old_b);
    p.push(fresh);

    assert_eq!(p.time_out(50).len(), 2);
    assert_eq!(p.len(), 3);
    assert_eq!(p.in_flight(), 1);
    assert!(p.find_by_client_order_id(2, &[3; 20]).is_some());
    assert!(p.push(make_user_order(1, 4, [4; 20])));
}

#[test]
fn time_out_keeps_resting_orders() {
    let mut p = PendingOrders::new(10);
    let mut resting = make_user_order(1, 1, [1; 20]);
    resting.timestamp_ns = 1;
    let mut unanswered = make_user_order(1, 2, [2; 20]);
    unanswered.timestamp_ns = 1;
    p.push(resting);
    p.push(unanswered);
    let mut resting_id = [0u8; 16];
    resting_id[0] = 1;
    assert!(p.mark_resting(1, &resting_id));

    let stale = p.time_out(50);

    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].order_id[0], 2);
    assert_eq!(p.len(), 2);
    assert_eq!(p.resting(), 1);
    assert!(p.find_by_order_id(1, &resting_id).is_some());
}

#[test]
fn mark_resting_finds_the_order_when_the_user_hint_is_wrong() {
    let mut p = PendingOrders::new(10);
    let mut order = make_user_order(1, 1, [1; 20]);
    order.timestamp_ns = 1;
    p.push(order);
    let mut id = [0u8; 16];
    id[0] = 1;

    assert!(p.mark_resting(2, &id));
    assert!(p.time_out(50).is_empty());
}

#[test]
fn mark_resting_of_an_unknown_order_is_false() {
    let mut p = PendingOrders::new(10);
    p.push(make_user_order(1, 1, [1; 20]));

    assert!(!p.mark_resting(1, &[9u8; 16]));
}

fn order_id(id_byte: u8) -> [u8; 16] {
    let mut id = [0u8; 16];
    id[0] = id_byte;
    id
}

#[test]
fn resting_orders_do_not_count_against_capacity() {
    let mut p = PendingOrders::new(2);
    for n in 1..=5 {
        let mut order = make_user_order(1, n, [n; 20]);
        order.stage = PendingStage::Resting;
        assert!(p.push(order));
    }
    assert!(!p.is_full());
    assert_eq!(p.len(), 5);
    assert_eq!(p.in_flight(), 0);
    assert_eq!(p.resting(), 5);
    assert!(p.push(make_user_order(2, 6, [6; 20])));
    assert!(p.push(make_user_order(3, 7, [7; 20])));
    assert!(p.is_full());
    assert!(!p.push(make_user_order(4, 8, [8; 20])));
    assert_eq!(p.in_flight(), 2);
    assert_eq!(p.resting(), 5);
}

#[test]
fn mark_resting_frees_one_capacity_slot_once() {
    let mut p = PendingOrders::new(1);
    p.push(make_order(1));
    assert!(p.is_full());

    assert!(p.mark_resting(1, &order_id(1)));
    assert!(p.mark_resting(1, &order_id(1)));

    assert!(!p.is_full());
    assert_eq!(p.in_flight(), 0);
    assert_eq!(p.resting(), 1);
    assert!(p.push(make_order(2)));
    assert!(p.is_full());
}

#[test]
fn remove_adjusts_the_count_of_the_orders_stage() {
    let mut p = PendingOrders::new(10);
    p.push(make_order(1));
    p.push(make_order(2));
    p.mark_resting(1, &order_id(1));

    assert_eq!((p.in_flight(), p.resting()), (1, 1));
    assert!(p.remove(1, &order_id(1)).is_some());
    assert_eq!((p.in_flight(), p.resting()), (1, 0));
    assert!(p.remove(1, &order_id(2)).is_some());
    assert_eq!((p.in_flight(), p.resting()), (0, 0));
}

#[test]
fn time_out_leaves_the_resting_count() {
    let mut p = PendingOrders::new(10);
    p.push(make_order(1));
    p.push(make_order(2));
    p.mark_resting(1, &order_id(1));

    assert_eq!(p.time_out(u64::MAX).len(), 1);
    assert_eq!((p.in_flight(), p.timed_out(), p.resting()), (0, 1, 1));
}

#[test]
fn a_timed_out_order_holds_no_capacity_slot_and_stays_cancellable() {
    let mut p = PendingOrders::new(1);
    p.push(make_order(1));
    assert!(p.is_full());

    p.time_out(u64::MAX);

    assert!(!p.is_full());
    assert!(p.find_by_order_id(1, &order_id(1)).is_some());
    assert!(p.push(make_order(2)));
    assert!(p.is_full());
}

#[test]
fn a_late_insert_turns_a_timed_out_order_resting() {
    let mut p = PendingOrders::new(10);
    p.push(make_order(1));
    p.time_out(u64::MAX);

    assert!(p.mark_resting(1, &order_id(1)));

    assert_eq!((p.in_flight(), p.timed_out(), p.resting()), (0, 0, 1));
    assert!(p.time_out(u64::MAX).is_empty());
    assert!(p.remove(1, &order_id(1)).is_some());
    assert_eq!(p.len(), 0);
}

#[test]
fn removing_a_timed_out_order_adjusts_its_count() {
    let mut p = PendingOrders::new(10);
    p.push(make_order(1));
    p.time_out(u64::MAX);

    assert!(p.remove(1, &order_id(1)).is_some());
    assert_eq!((p.len(), p.timed_out()), (0, 0));
}

#[test]
fn drop_timed_out_removes_only_old_timed_out_orders() {
    let mut p = PendingOrders::new(10);
    let mut old = make_user_order(1, 1, [1; 20]);
    old.timestamp_ns = 10;
    let mut newer = make_user_order(2, 2, [2; 20]);
    newer.timestamp_ns = 100;
    let mut submitted = make_user_order(3, 3, [3; 20]);
    submitted.timestamp_ns = 10;
    p.push(old);
    p.push(newer);
    p.time_out(200);
    p.push(submitted);

    p.drop_timed_out(50);

    assert_eq!((p.len(), p.timed_out(), p.in_flight()), (2, 1, 1));
    assert!(p.find_by_order_id(1, &order_id(1)).is_none());
    assert!(p.find_by_order_id(2, &order_id(2)).is_some());
    assert!(p.find_by_order_id(3, &order_id(3)).is_some());
}
