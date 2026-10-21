//! Risk responses injected over casting/UDP reach the owning user's sockets
//! as `U`, `F` and `Q` frames, and a completion removes the pending entry.

mod common;

use common::cancelled;
use common::cid;
use common::done;
use common::failed;
use common::fields;
use common::fill;
use common::ids;
use common::inserted;
use common::liquidation;
use common::order;
use common::order_hex;
use common::run;
use common::Options;

// ORDER_INSERTED for user 7 -> U status 1 on user 7's socket only
#[test]
fn inserted_reaches_only_the_owner() {
    run(Options::default(), |gateway| async move {
        let mut owner = gateway.connect(7).await;
        let mut other = gateway.connect(8).await;
        let placed = gateway.place(&mut owner, &order(&cid(1))).await;

        gateway.risk.inject(inserted(ids(&placed), 10)).await;

        let expected = format!(r#"{{"U":["{}",1,0,10,0,0,0,0,0,0]}}"#, order_hex(&placed));
        assert_eq!(owner.recv().await, expected);
        assert_eq!(other.settle().await, Vec::<String>::new());
    });
}

// ORDER_DONE filled -> U status 0 with the fill quantities, pending entry removed
#[test]
fn done_reaches_the_owner_and_removes_pending() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;

        gateway.risk.inject(done(ids(&placed), 10, 0, 0)).await;

        let update = fields(&client.recv().await, "U");
        assert_eq!(update[0], order_hex(&placed));
        assert_eq!(update[1], 0);
        assert_eq!(update[2], 10);
        assert_eq!(update[3], 0);
        assert_eq!(update[4], 0);
        assert!(update[9].as_u64().unwrap() > 0, "gw_out_ns is stamped");
        assert_eq!(gateway.state.borrow().pending.len(), 0);
    });
}

// ORDER_DONE cancelled -> U status 2 with the remaining quantity
#[test]
fn done_cancelled_is_reported_as_status_2() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;

        gateway.risk.inject(done(ids(&placed), 0, 10, 2)).await;

        let update = fields(&client.recv().await, "U");
        assert_eq!(update[1], 2);
        assert_eq!(update[3], 10);
    });
}

// ORDER_CANCELLED -> U status 2 with the remaining quantity, pending entry removed
#[test]
fn cancelled_reaches_the_owner_and_removes_pending() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;

        gateway.risk.inject(cancelled(ids(&placed), 10, 0)).await;

        let expected = format!(r#"{{"U":["{}",2,0,10,0,0,0,0,0,0]}}"#, order_hex(&placed));
        assert_eq!(client.recv().await, expected);
        assert_eq!(gateway.state.borrow().pending.len(), 0);
    });
}

// ORDER_FAILED with reason 2 -> U status 3 carrying the reason, pending entry removed
#[test]
fn failed_reaches_the_owner_with_its_reason() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;

        gateway.risk.inject(failed(ids(&placed), 2)).await;

        let expected = format!(r#"{{"U":["{}",3,0,0,2,0,0,0,0,0]}}"#, order_hex(&placed));
        assert_eq!(client.recv().await, expected);
        assert_eq!(gateway.state.borrow().pending.len(), 0);
    });
}

// failure record naming user 8 for user 7's pending order -> delivered to user 7
#[test]
fn completion_is_routed_by_order_id_not_by_the_record_user() {
    run(Options::default(), |gateway| async move {
        let mut owner = gateway.connect(7).await;
        let mut bystander = gateway.connect(8).await;
        let placed = gateway.place(&mut owner, &order(&cid(1))).await;
        let mut wrong = ids(&placed);
        wrong.user_id = 8;

        gateway.risk.inject(failed(wrong, 2)).await;

        let update = fields(&owner.recv().await, "U");
        assert_eq!(update[0], order_hex(&placed));
        assert_eq!(bystander.settle().await, Vec::<String>::new());
    });
}

// fill between users 7 (taker) and 8 (maker) -> the same F frame on both sockets
#[test]
fn fill_reaches_taker_and_maker() {
    run(Options::default(), |gateway| async move {
        let mut taker = gateway.connect(7).await;
        let mut maker = gateway.connect(8).await;
        let taker_order = gateway.place(&mut taker, &order(&cid(1))).await;
        let maker_order = gateway.place(&mut maker, &order(&cid(2))).await;

        let record = fill(ids(&taker_order), ids(&maker_order), 1000, 10);
        gateway.risk.inject(record).await;

        let to_taker = taker.recv().await;
        assert_eq!(maker.recv().await, to_taker);
        let frame = fields(&to_taker, "F");
        assert_eq!(frame[0], order_hex(&taker_order));
        assert_eq!(frame[1], order_hex(&maker_order));
        assert_eq!(frame[2], 1000);
        assert_eq!(frame[3], 10);
        assert_eq!(frame[4], 7);
        assert_eq!(frame[5], 0);
        assert!(frame[10].as_u64().unwrap() > 0, "gw_out_ns is stamped");
    });
}

// liquidation notice for user 7 -> Q frame on user 7's socket only
#[test]
fn liquidation_reaches_the_user() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let mut other = gateway.connect(8).await;

        gateway.risk.inject(liquidation(7)).await;

        assert_eq!(client.recv().await, r#"{"Q":[0,1,2,1,30,900,15]}"#);
        assert_eq!(other.settle().await, Vec::<String>::new());
    });
}

// user with two sockets -> both receive the update
#[test]
fn update_reaches_every_connection_of_the_user() {
    run(Options::default(), |gateway| async move {
        let mut first = gateway.connect(7).await;
        let mut second = gateway.connect(7).await;
        let placed = gateway.place(&mut first, &order(&cid(1))).await;

        gateway.risk.inject(inserted(ids(&placed), 10)).await;

        let expected = format!(r#"{{"U":["{}",1,0,10,0,0,0,0,0,0]}}"#, order_hex(&placed));
        assert_eq!(first.recv().await, expected);
        assert_eq!(second.recv().await, expected);
    });
}

// self-trade: one user is taker and maker -> a single F frame
#[test]
#[ignore = "BUGS.md GW-SELF-TRADE-FILL-SENT-TWICE"]
fn known_defect_self_trade_fill_is_sent_once() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let taker_order = gateway.place(&mut client, &order(&cid(1))).await;
        let maker_order = gateway.place(&mut client, &order(&cid(2))).await;

        let record = fill(ids(&taker_order), ids(&maker_order), 1000, 10);
        gateway.risk.inject(record).await;

        let first = client.recv().await;
        assert!(first.starts_with(r#"{"F":["#));
        assert_eq!(client.settle().await, Vec::<String>::new());
    });
}

// ORDER_CANCELLED with reason 4 (post-only) -> U carries reason 4
#[test]
#[ignore = "BUGS.md GW-CANCEL-REASON-DROPPED"]
fn known_defect_cancelled_update_carries_the_cancel_reason() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;

        gateway.risk.inject(cancelled(ids(&placed), 10, 4)).await;

        let update = fields(&client.recv().await, "U");
        assert_eq!(update[1], 2);
        assert_eq!(update[4], 4);
    });
}

// ORDER_DONE with an undefined final status -> not reported to the client as filled
#[test]
#[ignore = "BUGS.md GW-ORDER-DONE-UNKNOWN-STATUS-BECOMES-FILLED"]
fn known_defect_done_with_unknown_status_is_not_reported_as_filled() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;

        gateway.risk.inject(done(ids(&placed), 0, 10, 9)).await;

        assert_eq!(client.settle().await, Vec::<String>::new());
    });
}
