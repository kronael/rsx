/// E2E tests for insurance fund integration with liquidation engine.

use rsx_risk::insurance::InsuranceFund;
use rsx_risk::liquidation::LiquidationEngine;
use rsx_risk::liquidation::SocializedLoss;
use rustc_hash::FxHashMap;

fn make_engine_with_max_rounds(max_rounds: u32) -> LiquidationEngine {
    LiquidationEngine::new(
        0, // no delay
        10,
        max_rounds,
    )
}

// -- socialized loss generation --

#[test]
fn socialized_loss_emitted_after_max_rounds() {
    let mut e = make_engine_with_max_rounds(3);
    e.enqueue(1, 100, 0);

    // Rounds 1,2,3
    for i in 0..3 {
        let (_, losses) = e.maybe_process(
            i as u64,
            &|_, _| 10,
            &|_| 50_000,
        );
        assert_eq!(losses.len(), 0);
    }

    // Round 4 exceeds max, emit socialized loss
    let (_, losses) =
        e.maybe_process(100, &|_, _| 10, &|_| 50_000);
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
    let (_, losses) =
        e.maybe_process(100, &|_, _| 10, &|_| 50_000);

    assert_eq!(losses[0].qty, 10);
    assert_eq!(losses[0].price, 50_000);
    assert_eq!(losses[0].side, 1); // sell for long
}

#[test]
fn socialized_loss_short_position() {
    let mut e = make_engine_with_max_rounds(1);
    e.enqueue(1, 100, 0);

    let (_, _) = e.maybe_process(0, &|_, _| -10, &|_| 50_000);
    let (_, losses) =
        e.maybe_process(100, &|_, _| -10, &|_| 50_000);

    assert_eq!(losses[0].qty, 10);
    assert_eq!(losses[0].side, 0); // buy for short
}

// -- insurance fund deduction simulation --

#[test]
fn insurance_fund_deduct_socialized_loss() {
    let mut fund = InsuranceFund::new(100, 1_000_000);
    let loss = SocializedLoss {
        user_id: 1,
        symbol_id: 100,
        round: 4,
        side: 1,
        price: 50_000,
        qty: 10,
        timestamp_ns: 0,
    };
    let loss_amount = loss.qty * loss.price;
    fund.deduct(loss_amount);
    assert_eq!(fund.balance, 1_000_000 - 500_000);
}

#[test]
fn insurance_fund_multiple_losses() {
    let mut fund = InsuranceFund::new(100, 10_000_000);

    let loss1 = SocializedLoss {
        user_id: 1,
        symbol_id: 100,
        round: 4,
        side: 1,
        price: 50_000,
        qty: 10,
        timestamp_ns: 0,
    };
    fund.deduct(loss1.qty * loss1.price);

    let loss2 = SocializedLoss {
        user_id: 2,
        symbol_id: 100,
        round: 5,
        side: 0,
        price: 51_000,
        qty: 5,
        timestamp_ns: 1000,
    };
    fund.deduct(loss2.qty * loss2.price);

    assert_eq!(fund.balance, 10_000_000 - 500_000 - 255_000);
}

#[test]
fn insurance_fund_depleted_goes_negative() {
    let mut fund = InsuranceFund::new(100, 100_000);
    let loss = SocializedLoss {
        user_id: 1,
        symbol_id: 100,
        round: 10,
        side: 1,
        price: 50_000,
        qty: 10,
        timestamp_ns: 0,
    };
    fund.deduct(loss.qty * loss.price);
    assert_eq!(fund.balance, -400_000);
}

// -- multiple symbols --

#[test]
fn independent_insurance_funds_per_symbol() {
    let mut funds: FxHashMap<u32, InsuranceFund> = FxHashMap::default();
    funds.insert(100, InsuranceFund::new(100, 1_000_000));
    funds.insert(200, InsuranceFund::new(200, 2_000_000));

    let loss = SocializedLoss {
        user_id: 1,
        symbol_id: 100,
        round: 4,
        side: 1,
        price: 50_000,
        qty: 10,
        timestamp_ns: 0,
    };

    funds.get_mut(&100).unwrap().deduct(loss.qty * loss.price);

    assert_eq!(funds.get(&100).unwrap().balance, 500_000);
    assert_eq!(funds.get(&200).unwrap().balance, 2_000_000);
}

#[test]
fn zero_initial_fund_created_on_first_loss() {
    let mut funds: FxHashMap<u32, InsuranceFund> = FxHashMap::default();

    let loss = SocializedLoss {
        user_id: 1,
        symbol_id: 100,
        round: 4,
        side: 1,
        price: 50_000,
        qty: 10,
        timestamp_ns: 0,
    };

    let fund = funds
        .entry(100)
        .or_insert_with(|| InsuranceFund::new(100, 0));
    fund.deduct(loss.qty * loss.price);

    assert_eq!(funds.get(&100).unwrap().balance, -500_000);
}

// -- recovery scenarios --

#[test]
fn no_socialized_loss_if_position_closes_before_max_rounds() {
    let mut e = make_engine_with_max_rounds(5);
    e.enqueue(1, 100, 0);

    // Round 1
    let (_, _) = e.maybe_process(0, &|_, _| 10, &|_| 50_000);

    // Position closes (filled)
    let (_, losses) =
        e.maybe_process(100, &|_, _| 0, &|_| 50_000);
    assert_eq!(losses.len(), 0);
}

#[test]
fn no_socialized_loss_if_margin_recovered() {
    let mut e = make_engine_with_max_rounds(5);
    e.enqueue(1, 100, 0);

    let (_, _) = e.maybe_process(0, &|_, _| 10, &|_| 50_000);

    e.cancel_if_recovered(1, 100);

    let (orders, losses) =
        e.maybe_process(100, &|_, _| 10, &|_| 50_000);
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

// -- edge cases --

#[test]
fn zero_qty_socialized_loss() {
    let mut fund = InsuranceFund::new(100, 100_000);
    let loss = SocializedLoss {
        user_id: 1,
        symbol_id: 100,
        round: 4,
        side: 1,
        price: 50_000,
        qty: 0,
        timestamp_ns: 0,
    };
    fund.deduct(loss.qty * loss.price);
    assert_eq!(fund.balance, 100_000);
}

#[test]
fn zero_price_socialized_loss() {
    let mut fund = InsuranceFund::new(100, 100_000);
    let loss = SocializedLoss {
        user_id: 1,
        symbol_id: 100,
        round: 4,
        side: 1,
        price: 0,
        qty: 10,
        timestamp_ns: 0,
    };
    fund.deduct(loss.qty * loss.price);
    assert_eq!(fund.balance, 100_000);
}

#[test]
fn large_socialized_loss() {
    let mut fund =
        InsuranceFund::new(100, 100_000_000_000);
    let loss = SocializedLoss {
        user_id: 1,
        symbol_id: 100,
        round: 50,
        side: 1,
        price: 100_000,
        qty: 1_000_000,
        timestamp_ns: 0,
    };
    fund.deduct(loss.qty * loss.price);
    assert_eq!(fund.balance, 0);
}

// -- concurrent liquidations --

#[test]
fn multiple_users_same_symbol_depletes_fund() {
    let mut funds: FxHashMap<u32, InsuranceFund> = FxHashMap::default();
    funds.insert(100, InsuranceFund::new(100, 200_000));

    let loss1 = SocializedLoss {
        user_id: 1,
        symbol_id: 100,
        round: 4,
        side: 1,
        price: 50_000,
        qty: 2,
        timestamp_ns: 0,
    };
    funds.get_mut(&100).unwrap().deduct(loss1.qty * loss1.price);

    let loss2 = SocializedLoss {
        user_id: 2,
        symbol_id: 100,
        round: 4,
        side: 1,
        price: 50_000,
        qty: 2,
        timestamp_ns: 1000,
    };
    funds.get_mut(&100).unwrap().deduct(loss2.qty * loss2.price);

    assert_eq!(funds.get(&100).unwrap().balance, 0);
}

#[test]
fn fund_recovery_via_fee_collection() {
    let mut fund = InsuranceFund::new(100, -100_000);

    fund.add(10_000);
    fund.add(20_000);
    fund.add(30_000);
    fund.add(40_000);

    assert_eq!(fund.balance, 0);
}
