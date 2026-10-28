//! The shutdown drain: `advance_shutdown` is what the binary's housekeeping
//! loop calls once the signal arrives. New orders are refused while it
//! drains, risk's answers still reach the client, and every session ends with
//! close 1001.

mod common;

use common::cid;
use common::done;
use common::ids;
use common::order;
use common::run;
use common::Options;
use rsx_gateway::state::Phase;

const DRAIN_NS: u64 = 1_000;
const CLOSE_1001: (u8, [u8; 2]) = (8, [0x03, 0xE9]);

// first call -> Draining; a new order is refused E[5], a cancel of a pending order and the order's own answer still flow
#[test]
fn draining_refuses_new_orders_and_still_routes_answers() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;

        assert!(!gateway.state.borrow_mut().advance_shutdown(0, DRAIN_NS));
        assert_eq!(gateway.state.borrow().phase, Phase::Draining);

        client.reject(&order(&cid(2)), 5, "overloaded").await;
        gateway.risk.assert_silent();

        gateway.risk.inject(done(ids(&placed), 10, 0, 1)).await;
        assert_eq!(client.settle().await.len(), 1);
    });
}

// Submitted order answered -> next step is Closing: the client gets close 1001 after its answer, the socket ends, the step after that may exit
#[test]
fn answered_orders_end_the_drain_and_every_session_gets_close_1001() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let mut idle = gateway.connect(8).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;
        gateway.state.borrow_mut().advance_shutdown(0, DRAIN_NS);

        gateway.risk.inject(done(ids(&placed), 10, 0, 1)).await;
        gateway
            .wait_until("order answered", |gateway| {
                gateway.state.borrow().pending.in_flight() == 0
            })
            .await;
        assert!(!gateway.state.borrow_mut().advance_shutdown(1, DRAIN_NS));
        assert_eq!(gateway.state.borrow().phase, Phase::Closing);

        let answer = client.recv().await;
        assert!(answer.starts_with(r#"{"U":["#), "got {answer}");
        let (opcode, payload) = client.recv_frame().await;
        assert_eq!((opcode, [payload[0], payload[1]]), CLOSE_1001);
        assert!(client.read_to_close().await.is_empty());
        let (opcode, payload) = idle.recv_frame().await;
        assert_eq!((opcode, [payload[0], payload[1]]), CLOSE_1001);

        gateway
            .wait_until("sessions gone", |gateway| gateway.connection_count() == 0)
            .await;
        assert!(gateway.state.borrow_mut().advance_shutdown(2, DRAIN_NS));
    });
}

// order never answered -> Draining holds until the bound, then Closing
#[test]
fn unanswered_order_holds_the_drain_until_the_bound() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        gateway.place(&mut client, &order(&cid(1))).await;
        let mut state = gateway.state.borrow_mut();

        state.advance_shutdown(0, DRAIN_NS);
        state.advance_shutdown(DRAIN_NS - 1, DRAIN_NS);
        assert_eq!(state.phase, Phase::Draining);

        state.advance_shutdown(DRAIN_NS, DRAIN_NS);
        assert_eq!(state.phase, Phase::Closing);
    });
}

// Closing with a session that never ends -> exit is allowed once the grace passes
#[test]
fn closing_ends_after_the_grace_with_a_session_still_open() {
    run(Options::default(), |gateway| async move {
        let _client = gateway.connect(7).await;
        let mut state = gateway.state.borrow_mut();
        state.advance_shutdown(0, DRAIN_NS);
        state.advance_shutdown(0, DRAIN_NS);

        assert!(!state.advance_shutdown(1, DRAIN_NS));
        assert!(state.advance_shutdown(2_000_000_000, DRAIN_NS));
    });
}

// duplicate cid while draining -> still answered E[1011], not E[5]
#[test]
fn draining_still_answers_a_duplicate_cid() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        gateway.place(&mut client, &order(&cid(1))).await;
        gateway.state.borrow_mut().advance_shutdown(0, DRAIN_NS);

        client.send_text(&order(&cid(1))).await;

        let reply = client.recv().await;
        assert!(reply.starts_with(r#"{"E":[1011,"#), "got {reply}");
    });
}
