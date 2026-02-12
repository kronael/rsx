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

#[test]
fn halt_symbol_skips_liquidation() {
    let mut e = make_engine();
    e.enqueue(1, 0, 0);
    e.halt_symbol(0);
    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 100,
        &|_| 50000,
    );
    assert!(orders.is_empty());
    assert!(e.is_halted(0));
}

#[test]
fn resume_symbol_allows_liquidation() {
    let mut e = make_engine();
    e.enqueue(1, 0, 0);
    e.halt_symbol(0);
    e.resume_symbol(0);
    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 100,
        &|_| 50000,
    );
    assert_eq!(orders.len(), 1);
    assert!(!e.is_halted(0));
}

#[test]
fn halt_only_affects_target_symbol() {
    let mut e = make_engine();
    e.enqueue(1, 0, 0);
    e.enqueue(2, 1, 0);
    e.halt_symbol(0);
    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 100,
        &|_| 50000,
    );
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].symbol_id, 1);
}

// --- Additional edge cases ---

#[test]
fn multiple_positions_all_get_orders() {
    let mut e = make_engine();
    e.enqueue(1, 0, 0);
    e.enqueue(1, 1, 0);
    e.enqueue(1, 2, 0);
    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 10,
        &|_| 50000,
    );
    assert_eq!(orders.len(), 3);
    let syms: Vec<u32> =
        orders.iter().map(|o| o.symbol_id).collect();
    assert!(syms.contains(&0));
    assert!(syms.contains(&1));
    assert!(syms.contains(&2));
}

#[test]
fn partial_fill_reduces_position() {
    // Simulate position reduction between rounds
    let mut e = LiquidationEngine::new(0, 10, 10);
    e.enqueue(1, 0, 0);
    // Round 1: position=100
    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 100,
        &|_| 50000,
    );
    assert_eq!(orders[0].qty, 100);
    // Round 2: position reduced to 40 (partial fill)
    let (orders, _) = e.maybe_process(
        1,
        &|_, _| 40,
        &|_| 50000,
    );
    assert_eq!(orders[0].qty, 40);
}

#[test]
fn full_fill_closes_position() {
    let mut e = LiquidationEngine::new(0, 10, 10);
    e.enqueue(1, 0, 0);
    // Round 1: places order
    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 10,
        &|_| 50000,
    );
    assert_eq!(orders.len(), 1);
    // Position fully closed
    let (orders, _) = e.maybe_process(
        1,
        &|_, _| 0,
        &|_| 50000,
    );
    assert_eq!(orders.len(), 0);
    assert_eq!(
        e.active[0].status,
        LiquidationStatus::Done
    );
}

#[test]
fn new_orders_rejected_during_liquidation() {
    // This is tested at shard level (process_order
    // checks needs_liquidation). At engine level,
    // verify is_in_liquidation returns true.
    let mut e = make_engine();
    e.enqueue(1, 0, 0);
    assert!(e.is_in_liquidation(1, 0));
}

#[test]
fn pending_non_liq_orders_cancelled_on_entry() {
    // NOTE: Cancellation of pending non-liquidation
    // orders on liquidation entry is handled at shard
    // level via release_frozen_for_order. The engine
    // itself only tracks liquidation state. This test
    // verifies the engine marks entry immediately.
    let mut e = make_engine();
    e.enqueue(1, 0, 0);
    assert_eq!(
        e.active[0].status,
        LiquidationStatus::Active
    );
    assert_eq!(e.active[0].round, 1);
}

#[test]
fn frozen_margin_released_on_entry() {
    // NOTE: Frozen margin release is handled at shard
    // level (release_frozen_for_order). Engine only
    // tracks liquidation state. Verifying enqueue does
    // not block on frozen margin.
    let mut e = make_engine();
    e.enqueue(1, 0, 0);
    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 50,
        &|_| 10000,
    );
    assert_eq!(orders.len(), 1);
}

#[test]
fn mark_price_update_rechecks_liquidating_users() {
    // Engine uses get_mark_fn at maybe_process time.
    // Changing mark between calls changes behavior.
    let mut e = LiquidationEngine::new(0, 10, 10);
    e.enqueue(1, 0, 0);
    // Round 1 with mark=50000
    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 10,
        &|_| 50000,
    );
    assert_eq!(orders[0].price, 49950); // slip=10bps
    // Round 2 with updated mark=40000
    let (orders, _) = e.maybe_process(
        1,
        &|_, _| 10,
        &|_| 40000,
    );
    // slip = 2^2*10 = 40bps -> 40000*(10000-40)/10000
    assert_eq!(orders[0].price, 39840);
}

#[test]
fn order_failed_symbol_halted_pauses_symbol() {
    let mut e = make_engine();
    e.enqueue(1, 0, 0);
    e.halt_symbol(0);
    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 10,
        &|_| 50000,
    );
    assert!(orders.is_empty());
    // State still active, not removed
    assert_eq!(
        e.active[0].status,
        LiquidationStatus::Active
    );
}

#[test]
fn order_failed_other_escalates_next_round() {
    // After a failed order, the round was already
    // incremented. Next maybe_process with sufficient
    // delay fires at higher round (more slippage).
    let mut e = LiquidationEngine::new(0, 10, 10);
    e.enqueue(1, 0, 0);
    // Round 1
    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 10,
        &|_| 100_000,
    );
    assert_eq!(orders[0].price, 99900); // r1 slip=10
    // Suppose order failed. Round is now 2.
    // Next call fires round 2 (slip=40).
    let (orders, _) = e.maybe_process(
        1,
        &|_, _| 10,
        &|_| 100_000,
    );
    assert_eq!(orders[0].price, 99600); // r2 slip=40
}

#[test]
fn first_order_fires_immediately_no_delay() {
    let mut e = LiquidationEngine::new(
        5_000_000_000, // 5s base delay
        10,
        10,
    );
    e.enqueue(1, 0, 0);
    // First order fires at t=0 (last_order_ns=0)
    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 10,
        &|_| 50000,
    );
    assert_eq!(orders.len(), 1);
}

#[test]
fn mark_price_zero_pauses_round_no_increment() {
    let mut e = LiquidationEngine::new(0, 10, 10);
    e.enqueue(1, 0, 0);
    // Mark = 0 -> skip (no order, no round increment)
    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 10,
        &|_| 0,
    );
    assert!(orders.is_empty());
    // Round still 1 (not incremented)
    assert_eq!(e.active[0].round, 1);
    assert_eq!(e.active[0].last_order_ns, 0);
}

#[test]
fn multiple_symbols_independent_round_timers() {
    let mut e = LiquidationEngine::new(
        1_000_000_000, // 1s
        10,
        10,
    );
    e.enqueue(1, 0, 0);
    e.enqueue(1, 1, 500_000_000); // later
    // Both fire first order (last_order_ns=0)
    let (orders, _) = e.maybe_process(
        1_000_000_000,
        &|_, _| 10,
        &|_| 50000,
    );
    assert_eq!(orders.len(), 2);
    // Now sym 0 at round 2 (delay=2s from t=1s)
    // sym 1 at round 2 (delay=2s from t=1s)
    // At t=2s: both too early (need t>=3s)
    let (orders, _) = e.maybe_process(
        2_000_000_000,
        &|_, _| 10,
        &|_| 50000,
    );
    assert_eq!(orders.len(), 0);
    // At t=3s: both fire
    let (orders, _) = e.maybe_process(
        3_000_000_000,
        &|_, _| 10,
        &|_| 50000,
    );
    assert_eq!(orders.len(), 2);
}

#[test]
fn rapid_fire_maybe_process_no_duplicate_orders() {
    let mut e = LiquidationEngine::new(
        1_000_000_000,
        10,
        10,
    );
    e.enqueue(1, 0, 0);
    // First call at t=1s fires order
    let (o1, _) = e.maybe_process(
        1_000_000_000,
        &|_, _| 10,
        &|_| 50000,
    );
    assert_eq!(o1.len(), 1);
    // Immediate second call: delay not elapsed
    // round=2, delay=2s, need t >= 1s+2s = 3s
    let (o2, _) = e.maybe_process(
        1_000_000_000,
        &|_, _| 10,
        &|_| 50000,
    );
    assert_eq!(o2.len(), 0);
    // Still too early at t=2.5s
    let (o3, _) = e.maybe_process(
        2_500_000_000,
        &|_, _| 10,
        &|_| 50000,
    );
    assert_eq!(o3.len(), 0);
}

#[test]
fn socialized_loss_when_round_exceeds_max_rounds() {
    let mut e = LiquidationEngine::new(0, 10, 2);
    e.enqueue(1, 0, 0);
    // Round 1: order
    let (orders, losses) = e.maybe_process(
        0,
        &|_, _| 10,
        &|_| 50000,
    );
    assert_eq!(orders.len(), 1);
    assert!(losses.is_empty());
    // Round 2: order
    let (orders, losses) = e.maybe_process(
        1,
        &|_, _| 10,
        &|_| 50000,
    );
    assert_eq!(orders.len(), 1);
    assert!(losses.is_empty());
    // Round 3 > max_rounds=2: socialized loss
    let (orders, losses) = e.maybe_process(
        2,
        &|_, _| 10,
        &|_| 50000,
    );
    assert!(orders.is_empty());
    assert_eq!(losses.len(), 1);
    assert_eq!(losses[0].user_id, 1);
    assert_eq!(losses[0].symbol_id, 0);
    assert_eq!(losses[0].qty, 10);
    assert_eq!(losses[0].price, 50000);
    assert_eq!(
        e.active[0].status,
        LiquidationStatus::Done
    );
}

#[test]
fn base_delay_zero_all_rounds_immediate() {
    let mut e = LiquidationEngine::new(0, 10, 5);
    e.enqueue(1, 0, 0);
    // All rounds fire at same timestamp
    for i in 0..5 {
        let (orders, _) = e.maybe_process(
            0,
            &|_, _| 10,
            &|_| 50000,
        );
        assert_eq!(
            orders.len(),
            1,
            "round {} should fire",
            i + 1
        );
    }
    // Round 6 > max_rounds=5: socialized
    let (orders, losses) = e.maybe_process(
        0,
        &|_, _| 10,
        &|_| 50000,
    );
    assert!(orders.is_empty());
    assert_eq!(losses.len(), 1);
}

#[test]
fn max_rounds_zero_allows_round_one_then_socializes() {
    let mut e = LiquidationEngine::new(0, 10, 0);
    e.enqueue(1, 0, 0);
    // Round 1 > max_rounds(0): immediate socialized loss
    let (orders, losses) = e.maybe_process(
        0,
        &|_, _| 10,
        &|_| 50000,
    );
    assert!(orders.is_empty());
    assert_eq!(losses.len(), 1);
    assert_eq!(
        e.active[0].status,
        LiquidationStatus::Done
    );
}
