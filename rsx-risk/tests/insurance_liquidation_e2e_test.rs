//! E2E tests for insurance fund integration with liquidation engine.

use rsx_risk::insurance::InsuranceFund;
use rsx_risk::liquidation::LiquidationEngine;

fn make_engine_with_max_rounds(max_rounds: u32) -> LiquidationEngine {
    LiquidationEngine::new(
        0, // no delay
        10, max_rounds, 500, 0,
    )
}

// -- socialized loss generation --

#[test]
fn socialized_loss_emitted_after_max_rounds() {
    let mut e = make_engine_with_max_rounds(3);
    e.enqueue(1, 100, 0);

    // Rounds 1,2,3
    for i in 0..3 {
        let (_, losses) = e.maybe_process(i as u64, &|_, _| 10, &|_| 50_000);
        assert_eq!(losses.len(), 0);
    }

    // Round 4 exceeds max, emit socialized loss
    let (_, losses) = e.maybe_process(100, &|_, _| 10, &|_| 50_000);
    assert_eq!(losses.len(), 1);
    assert_eq!(losses[0].user_id, 1);
    assert_eq!(losses[0].symbol_id, 100);
    assert_eq!(losses[0].round, 4);
}

#[test]
fn socialized_loss_contains_position_details() {
    let mut e = make_engine_with_max_rounds(1);
    e.enqueue(1, 100, 0);

    let (_, _) = e.maybe_process(0, &|_, _| 10, &|_| 50_000);
    let (_, losses) = e.maybe_process(100, &|_, _| 10, &|_| 50_000);

    assert_eq!(losses[0].qty, 10);
    assert_eq!(losses[0].price, 50_000);
    assert_eq!(losses[0].side, 1); // sell for long
}

#[test]
fn socialized_loss_short_position() {
    let mut e = make_engine_with_max_rounds(1);
    e.enqueue(1, 100, 0);

    let (_, _) = e.maybe_process(0, &|_, _| -10, &|_| 50_000);
    let (_, losses) = e.maybe_process(100, &|_, _| -10, &|_| 50_000);

    assert_eq!(losses[0].qty, 10);
    assert_eq!(losses[0].side, 0); // buy for short
}

// -- recovery scenarios --

#[test]
fn no_socialized_loss_if_position_closes_before_max_rounds() {
    let mut e = make_engine_with_max_rounds(5);
    e.enqueue(1, 100, 0);

    // Round 1
    let (_, _) = e.maybe_process(0, &|_, _| 10, &|_| 50_000);

    // Position closes (filled)
    let (_, losses) = e.maybe_process(100, &|_, _| 0, &|_| 50_000);
    assert_eq!(losses.len(), 0);
}

#[test]
fn no_socialized_loss_if_margin_recovered() {
    let mut e = make_engine_with_max_rounds(5);
    e.enqueue(1, 100, 0);

    let (_, _) = e.maybe_process(0, &|_, _| 10, &|_| 50_000);

    e.cancel_if_recovered(1, 100);

    let (orders, losses) = e.maybe_process(100, &|_, _| 10, &|_| 50_000);
    assert_eq!(orders.len(), 0);
    assert_eq!(losses.len(), 0);
}

// -- version tracking --

#[test]
fn insurance_fund_version_tracks_updates() {
    let mut fund = InsuranceFund::new(100, 1_000_000);
    assert_eq!(fund.version, 0);

    fund.deduct(100_000);
    assert_eq!(fund.version, 1);

    fund.deduct(50_000);
    assert_eq!(fund.version, 2);

    fund.add(25_000);
    assert_eq!(fund.version, 3);
}

// -- fee collection --

#[test]
fn fund_recovery_via_fee_collection() {
    let mut fund = InsuranceFund::new(100, -100_000);

    fund.add(10_000);
    fund.add(20_000);
    fund.add(30_000);
    fund.add(40_000);

    assert_eq!(fund.balance, 0);
}
