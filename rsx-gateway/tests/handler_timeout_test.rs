//! The pending-order timeout: an order risk has not answered by the cutoff
//! fails with a TIMEOUT `U` to its owner; an order risk has inserted into the
//! book is not timed out and stays cancellable. The binary's housekeeping loop
//! calls `expire_stale_orders` with a cutoff; the tests pass the cutoff
//! directly.

mod common;

use common::cancel;
use common::cid;
use common::done;
use common::ids;
use common::inserted;
use common::order;
use common::order_hex;
use common::run;
use common::Options;
use rsx_gateway::route::expire_stale_orders;

fn timeout_frame(order_hex: &str) -> String {
    format!(r#"{{"U":["{order_hex}",3,0,0,10,0,0,0,0,0]}}"#)
}

// order risk has not answered, older than the cutoff -> owner gets one U status 3 reason 10, entry gone
#[test]
fn unanswered_order_past_the_cutoff_gets_a_timeout_update() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;

        expire_stale_orders(&gateway.state, placed.timestamp_ns + 1);

        assert_eq!(client.settle().await, [timeout_frame(&order_hex(&placed))]);
        assert!(gateway.state.borrow().pending.is_empty());
    });
}

// order younger than the cutoff -> nothing sent, entry kept
#[test]
fn order_younger_than_the_cutoff_is_kept() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;

        expire_stale_orders(&gateway.state, placed.timestamp_ns);

        assert_eq!(client.settle().await, Vec::<String>::new());
        assert_eq!(gateway.state.borrow().pending.len(), 1);
    });
}

// two users each with an unanswered order -> each gets only their own timeout update
#[test]
fn timeout_update_goes_to_the_orders_owner_only() {
    run(Options::default(), |gateway| async move {
        let mut first = gateway.connect(7).await;
        let mut second = gateway.connect(8).await;
        let first_order = gateway.place(&mut first, &order(&cid(1))).await;
        let second_order = gateway.place(&mut second, &order(&cid(2))).await;

        expire_stale_orders(&gateway.state, second_order.timestamp_ns + 1);

        assert_eq!(
            first.settle().await,
            [timeout_frame(&order_hex(&first_order))]
        );
        assert_eq!(
            second.settle().await,
            [timeout_frame(&order_hex(&second_order))]
        );
    });
}

// one order inserted, one unanswered, both past the cutoff -> only the unanswered one times out
#[test]
fn inserted_order_is_not_timed_out_beside_an_unanswered_one() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let resting = gateway.place(&mut client, &order(&cid(1))).await;
        let unanswered = gateway.place(&mut client, &order(&cid(2))).await;
        gateway.risk.inject(inserted(ids(&resting), 10)).await;
        assert_eq!(client.settle().await.len(), 1);

        expire_stale_orders(&gateway.state, unanswered.timestamp_ns + 1);

        assert_eq!(
            client.settle().await,
            [timeout_frame(&order_hex(&unanswered))]
        );
        assert_eq!(gateway.state.borrow().pending.len(), 1);
    });
}

// order inserted and then swept past the cutoff -> a cancel by order id and by client order id still reaches risk
#[test]
fn inserted_order_stays_cancellable_after_the_sweep() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;
        gateway.risk.inject(inserted(ids(&placed), 10)).await;
        client.settle().await;

        expire_stale_orders(&gateway.state, placed.timestamp_ns + 1);
        assert_eq!(client.settle().await, Vec::<String>::new());

        client.send_text(&cancel(&order_hex(&placed))).await;
        client.send_text(&cancel(&cid(1))).await;
        assert_eq!(client.settle().await, Vec::<String>::new());
        assert_eq!(gateway.risk.take_cancels().len(), 2);
    });
}

// inserted order that then completes -> the entry is removed, so a later sweep sends nothing
#[test]
fn completed_resting_order_leaves_nothing_to_time_out() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;
        gateway.risk.inject(inserted(ids(&placed), 10)).await;
        gateway.risk.inject(done(ids(&placed), 10, 0, 0)).await;
        assert_eq!(client.settle().await.len(), 2);

        expire_stale_orders(&gateway.state, placed.timestamp_ns + 1);

        assert_eq!(client.settle().await, Vec::<String>::new());
        assert!(gateway.state.borrow().pending.is_empty());
    });
}

// timed-out order, then a cancel for it -> E[1005] order not found, nothing forwarded
#[test]
fn cancel_of_a_timed_out_order_is_rejected_1005() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;
        expire_stale_orders(&gateway.state, placed.timestamp_ns + 1);
        assert_eq!(client.settle().await.len(), 1);

        client
            .reject(&cancel(&order_hex(&placed)), 1005, "order not found")
            .await;
        gateway.risk.assert_silent();
    });
}
