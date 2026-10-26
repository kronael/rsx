//! Resting orders stay pending so they can be cancelled, but only `Submitted`
//! orders count against `max_pending`.

mod common;

use common::cancel;
use common::cid;
use common::done;
use common::ids;
use common::inserted;
use common::order;
use common::order_hex;
use common::run;
use common::Client;
use common::Gateway;
use common::Options;
use rsx_messages::OrderRequest;

/// Place `frame` as `client`, then have risk report it resting; returns the
/// request risk saw once the `U` status 1 reached the client.
async fn rest(gateway: &Gateway, client: &mut Client, frame: &str) -> OrderRequest {
    let placed = gateway.place(client, frame).await;
    gateway.risk.inject(inserted(ids(&placed), 10)).await;
    client.recv().await;
    placed
}

// cap 3, user 7 rests 3 orders -> user 8 and user 7 are still accepted, nothing answers E[5]
#[test]
fn resting_orders_do_not_lock_out_other_users() {
    let options = Options {
        max_pending: 3,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut hog = gateway.connect(7).await;
        let mut other = gateway.connect(8).await;
        for n in 1..=3 {
            rest(&gateway, &mut hog, &order(&cid(n))).await;
        }
        assert_eq!(gateway.state.borrow().pending.len(), 3);

        gateway.place(&mut other, &order(&cid(10))).await;
        gateway.place(&mut hog, &order(&cid(4))).await;
        assert_eq!(gateway.state.borrow().pending.len(), 5);
    });
}

// cap 2, two resting and two in flight -> a third user is E[5]; a resting order's terminal record
// frees no in-flight slot; an in-flight order turning resting frees one
#[test]
fn in_flight_cap_still_rejects_at_max_pending() {
    let options = Options {
        max_pending: 2,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut first = gateway.connect(7).await;
        let mut second = gateway.connect(8).await;
        let mut third = gateway.connect(9).await;
        let resting = rest(&gateway, &mut first, &order(&cid(1))).await;
        rest(&gateway, &mut first, &order(&cid(2))).await;
        let in_flight = gateway.place(&mut first, &order(&cid(3))).await;
        gateway.place(&mut second, &order(&cid(4))).await;

        third.reject(&order(&cid(5)), 5, "overloaded").await;
        gateway.risk.assert_silent();

        gateway.risk.inject(done(ids(&resting), 0, 10, 2)).await;
        first.recv().await;
        third.reject(&order(&cid(6)), 5, "overloaded").await;

        gateway.risk.inject(inserted(ids(&in_flight), 10)).await;
        first.recv().await;
        gateway.place(&mut third, &order(&cid(7))).await;
    });
}

// cap 1, one order resting and one in flight -> the resting order is still found by a cancel
#[test]
fn resting_order_past_the_cap_can_be_cancelled() {
    let options = Options {
        max_pending: 1,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        let resting = rest(&gateway, &mut client, &order(&cid(1))).await;
        gateway.place(&mut client, &order(&cid(2))).await;

        client.send_text(&cancel(&order_hex(&resting))).await;
        assert_eq!(client.settle().await, Vec::<String>::new());
        let cancels = gateway.risk.take_cancels();
        assert_eq!(cancels.len(), 1);
        assert_eq!(
            (cancels[0].order_id_hi, cancels[0].order_id_lo),
            (resting.order_id_hi, resting.order_id_lo)
        );
    });
}
