//! The counters behind `/metrics`, written where the handler and the routes
//! decide: an order forwarded to risk counts in `orders_processed`; every
//! order refused or failed counts once under its `FailureReason`.

mod common;

use common::cid;
use common::failed;
use common::ids;
use common::order;
use common::run;
use common::Options;
use rsx_gateway::route::expire_stale_orders;
use rsx_types::FailureReason;

// one accepted order -> orders_processed 1, no rejects
#[test]
fn forwarded_order_counts_as_processed() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        gateway.place(&mut client, &order(&cid(1))).await;

        let state = gateway.state.borrow();
        assert_eq!(state.metrics.orders_processed.get(), 1);
        assert_eq!(state.metrics.rejects_total(), 0);
    });
}

// per-user limit 1 -> the second order is a RATE_LIMIT reject and is not processed
#[test]
fn rate_limited_order_counts_as_rate_limit() {
    let options = Options {
        rate_limit_per_user: 1,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.send_text(&order(&cid(1))).await;
        client.reject(&order(&cid(2)), 1006, "rate limited").await;

        let state = gateway.state.borrow();
        assert_eq!(state.metrics.orders_processed.get(), 1);
        assert_eq!(state.metrics.rejects(FailureReason::RateLimit), 1);
        assert_eq!(state.metrics.rejects_total(), 1);
    });
}

// unknown symbol, price off the tick, qty off the lot -> one reject each under its own reason
#[test]
fn validation_rejects_count_by_reason() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let unknown_symbol = format!(r#"{{"N":[9,0,1000,10,"{}",0]}}"#, cid(1));
        client.reject(&unknown_symbol, 1007, "unknown symbol").await;
        // symbol 0 has tick 10 and lot 5
        let off_tick = format!(r#"{{"N":[0,0,1001,10,"{}",0]}}"#, cid(2));
        client
            .reject(&off_tick, 1008, "price not tick aligned")
            .await;
        let off_lot = format!(r#"{{"N":[0,0,1000,11,"{}",0]}}"#, cid(3));
        client.reject(&off_lot, 1009, "qty not lot aligned").await;

        let state = gateway.state.borrow();
        assert_eq!(state.metrics.rejects(FailureReason::SymbolNotFound), 1);
        assert_eq!(state.metrics.rejects(FailureReason::InvalidTickSize), 1);
        assert_eq!(state.metrics.rejects(FailureReason::InvalidLotSize), 1);
        assert_eq!(state.metrics.rejects_total(), 3);
        assert_eq!(state.metrics.orders_processed.get(), 0);
    });
}

// full pending set, open circuit and spent instance limit -> three OVERLOADED rejects
#[test]
fn overload_rejects_count_as_overloaded() {
    let options = Options {
        max_pending: 1,
        circuit_threshold: 1,
        rate_limit_per_instance: 3,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.send_text(&order(&cid(1))).await;
        client.reject(&order(&cid(2)), 5, "overloaded").await;
        gateway.state.borrow_mut().pending.time_out(u64::MAX);
        gateway.state.borrow_mut().circuit.record_failure();
        client.reject(&order(&cid(3)), 5, "overloaded").await;
        client.reject(&order(&cid(4)), 5, "overloaded").await;

        let state = gateway.state.borrow();
        assert_eq!(state.metrics.rejects(FailureReason::Overloaded), 3);
        assert_eq!(state.metrics.rejects_total(), 3);
        assert_eq!(state.metrics.orders_processed.get(), 1);
    });
}

// risk unreachable -> the failed forward is a NETWORK_ERROR reject and the order is not processed
#[test]
fn failed_forward_counts_as_network_error() {
    let options = Options {
        risk_reachable: false,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.reject(&order(&cid(1)), 8, "network error").await;

        let state = gateway.state.borrow();
        assert_eq!(state.metrics.rejects(FailureReason::NetworkError), 1);
        assert_eq!(state.metrics.rejects_total(), 1);
        assert_eq!(state.metrics.orders_processed.get(), 0);
    });
}

// ORDER_FAILED from risk with reasons 4 and 13 -> counted under each, a repeat counts again
#[test]
fn order_failed_from_risk_counts_by_its_reason() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let first = gateway.place(&mut client, &order(&cid(1))).await;
        let second = gateway.place(&mut client, &order(&cid(2))).await;
        gateway
            .risk
            .inject(failed(ids(&first), FailureReason::InsufficientMargin as u8))
            .await;
        gateway
            .risk
            .inject(failed(ids(&second), FailureReason::FillOrKill as u8))
            .await;
        gateway
            .risk
            .inject(failed(ids(&second), FailureReason::FillOrKill as u8))
            .await;

        let state = gateway.state.borrow();
        assert_eq!(state.metrics.rejects(FailureReason::InsufficientMargin), 1);
        assert_eq!(state.metrics.rejects(FailureReason::FillOrKill), 2);
        assert_eq!(state.metrics.rejects_total(), 3);
    });
}

// ORDER_FAILED from risk with reason 14 -> counted under ORDER_TOO_SMALL, not rejects_other
#[test]
fn order_failed_order_too_small_counts_under_its_own_reason() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let first = gateway.place(&mut client, &order(&cid(1))).await;
        gateway
            .risk
            .inject(failed(ids(&first), FailureReason::OrderTooSmall as u8))
            .await;

        let state = gateway.state.borrow();
        assert_eq!(state.metrics.rejects(FailureReason::OrderTooSmall), 1);
        assert_eq!(state.metrics.rejects_total(), 1);
    });
}

// order unanswered past the cutoff -> the sweep counts one TIMEOUT reject
#[test]
fn timed_out_order_counts_as_timeout() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;

        expire_stale_orders(&gateway.state, placed.timestamp_ns + 1);

        let state = gateway.state.borrow();
        assert_eq!(state.metrics.rejects(FailureReason::Timeout), 1);
        assert_eq!(state.metrics.rejects_total(), 1);
    });
}
