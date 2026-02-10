use rsx_risk::liquidation::LiquidationEngine;
use rsx_risk::liquidation::LiquidationStatus;

fn make_engine() -> LiquidationEngine {
    LiquidationEngine::new(
        1_000_000_000, // 1s
        10,            // base_slip_bps
        10,            // max_rounds
    )
}

// -- enqueue --

#[test]
fn enqueue_creates_active_state() {
    let mut e = make_engine();
    e.enqueue(1, 100, 0);
    assert_eq!(e.active.len(), 1);
    assert_eq!(e.active[0].user_id, 1);
    assert_eq!(e.active[0].symbol_id, 100);
    assert_eq!(e.active[0].round, 1);
    assert_eq!(
        e.active[0].status,
        LiquidationStatus::Active
    );
}

#[test]
fn enqueue_dedup_same_user_symbol() {
    let mut e = make_engine();
    e.enqueue(1, 100, 0);
    e.enqueue(1, 100, 1000);
    assert_eq!(e.active.len(), 1);
}

#[test]
fn enqueue_allows_different_symbols() {
    let mut e = make_engine();
    e.enqueue(1, 100, 0);
    e.enqueue(1, 200, 0);
    assert_eq!(e.active.len(), 2);
}

#[test]
fn enqueue_allows_different_users() {
    let mut e = make_engine();
    e.enqueue(1, 100, 0);
    e.enqueue(2, 100, 0);
    assert_eq!(e.active.len(), 2);
}

// -- maybe_process --

#[test]
fn maybe_process_immediate_first_order() {
    let mut e = make_engine();
    e.enqueue(1, 100, 0);
    let (orders, _losses) = e.maybe_process(
        0,
        &|_u, _s| 10,
        &|_s| 50000,
    );
    assert_eq!(orders.len(), 1);
}

#[test]
fn maybe_process_respects_delay() {
    let mut e = make_engine();
    e.enqueue(1, 100, 0);
    // First order at t=1s
    let (_, _) = e.maybe_process(
        1_000_000_000,
        &|_, _| 10,
        &|_| 50000,
    );
    // round is now 2, delay = 2 * 1s = 2s
    // At t=2s (1s after last), too early
    let (orders, _losses) = e.maybe_process(
        2_000_000_000,
        &|_, _| 10,
        &|_| 50000,
    );
    assert_eq!(orders.len(), 0);
}

#[test]
fn maybe_process_delay_elapsed() {
    let mut e = make_engine();
    e.enqueue(1, 100, 0);
    // First order at t=1s
    let (_, _) = e.maybe_process(
        1_000_000_000,
        &|_, _| 10,
        &|_| 50000,
    );
    // round=2, delay=2s, last_order_ns=1s
    // need now >= 1s + 2s = 3s
    let (orders, _losses) = e.maybe_process(
        3_000_000_000,
        &|_, _| 10,
        &|_| 50000,
    );
    assert_eq!(orders.len(), 1);
}

#[test]
fn maybe_process_escalates_slippage() {
    // slip = round^2 * base_slip_bps
    // round 1: 1*1*10=10, round 2: 4*10=40, round 3: 9*10=90
    let mut e = make_engine();
    e.enqueue(1, 100, 0);
    let mark = 100_000i64;

    // Round 1 (slip=10bps): sell price = 100000*(10000-10)
    // /10000 = 99900
    let (orders, _losses) =
        e.maybe_process(0, &|_, _| 10, &|_| mark);
    assert_eq!(orders[0].price, 99900);

    // Round 2 (slip=40bps): sell price = 100000*(10000-40)
    // /10000 = 99600
    let (orders, _losses) = e.maybe_process(
        10_000_000_000,
        &|_, _| 10,
        &|_| mark,
    );
    assert_eq!(orders[0].price, 99600);

    // Round 3 (slip=90bps): sell price = 100000*(10000-90)
    // /10000 = 99100
    let (orders, _losses) = e.maybe_process(
        30_000_000_000,
        &|_, _| 10,
        &|_| mark,
    );
    assert_eq!(orders[0].price, 99100);
}

#[test]
fn maybe_process_generates_reduce_only_order() {
    let mut e = make_engine();
    e.enqueue(1, 100, 0);
    let (orders, _losses) =
        e.maybe_process(0, &|_, _| 10, &|_| 50000);
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].user_id, 1);
    assert_eq!(orders[0].symbol_id, 100);
    assert!(orders[0].qty > 0);
}

#[test]
fn maybe_process_long_position_sells() {
    let mut e = make_engine();
    e.enqueue(1, 100, 0);
    let (orders, _losses) =
        e.maybe_process(0, &|_, _| 10, &|_| 50000);
    assert_eq!(orders[0].side, 1); // sell
}

#[test]
fn maybe_process_short_position_buys() {
    let mut e = make_engine();
    e.enqueue(1, 100, 0);
    let (orders, _losses) =
        e.maybe_process(0, &|_, _| -10, &|_| 50000);
    assert_eq!(orders[0].side, 0); // buy
}

#[test]
fn maybe_process_order_qty_equals_abs_position() {
    let mut e = make_engine();
    e.enqueue(1, 100, 0);
    let (orders, _losses) =
        e.maybe_process(0, &|_, _| -7, &|_| 50000);
    assert_eq!(orders[0].qty, 7);
}

#[test]
fn maybe_process_order_price_with_slippage() {
    let mut e = make_engine();
    let mark = 100_000i64;

    // Long -> sell: mark*(10000-slip)/10000
    // round 1, slip=10
    e.enqueue(1, 100, 0);
    let (orders, _losses) =
        e.maybe_process(0, &|_, _| 10, &|_| mark);
    assert_eq!(
        orders[0].price,
        mark * (10_000 - 10) / 10_000
    );

    // Short -> buy: mark*(10000+slip)/10000
    let mut e2 = make_engine();
    e2.enqueue(2, 100, 0);
    let (orders, _losses) =
        e2.maybe_process(0, &|_, _| -10, &|_| mark);
    assert_eq!(
        orders[0].price,
        mark * (10_000 + 10) / 10_000
    );
}

#[test]
fn maybe_process_marks_done_after_max_rounds() {
    let mut e = LiquidationEngine::new(
        0, // no delay
        10,
        3, // max 3 rounds
    );
    e.enqueue(1, 100, 0);

    // rounds 1,2,3 place orders; round 4 > max triggers Done
    for i in 0..4 {
        let (_, _) = e.maybe_process(
            i as u64,
            &|_, _| 10,
            &|_| 50000,
        );
    }
    assert_eq!(
        e.active[0].status,
        LiquidationStatus::Done
    );

    // No more orders
    let (orders, _losses) =
        e.maybe_process(100, &|_, _| 10, &|_| 50000);
    assert_eq!(orders.len(), 0);
}

// -- cancel / remove --

#[test]
fn cancel_if_recovered_removes_active() {
    let mut e = make_engine();
    e.enqueue(1, 100, 0);
    assert_eq!(e.active.len(), 1);
    e.cancel_if_recovered(1, 100);
    assert_eq!(e.active.len(), 0);
}

#[test]
fn cancel_if_recovered_noop_when_not_active() {
    let mut e = make_engine();
    e.cancel_if_recovered(1, 100); // no crash
    assert_eq!(e.active.len(), 0);
}

#[test]
fn remove_done_cleans_completed() {
    let mut e = LiquidationEngine::new(0, 10, 1);
    e.enqueue(1, 100, 0);
    // round 1 places order, round 2 > max_rounds triggers Done
    let (_, _) = e.maybe_process(0, &|_, _| 10, &|_| 50000);
    let (_, _) = e.maybe_process(1, &|_, _| 10, &|_| 50000);
    assert_eq!(
        e.active[0].status,
        LiquidationStatus::Done
    );
    e.remove_done();
    assert_eq!(e.active.len(), 0);
}

// -- edge cases --

#[test]
fn zero_position_no_order() {
    let mut e = make_engine();
    e.enqueue(1, 100, 0);
    let (orders, _losses) =
        e.maybe_process(0, &|_, _| 0, &|_| 50000);
    assert_eq!(orders.len(), 0);
    // Also marks done
    assert_eq!(
        e.active[0].status,
        LiquidationStatus::Done
    );
}

#[test]
fn multiple_users_independent_rounds() {
    let mut e = LiquidationEngine::new(
        1_000_000_000,
        10,
        10,
    );
    e.enqueue(1, 100, 0);
    e.enqueue(2, 100, 500_000_000); // enqueued later

    // Both fire first order (last_order_ns=0)
    let (orders, _losses) = e.maybe_process(
        500_000_000,
        &|u, _| if u == 1 { 10 } else { -5 },
        &|_| 50000,
    );
    assert_eq!(orders.len(), 2);

    // User 1 at round 2 (delay=2s from t=500ms)
    // User 2 at round 2 (delay=2s from t=500ms)
    // At t=1.5s, neither should fire
    let (orders, _losses) = e.maybe_process(
        1_500_000_000,
        &|u, _| if u == 1 { 10 } else { -5 },
        &|_| 50000,
    );
    assert_eq!(orders.len(), 0);

    // At t=2.5s (2s after 500ms), both fire
    let (orders, _losses) = e.maybe_process(
        2_500_000_000,
        &|u, _| if u == 1 { 10 } else { -5 },
        &|_| 50000,
    );
    assert_eq!(orders.len(), 2);
}
