//! The connection handler's cancel path: a cancel names an order by order id
//! (32 hex) or client order id and is forwarded to risk only when the order
//! is tracked as pending; an untracked order is answered with an `E` frame.

mod common;

use common::cancel;
use common::cid;
use common::done;
use common::error_code;
use common::failed;
use common::ids;
use common::inserted;
use common::order;
use common::order_hex;
use common::run;
use common::Options;
use rsx_gateway::pending::PendingOrder;
use rsx_gateway::pending::PendingStage;

// cancel by 32-hex order id of a pending order -> one cancel record with that order's ids, entry kept
#[test]
fn cancel_by_order_id_is_forwarded_to_risk() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let frame = format!(r#"{{"N":[1,0,1000,10,"{}",0]}}"#, cid(1));
        let placed = gateway.place(&mut client, &frame).await;

        client.send_text(&cancel(&order_hex(&placed))).await;
        assert_eq!(client.settle().await, Vec::<String>::new());

        let cancels = gateway.risk.take_cancels();
        assert_eq!(cancels.len(), 1);
        assert_eq!(cancels[0].seq, 2);
        assert_eq!(cancels[0].user_id, 7);
        assert_eq!(cancels[0].symbol_id, 1);
        assert_eq!(
            (cancels[0].order_id_hi, cancels[0].order_id_lo),
            (placed.order_id_hi, placed.order_id_lo)
        );
        assert_eq!(gateway.state.borrow().pending.len(), 1);
    });
}

// cancel by the 20-byte client order id of a pending order -> one cancel record with that order's ids
#[test]
fn cancel_by_client_order_id_is_forwarded_to_risk() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        gateway.place(&mut client, &order(&cid(1))).await;
        let second = gateway.place(&mut client, &order(&cid(2))).await;

        client.send_text(&cancel(&cid(2))).await;
        assert_eq!(client.settle().await, Vec::<String>::new());

        let cancels = gateway.risk.take_cancels();
        assert_eq!(cancels.len(), 1);
        assert_eq!(
            (cancels[0].order_id_hi, cancels[0].order_id_lo),
            (second.order_id_hi, second.order_id_lo)
        );
    });
}

// order that rested (ORDER_INSERTED) and is still pending -> a cancel by order id reaches risk
#[test]
fn resting_order_can_be_cancelled() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;
        gateway.risk.inject(inserted(ids(&placed), 10)).await;
        client.settle().await;

        client.send_text(&cancel(&order_hex(&placed))).await;
        assert_eq!(client.settle().await, Vec::<String>::new());
        assert_eq!(gateway.risk.take_cancels().len(), 1);
    });
}

// cancel of a client order id that was never placed -> E[1005], nothing forwarded
#[test]
fn cancel_of_unknown_client_order_id_is_rejected_1005() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        client
            .reject(&cancel(&cid(9)), 1005, "order not found")
            .await;
        gateway.risk.assert_silent();
    });
}

// cancel of a well-formed order id that is not pending -> E[1005], nothing forwarded
#[test]
fn cancel_of_unknown_order_id_is_rejected_1005() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let unknown = "ab".repeat(16);
        client
            .reject(&cancel(&unknown), 1005, "order not found")
            .await;
        gateway.risk.assert_silent();
    });
}

// 32-character key that is not hex -> E[1005] invalid order id, nothing forwarded
#[test]
fn cancel_with_non_hex_order_id_is_rejected_1005() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let key = "z".repeat(32);
        client.reject(&cancel(&key), 1005, "invalid order id").await;
        gateway.risk.assert_silent();
    });
}

// 33-character cancel key -> E[1002] parse error, nothing forwarded
#[test]
fn cancel_key_of_neither_length_is_rejected_1002() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.send_text(&cancel(&"a".repeat(33))).await;
        let reply = client.recv().await;
        assert_eq!(error_code(&reply), 1002, "got {reply}");
        gateway.risk.assert_silent();
    });
}

// order completed by ORDER_DONE -> its pending entry is gone, a later cancel gets E[1005]
#[test]
fn done_order_is_no_longer_cancellable() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;
        gateway.risk.inject(done(ids(&placed), 10, 0, 0)).await;
        client.settle().await;
        assert_eq!(gateway.state.borrow().pending.len(), 0);

        client
            .reject(&cancel(&order_hex(&placed)), 1005, "order not found")
            .await;
        gateway.risk.assert_silent();
    });
}

// order rejected by risk (ORDER_FAILED) -> its pending entry is gone, a later cancel gets E[1005]
#[test]
fn failed_order_is_no_longer_cancellable() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;
        gateway.risk.inject(failed(ids(&placed), 2)).await;
        client.settle().await;
        assert_eq!(gateway.state.borrow().pending.len(), 0);

        client
            .reject(&cancel(&cid(1)), 1005, "order not found")
            .await;
        gateway.risk.assert_silent();
    });
}

// user 7 and user 8 place the same cid; user 7 cancels it -> the cancel carries user 7's order id
#[test]
fn cancel_by_client_order_id_finds_the_requesters_order() {
    run(Options::default(), |gateway| async move {
        let mut other = gateway.connect(8).await;
        let mut client = gateway.connect(7).await;
        gateway.place(&mut other, &order(&cid(1))).await;
        let own = gateway.place(&mut client, &order(&cid(1))).await;

        client.send_text(&cancel(&cid(1))).await;
        assert_eq!(client.settle().await, Vec::<String>::new());

        let cancels = gateway.risk.take_cancels();
        assert_eq!(cancels.len(), 1);
        assert_eq!(cancels[0].user_id, 7);
        assert_eq!(
            (cancels[0].order_id_hi, cancels[0].order_id_lo),
            (own.order_id_hi, own.order_id_lo)
        );
    });
}

// user 8 cancels user 7's order id -> E[1005], nothing forwarded
#[test]
fn cancel_of_another_users_order_id_is_rejected() {
    run(Options::default(), |gateway| async move {
        let mut owner = gateway.connect(7).await;
        let mut intruder = gateway.connect(8).await;
        let placed = gateway.place(&mut owner, &order(&cid(1))).await;

        intruder
            .reject(&cancel(&order_hex(&placed)), 1005, "order not found")
            .await;
        gateway.risk.assert_silent();
    });
}

// order placed with a 15-byte cid, cancelled by that cid -> forwarded
#[test]
fn cancel_by_short_client_order_id_is_forwarded() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let short = "short-cid-15byt";
        assert_eq!(short.len(), 15);
        gateway.place(&mut client, &order(short)).await;

        client.send_text(&cancel(short)).await;
        assert_eq!(client.settle().await, Vec::<String>::new());
        assert_eq!(gateway.risk.take_cancels().len(), 1);
    });
}

// pending order whose cancel cannot be sent -> E[8,"network error"]
#[test]
#[ignore = "BUGS.md GW-OVERLOADED-CODE"]
fn known_defect_failed_cancel_forward_answers_network_error() {
    let options = Options {
        risk_reachable: false,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        let pending = PendingOrder {
            order_id: [3u8; 16],
            user_id: 7,
            symbol_id: 0,
            client_order_id: cid(1).as_bytes().try_into().unwrap(),
            timestamp_ns: 1,
            stage: PendingStage::Submitted,
        };
        assert!(gateway.state.borrow_mut().pending.push(pending));
        client.reject(&cancel(&cid(1)), 8, "network error").await;
    });
}
