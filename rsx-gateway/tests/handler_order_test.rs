//! The connection handler's order path: an accepted order becomes one
//! `RECORD_ORDER_REQUEST` to risk and one pending entry; the rate limits, the
//! circuit breaker and the pending cap reject before anything is forwarded,
//! and each reject reaches the client as an `E` frame.

mod common;

use common::cid;
use common::error_code;
use common::error_frame;
use common::order;
use common::run;
use common::Options;
use rsx_gateway::circuit::State;

// buy-side limit on symbol 1 with every flag set -> one order record with those fields
#[test]
fn accepted_order_is_forwarded_to_risk_and_tracked() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let frame = format!(r#"{{"N":[1,1,1000,10,"{}",2,1,1]}}"#, cid(42));
        client.send_text(&frame).await;
        assert_eq!(client.settle().await, Vec::<String>::new());

        let forwarded = gateway.risk.take_order();
        assert_eq!(forwarded.seq, 1);
        assert_eq!(forwarded.user_id, 7);
        assert_eq!(forwarded.symbol_id, 1);
        assert_eq!(forwarded.price, 1000);
        assert_eq!(forwarded.qty, 10);
        assert_eq!(forwarded.side, 1);
        assert_eq!(forwarded.tif, 2);
        assert!(forwarded.reduce_only);
        assert!(forwarded.post_only);
        assert!(!forwarded.is_liquidation);
        assert!(forwarded.timestamp_ns > 0);
        assert_eq!((forwarded.order_id_hi >> 8) & 0xF0, 0x70, "UUIDv7");

        let state = gateway.state.borrow();
        assert_eq!(state.pending.len(), 1);
        let mut order_id = [0u8; 16];
        order_id[..8].copy_from_slice(&forwarded.order_id_hi.to_be_bytes());
        order_id[8..].copy_from_slice(&forwarded.order_id_lo.to_be_bytes());
        let tracked = state.pending.find_by_order_id(7, &order_id).unwrap();
        assert_eq!(tracked.user_id, 7);
        assert_eq!(tracked.symbol_id, 1);
        let expected_cid: [u8; 20] = cid(42).as_bytes().try_into().unwrap();
        assert_eq!(tracked.client_order_id, expected_cid);
    });
}

// two orders -> consecutive seq, distinct order ids, both tracked
#[test]
fn consecutive_orders_get_consecutive_seq_and_distinct_ids() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.send_text(&order(&cid(1))).await;
        client.send_text(&order(&cid(2))).await;
        assert_eq!(client.settle().await, Vec::<String>::new());

        let orders = gateway.risk.take_orders();
        assert_eq!(orders.len(), 2);
        assert_eq!((orders[0].seq, orders[1].seq), (1, 2));
        assert_ne!(
            (orders[0].order_id_hi, orders[0].order_id_lo),
            (orders[1].order_id_hi, orders[1].order_id_lo)
        );
        assert_eq!(gateway.state.borrow().pending.len(), 2);
    });
}

// per-user limit 3 -> orders 4 and 5 get E[1006], only 3 reach risk
#[test]
fn user_rate_limit_rejects_1006_and_does_not_forward() {
    let options = Options {
        rate_limit_per_user: 3,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        for n in 0..5 {
            client.send_text(&order(&cid(n))).await;
        }
        for _ in 0..2 {
            assert_eq!(client.recv().await, error_frame(1006, "rate limited"));
        }
        assert_eq!(client.settle().await, Vec::<String>::new());
        assert_eq!(gateway.risk.take_orders().len(), 3);
        assert_eq!(gateway.state.borrow().pending.len(), 3);
    });
}

// user 7 exhausts a per-user limit of 1 -> user 8 is not throttled
#[test]
fn user_rate_limit_is_per_user() {
    let options = Options {
        rate_limit_per_user: 1,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut first = gateway.connect(7).await;
        let mut second = gateway.connect(8).await;
        first.send_text(&order(&cid(1))).await;
        first.reject(&order(&cid(2)), 1006, "rate limited").await;
        second.send_text(&order(&cid(3))).await;
        assert_eq!(second.settle().await, Vec::<String>::new());
        let users: Vec<u32> = gateway
            .risk
            .take_orders()
            .iter()
            .map(|o| o.user_id)
            .collect();
        assert_eq!(users, [7, 8]);
    });
}

// per-IP limit 2 shared by two users on one address -> the third order gets E[1006]
#[test]
fn ip_rate_limit_is_shared_across_users_on_one_address() {
    let options = Options {
        rate_limit_per_ip: 2,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut first = gateway.connect(7).await;
        let mut second = gateway.connect(8).await;
        first.send_text(&order(&cid(1))).await;
        second.send_text(&order(&cid(2))).await;
        assert_eq!(second.settle().await, Vec::<String>::new());
        first.reject(&order(&cid(3)), 1006, "rate limited").await;
        assert_eq!(gateway.risk.take_orders().len(), 2);
    });
}

// circuit forced open -> E[5], nothing forwarded or tracked
#[test]
fn open_circuit_rejects_overloaded() {
    let options = Options {
        circuit_threshold: 1,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        gateway.state.borrow_mut().circuit.record_failure();
        client.reject(&order(&cid(1)), 5, "overloaded").await;
        gateway.risk.assert_silent();
        assert_eq!(gateway.state.borrow().pending.len(), 0);
    });
}

// symbol whose tick size is 0 -> the order is answered with an E frame and the handler survives
#[test]
#[ignore = "BUGS.md GW-ZERO-TICK-PANIC"]
fn known_defect_zero_tick_symbol_does_not_panic_the_handler() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        gateway.state.borrow_mut().symbol_configs[0].tick_size = 0;
        client.send_text(&order(&cid(1))).await;
        let reply = client.recv().await;
        assert!(error_code(&reply) > 0, "expected an E frame, got {reply}");
    });
}

// probe slot spent on an order rejected by validation -> a valid order after another cooldown is admitted
#[test]
#[ignore = "BUGS.md GW-CIRCUIT-PROBE-LEAK"]
fn known_defect_rejected_probe_does_not_wedge_the_circuit() {
    let options = Options {
        circuit_threshold: 1,
        circuit_cooldown_ms: 100,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        gateway.state.borrow_mut().circuit.record_failure();
        monoio::time::sleep(std::time::Duration::from_millis(150)).await;
        let unknown_symbol = format!(r#"{{"N":[2,0,1000,10,"{}",0]}}"#, cid(1));
        client.reject(&unknown_symbol, 1007, "unknown symbol").await;

        monoio::time::sleep(std::time::Duration::from_millis(150)).await;
        client.send_text(&order(&cid(2))).await;
        assert_eq!(client.settle().await, Vec::<String>::new());
        assert_eq!(gateway.risk.take_orders().len(), 1);
    });
}

// forced-open circuit past its cooldown -> one probe order goes through and closes it
#[test]
fn circuit_probe_after_cooldown_closes_the_circuit() {
    let options = Options {
        circuit_threshold: 1,
        circuit_cooldown_ms: 100,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        gateway.state.borrow_mut().circuit.record_failure();
        client.reject(&order(&cid(1)), 5, "overloaded").await;
        monoio::time::sleep(std::time::Duration::from_millis(150)).await;
        client.send_text(&order(&cid(2))).await;
        client.send_text(&order(&cid(3))).await;
        assert_eq!(client.settle().await, Vec::<String>::new());
        assert_eq!(gateway.state.borrow().circuit.state(), State::Closed);
        assert_eq!(gateway.risk.take_orders().len(), 2);
    });
}

// risk unreachable, threshold 2 -> two failed forwards answer E and open the circuit, the third gets E[5]
#[test]
fn forward_failures_open_the_circuit() {
    let options = Options {
        circuit_threshold: 2,
        risk_reachable: false,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        for n in 0..2 {
            client.send_text(&order(&cid(n))).await;
            let reply = client.recv().await;
            assert!(error_code(&reply) != 5, "circuit opened early: {reply}");
        }
        assert_eq!(gateway.state.borrow().circuit.state(), State::Open);
        assert_eq!(gateway.state.borrow().pending.len(), 0);
        client.reject(&order(&cid(3)), 5, "overloaded").await;
    });
}

// pending cap 1 -> the second order is answered with an E frame and not forwarded
#[test]
fn full_pending_queue_rejects_without_forwarding() {
    let options = Options {
        max_pending: 1,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.send_text(&order(&cid(1))).await;
        client.send_text(&order(&cid(2))).await;
        let reply = client.recv().await;
        assert!(error_code(&reply) > 0, "expected an E frame, got {reply}");
        assert_eq!(client.settle().await, Vec::<String>::new());
        assert_eq!(gateway.risk.take_orders().len(), 1);
        assert_eq!(gateway.state.borrow().pending.len(), 1);
    });
}

// pending cap 1 -> the second order is answered E[5,"overloaded"]
#[test]
#[ignore = "BUGS.md GW-OVERLOADED-CODE"]
fn known_defect_full_pending_queue_answers_overloaded() {
    let options = Options {
        max_pending: 1,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.send_text(&order(&cid(1))).await;
        client.reject(&order(&cid(2)), 5, "overloaded").await;
    });
}

// risk unreachable -> a failed forward is answered E[8,"network error"]
#[test]
#[ignore = "BUGS.md GW-OVERLOADED-CODE"]
fn known_defect_failed_forward_answers_network_error() {
    let options = Options {
        risk_reachable: false,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.reject(&order(&cid(1)), 8, "network error").await;
    });
}

// a burst of 1200 orders inside a second, user and IP limits lifted -> some get E[5]
#[test]
#[ignore = "BUGS.md GW-INSTANCE-RATE-LIMIT-MISSING"]
fn known_defect_instance_rate_limit_rejects_overloaded() {
    let options = Options {
        max_pending: 2000,
        rate_limit_per_user: 100_000,
        rate_limit_per_ip: 100_000,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        for n in 0..1200 {
            client.send_text(&order(&cid(n))).await;
        }
        let replies = client.settle().await;
        assert!(
            replies.contains(&error_frame(5, "overloaded")),
            "no order was shed; {} replies",
            replies.len()
        );
    });
}
