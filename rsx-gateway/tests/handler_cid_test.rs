//! Client order ids: inside the 1 hour window a repeated `(user_id, cid)`
//! maps to the order id already minted, is answered with the `U` the ME gives
//! a duplicate order id and is never forwarded to risk. The tests move the
//! window with the `now_ns` that the housekeeping sweep passes to
//! `ClientOrderIds::expire`.

mod common;

use common::cid;
use common::done;
use common::ids;
use common::order;
use common::order_hex;
use common::run;
use common::Options;
use rsx_gateway::client_ids::WINDOW_NS;
use rsx_types::FailureReason;

/// `U` status 3 (failed), reason 3 (DUPLICATE_ORDER_ID), naming `order_hex`.
fn duplicate_frame(order_hex: &str) -> String {
    format!(r#"{{"U":["{order_hex}",3,0,0,3,0,0,0,0,0]}}"#)
}

// same cid twice from one user -> one order reaches risk, the second frame is answered with the first order's id
#[test]
fn repeated_cid_is_answered_as_duplicate_and_not_forwarded() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let first = gateway.place(&mut client, &order(&cid(1))).await;

        client.send_text(&order(&cid(1))).await;
        assert_eq!(client.recv().await, duplicate_frame(&order_hex(&first)));
        assert_eq!(client.settle().await, Vec::<String>::new());

        gateway.risk.assert_silent();
        let state = gateway.state.borrow();
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.client_ids.len(), 1);
        assert_eq!(state.metrics.rejects(FailureReason::DuplicateOrderId), 1);
        assert_eq!(state.metrics.orders_processed.get(), 1);
    });
}

// retry on a second connection of the same user -> the same duplicate answer, and the first order stays cancellable by cid
#[test]
fn retry_on_another_connection_names_the_first_order() {
    run(Options::default(), |gateway| async move {
        let mut original = gateway.connect(7).await;
        let first = gateway.place(&mut original, &order(&cid(1))).await;

        let mut reconnected = gateway.connect(7).await;
        reconnected.send_text(&order(&cid(1))).await;
        assert_eq!(
            reconnected.recv().await,
            duplicate_frame(&order_hex(&first))
        );
        assert_eq!(reconnected.settle().await, Vec::<String>::new());
        gateway.risk.assert_silent();

        reconnected.send_text(&common::cancel(&cid(1))).await;
        assert_eq!(reconnected.settle().await, Vec::<String>::new());
        assert_eq!(gateway.risk.take_cancels().len(), 1);
    });
}

// order already finished at the ME, same cid again -> still the duplicate answer, nothing forwarded
#[test]
fn repeated_cid_after_the_order_completed_is_still_a_duplicate() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let first = gateway.place(&mut client, &order(&cid(1))).await;
        gateway.risk.inject(done(ids(&first), 10, 0, 0)).await;
        client.settle().await;
        assert!(gateway.state.borrow().pending.is_empty());

        client.send_text(&order(&cid(1))).await;
        assert_eq!(client.recv().await, duplicate_frame(&order_hex(&first)));
        assert_eq!(client.settle().await, Vec::<String>::new());
        gateway.risk.assert_silent();
    });
}

// same cid after the window -> a new order with a new id; one tick inside the window it is still a duplicate
#[test]
fn repeated_cid_after_the_window_is_a_new_order() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let first = gateway.place(&mut client, &order(&cid(1))).await;

        client.send_text(&order(&cid(1))).await;
        assert_eq!(client.recv().await, duplicate_frame(&order_hex(&first)));

        let edge = first.timestamp_ns + WINDOW_NS;
        gateway.state.borrow_mut().client_ids.expire(edge);
        assert_eq!(gateway.state.borrow().client_ids.len(), 1);
        client.send_text(&order(&cid(1))).await;
        assert_eq!(client.recv().await, duplicate_frame(&order_hex(&first)));

        gateway.state.borrow_mut().client_ids.expire(edge + 1);
        assert_eq!(gateway.state.borrow().client_ids.len(), 0);
        let second = gateway.place(&mut client, &order(&cid(1))).await;
        assert_ne!(
            (second.order_id_hi, second.order_id_lo),
            (first.order_id_hi, first.order_id_lo)
        );
        assert_eq!(gateway.state.borrow().client_ids.len(), 1);
    });
}

// user 7 and user 8 send the same cid -> two independent orders; user 7's repeat is still a duplicate
#[test]
fn the_same_cid_from_another_user_is_independent() {
    run(Options::default(), |gateway| async move {
        let mut first_user = gateway.connect(7).await;
        let mut second_user = gateway.connect(8).await;
        let first = gateway.place(&mut first_user, &order(&cid(1))).await;
        let second = gateway.place(&mut second_user, &order(&cid(1))).await;
        assert_ne!(
            (first.order_id_hi, first.order_id_lo),
            (second.order_id_hi, second.order_id_lo)
        );

        first_user.send_text(&order(&cid(1))).await;
        assert_eq!(first_user.recv().await, duplicate_frame(&order_hex(&first)));
        second_user.send_text(&order(&cid(1))).await;
        assert_eq!(
            second_user.recv().await,
            duplicate_frame(&order_hex(&second))
        );
        gateway.risk.assert_silent();
    });
}

// forward to risk fails -> E[8] and the cid is not reserved
#[test]
fn a_failed_forward_does_not_reserve_the_cid() {
    let options = Options {
        risk_reachable: false,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.reject(&order(&cid(1)), 8, "network error").await;
        assert_eq!(gateway.state.borrow().client_ids.len(), 0);
    });
}

// circuit open after the first order, same cid again -> the duplicate answer, not E[5]
#[test]
fn a_duplicate_is_answered_while_the_circuit_is_open() {
    let options = Options {
        circuit_threshold: 1,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        let first = gateway.place(&mut client, &order(&cid(1))).await;
        gateway.state.borrow_mut().circuit.record_failure();

        client.send_text(&order(&cid(1))).await;
        assert_eq!(client.recv().await, duplicate_frame(&order_hex(&first)));
        assert_eq!(client.settle().await, Vec::<String>::new());
    });
}
