use rsx_messages::OrderRequest;
use rsx_risk::account::Account;
use rsx_risk::margin::ExposureIndex;
use rsx_risk::margin::MarginState;
use rsx_risk::margin::PortfolioMargin;
use rsx_risk::margin::SymbolRiskParams;
use rsx_risk::position::Position;
use rsx_risk::types::RejectReason;

fn make_pm(n: usize) -> PortfolioMargin {
    PortfolioMargin {
        symbol_params: vec![
            Some(SymbolRiskParams {
                initial_margin_rate: 1000,    // 10%
                maintenance_margin_rate: 500, // 5%
                max_leverage: 10,
                min_notional: 1,
            });
            n
        ],
    }
}

fn make_order(user_id: u32, symbol_id: u32, price: i64, qty: i64, side: u8) -> OrderRequest {
    OrderRequest {
        seq: 1,
        user_id,
        symbol_id,
        price,
        qty,
        order_id_hi: 0,
        order_id_lo: 0,
        timestamp_ns: 0,
        side,
        tif: 0,
        reduce_only: false,
        post_only: false,
        is_liquidation: false,
        _pad: [0; 3],
    }
}

// -- core --

#[test]
fn portfolio_margin_single_position() {
    let pm = make_pm(1);
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1); // long 10@100
    let a = Account::new(1, 50000);
    let marks = vec![110];
    let s = pm.calculate(&a, std::iter::once(&p), &marks, 0);
    // upnl = 10*(110-100) = 100
    assert_eq!(s.unrealized_pnl, 100);
    // equity = 50000 + 100 = 50100
    assert_eq!(s.equity, 50100);
    // notional = 10*110 = 1100, im = 1100*1000/10000 = 110
    assert_eq!(s.initial_margin, 110);
    // mm = 1100*500/10000 = 55
    assert_eq!(s.maintenance_margin, 55);
    // available = 50100 - 110 - 0 = 49990
    assert_eq!(s.available_margin, 49990);
}

#[test]
fn portfolio_margin_multi_symbol() {
    let pm = make_pm(2);
    let mut p0 = Position::new(1, 0);
    p0.apply_fill(0, 100, 10, 1);
    let mut p1 = Position::new(1, 1);
    p1.apply_fill(1, 200, 5, 2);
    let a = Account::new(1, 100000);
    let marks = vec![100, 200];
    let s = pm.calculate(&a, [&p0, &p1].into_iter(), &marks, 0);
    // upnl p0 = 0, upnl p1 = -5*(200-200)=0
    assert_eq!(s.unrealized_pnl, 0);
    // notional p0=1000, p1=1000, im=100+100=200
    assert_eq!(s.initial_margin, 200);
}

#[test]
fn portfolio_margin_long_short_offset() {
    let pm = make_pm(1);
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1);
    p.apply_fill(1, 100, 5, 2); // net 5 long
    let a = Account::new(1, 50000);
    let marks = vec![100];
    let s = pm.calculate(&a, std::iter::once(&p), &marks, 0);
    // notional = 5*100=500, im=50
    assert_eq!(s.initial_margin, 50);
}

#[test]
fn check_order_sufficient_margin_accepts() {
    let pm = make_pm(1);
    let a = Account::new(1, 10000);
    let marks = vec![100];
    let order = make_order(1, 0, 100, 1, 0);
    let result = pm.check_order(&a, std::iter::empty(), &order, &marks, 10, 0);
    assert!(result.is_ok());
}

#[test]
fn check_order_insufficient_margin_rejects() {
    let pm = make_pm(1);
    let a = Account::new(1, 5); // tiny collateral
    let marks = vec![100];
    let order = make_order(1, 0, 100, 10, 0);
    let result = pm.check_order(&a, std::iter::empty(), &order, &marks, 10, 0);
    assert_eq!(result, Err(RejectReason::InsufficientMargin));
}

#[test]
fn needs_liquidation_below_maintenance() {
    let pm = make_pm(1);
    let state = MarginState {
        equity: 49,
        maintenance_margin: 50,
        ..Default::default()
    };
    assert!(pm.needs_liquidation(&state));
}

#[test]
fn needs_liquidation_above_maintenance_ok() {
    let pm = make_pm(1);
    let state = MarginState {
        equity: 51,
        maintenance_margin: 50,
        ..Default::default()
    };
    assert!(!pm.needs_liquidation(&state));
}

// frozen-margin lifecycle is exercised in shard_test.rs
// against `RiskShard::frozen_for_user`.

// -- edge cases --

#[test]
fn check_order_exactly_at_margin_limit_accepts() {
    let pm = make_pm(1);
    // order_notional=100*10=1000, im=100, fee=1
    // need exactly 101
    let a = Account::new(1, 101);
    let marks = vec![100];
    let order = make_order(1, 0, 100, 10, 0);
    let result = pm.check_order(&a, std::iter::empty(), &order, &marks, 10, 0);
    assert!(result.is_ok());
}

#[test]
fn check_order_one_unit_over_limit_rejects() {
    let pm = make_pm(1);
    // need 101, have 100
    let a = Account::new(1, 100);
    let marks = vec![100];
    let order = make_order(1, 0, 100, 10, 0);
    let result = pm.check_order(&a, std::iter::empty(), &order, &marks, 10, 0);
    assert_eq!(result, Err(RejectReason::InsufficientMargin));
}

#[test]
fn margin_with_zero_collateral_rejects_all() {
    let pm = make_pm(1);
    let a = Account::new(1, 0);
    let marks = vec![100];
    let order = make_order(1, 0, 100, 1, 0);
    let result = pm.check_order(&a, std::iter::empty(), &order, &marks, 10, 0);
    assert_eq!(result, Err(RejectReason::InsufficientMargin));
}

#[test]
fn margin_with_no_positions_all_available() {
    let pm = make_pm(1);
    let a = Account::new(1, 10000);
    let marks = vec![100];
    let s = pm.calculate(&a, std::iter::empty(), &marks, 0);
    assert_eq!(s.available_margin, 10000);
    assert_eq!(s.initial_margin, 0);
}

#[test]
fn margin_unrealized_pnl_affects_equity() {
    let pm = make_pm(1);
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1);
    let a = Account::new(1, 1000);
    // mark at 50 -> upnl = 10*(50-100) = -500
    let s = pm.calculate(&a, std::iter::once(&p), &[50], 0);
    assert_eq!(s.equity, 500);
}

#[test]
fn margin_mark_price_unavailable_uses_index() {
    // When mark unavailable, caller passes index
    // as mark_price. This test just verifies calc
    // works with any price passed as mark.
    let pm = make_pm(1);
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1);
    let a = Account::new(1, 5000);
    let index_as_mark = vec![105];
    let s = pm.calculate(&a, std::iter::once(&p), &index_as_mark, 0);
    assert_eq!(s.unrealized_pnl, 50); // 10*(105-100)
}

#[test]
fn margin_upnl_sum_saturates_no_panic() {
    // Two positions whose individual upnl each fit in i64 but
    // whose SUM overflows i64. Plain `+=` panics in debug /
    // wraps in release (a wrap from large-positive could flip
    // negative, or two large-negatives wrap positive and mask
    // insolvency). saturating_add must clamp instead.
    let pm = make_pm(2);
    let mut p0 = Position::new(1, 0);
    p0.apply_fill(0, 0, 3_000_000_000, 1); // long 3e9 @ entry 0
    let mut p1 = Position::new(1, 1);
    p1.apply_fill(0, 0, 3_000_000_000, 2); // long 3e9 @ entry 0
    let a = Account::new(1, 1000);
    let marks = vec![2_000_000_000, 2_000_000_000];
    // each upnl = 3e9 * 2e9 = 6e18 (fits i64); sum 1.2e19 > i64::MAX
    let s = pm.calculate(&a, [&p0, &p1].into_iter(), &marks, 0);
    assert_eq!(s.unrealized_pnl, i64::MAX);
    // equity saturates positive (huge gains) -> solvent, sane
    assert_eq!(s.equity, i64::MAX);
}

#[test]
fn margin_upnl_negative_sum_saturates_solvent_guard() {
    // Two deeply-underwater positions: each upnl fits i64 but
    // the sum underflows. A wrap (release `+=`) could flip the
    // sum positive and mask insolvency -> wrongly solvent.
    // saturating_add must clamp to i64::MIN (insolvent).
    let pm = make_pm(2);
    let mut p0 = Position::new(1, 0);
    // long 3e9 @ entry 2e9, mark 0 -> upnl = 3e9*(0-2e9) = -6e18
    p0.apply_fill(0, 2_000_000_000, 3_000_000_000, 1);
    let mut p1 = Position::new(1, 1);
    p1.apply_fill(0, 2_000_000_000, 3_000_000_000, 2);
    let a = Account::new(1, 1000);
    let marks = vec![0, 0];
    let s = pm.calculate(&a, [&p0, &p1].into_iter(), &marks, 0);
    assert_eq!(s.unrealized_pnl, i64::MIN);
    // equity = collateral + MIN, saturating -> deeply negative
    // (account looks insolvent: the safe direction).
    assert_eq!(s.equity, i64::MIN.saturating_add(1000));
    assert!(s.equity < 0);
}

#[test]
fn margin_mark_price_zero_handled() {
    let pm = make_pm(1);
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1);
    let a = Account::new(1, 5000);
    let s = pm.calculate(&a, std::iter::once(&p), &[0], 0);
    // upnl = 10*(0-100) = -1000
    assert_eq!(s.unrealized_pnl, -1000);
    assert_eq!(s.initial_margin, 0); // notional=0
}

#[test]
fn margin_max_leverage_enforced() {
    // max_leverage is stored but enforcement is
    // via im_rate (1/leverage). Verify im_rate works.
    let pm = PortfolioMargin {
        symbol_params: vec![Some(SymbolRiskParams {
            initial_margin_rate: 500, // 5% = 20x
            maintenance_margin_rate: 250,
            max_leverage: 20,
            min_notional: 1,
        })],
    };
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 100, 1);
    let a = Account::new(1, 1000);
    let s = pm.calculate(&a, std::iter::once(&p), &[100], 0);
    // notional=10000, im=500
    assert_eq!(s.initial_margin, 500);
}

#[test]
fn fee_reserve_included_in_pretrade_check() {
    let pm = make_pm(1);
    // order_notional=100*10=1000
    // im=100, fee=1000*50/10000=5, total=105
    let a = Account::new(1, 105);
    let marks = vec![100];
    let order = make_order(1, 0, 100, 10, 0);
    let result = pm.check_order(&a, std::iter::empty(), &order, &marks, 50, 0);
    assert!(result.is_ok());
    // With 104, should fail
    let a2 = Account::new(1, 104);
    let result2 = pm.check_order(&a2, std::iter::empty(), &order, &marks, 50, 0);
    assert_eq!(result2, Err(RejectReason::InsufficientMargin));
}

// -- exposure index --

#[test]
fn exposure_add_user_on_fill() {
    let mut idx = ExposureIndex::new(4);
    idx.add_user(0, 42);
    assert_eq!(idx.users_for_symbol(0), &[42]);
}

#[test]
fn exposure_remove_user_on_close() {
    let mut idx = ExposureIndex::new(4);
    idx.add_user(0, 42);
    idx.remove_user(0, 42);
    assert!(idx.users_for_symbol(0).is_empty());
}

#[test]
fn exposure_no_duplicate_entries() {
    let mut idx = ExposureIndex::new(4);
    idx.add_user(0, 42);
    idx.add_user(0, 42);
    assert_eq!(idx.users_for_symbol(0).len(), 1);
}

#[test]
fn exposure_user_in_multiple_symbols() {
    let mut idx = ExposureIndex::new(4);
    idx.add_user(0, 42);
    idx.add_user(1, 42);
    assert_eq!(idx.users_for_symbol(0), &[42]);
    assert_eq!(idx.users_for_symbol(1), &[42]);
}

#[test]
fn exposure_close_one_symbol_keeps_others() {
    let mut idx = ExposureIndex::new(4);
    idx.add_user(0, 42);
    idx.add_user(1, 42);
    idx.remove_user(0, 42);
    assert!(idx.users_for_symbol(0).is_empty());
    assert_eq!(idx.users_for_symbol(1), &[42]);
}

#[test]
fn exposure_empty_vec_for_unused_symbol() {
    let idx = ExposureIndex::new(4);
    assert!(idx.users_for_symbol(3).is_empty());
}

#[test]
#[should_panic]
fn exposure_symbol_idx_out_of_bounds_panics() {
    let mut idx = ExposureIndex::new(4);
    idx.add_user(99, 1);
}

#[test]
fn check_order_reduce_only_bypasses_margin() {
    let pm = make_pm(1);
    let a = Account::new(1, 0); // zero collateral
    let marks = vec![100];
    let mut order = make_order(1, 0, 100, 10, 0);
    order.reduce_only = true;
    let result = pm.check_order(&a, std::iter::empty(), &order, &marks, 10, 0);
    assert_eq!(result, Ok(0));
}

#[test]
fn check_order_liquidation_order_skips_margin_check() {
    let pm = make_pm(1);
    let a = Account::new(1, 0); // zero collateral
    let marks = vec![100];
    let mut order = make_order(1, 0, 100, 10, 0);
    order.is_liquidation = true;
    let result = pm.check_order(&a, std::iter::empty(), &order, &marks, 10, 0);
    assert_eq!(result, Ok(0));
}

#[test]
fn funding_uses_latest_mark_price() {
    use rsx_risk::FundingConfig;
    use rsx_risk::LiquidationConfig;
    use rsx_risk::ReplicationConfig;
    use rsx_risk::RiskShard;
    use rsx_risk::ShardConfig;
    let config = ShardConfig {
        shard_id: 0,
        shard_count: 2,
        max_symbols: 1,
        symbol_params: vec![Some(SymbolRiskParams {
            initial_margin_rate: 1000,
            maintenance_margin_rate: 500,
            max_leverage: 10,
            min_notional: 1,
        })],
        taker_fee_bps: vec![5],
        maker_fee_bps: vec![-1],
        funding_config: FundingConfig::default(),
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig::default(),
    };
    let mut s = RiskShard::new(config);
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.update_mark(0, 10_000);
    s.index_prices[0].price = 9_900;
    s.index_prices[0].valid = true;
    let f = rsx_risk::FillEvent {
        seq: 1,
        symbol_id: 0,
        taker_user_id: 0,
        maker_user_id: 1,
        price: 10_000,
        qty: 100,
        taker_side: 0,
        timestamp_ns: 0,
    };
    s.process_fill(&f);
    let before = s.accounts[&0].collateral;
    s.update_mark(0, 10_200);
    s.maybe_settle_funding(28_800);
    let after = s.accounts[&0].collateral;
    assert_ne!(before, after);
    s.update_mark(0, 10_500);
    s.index_prices[0].price = 10_000;
    let mid = s.accounts[&0].collateral;
    s.maybe_settle_funding(57_600);
    let after2 = s.accounts[&0].collateral;
    assert_ne!(mid, after2);
    let delta1 = (after - before).abs();
    let delta2 = (after2 - mid).abs();
    assert!(delta2 > delta1);
}

#[test]
fn max_leverage_caps_exposure_below_the_margin_rate() {
    // A 10% initial-margin rate implies a 10x cap on its own. With
    // max_leverage set to 2, an order the margin arithmetic would
    // happily accept must still be refused -- otherwise the
    // per-symbol cap traders are shown means nothing.
    let mut pm = make_pm(1);
    pm.symbol_params[0]
        .as_mut()
        .expect("configured")
        .max_leverage = 2;
    let account = Account::new(0, 1_000);
    let positions: Vec<Position> = Vec::new();
    let marks = [100i64];

    // Notional 500 = 0.5x equity: inside both caps.
    let small = make_order(0, 0, 100, 5, 0);
    assert!(pm
        .check_order(&account, positions.iter(), &small, &marks, 0, 0)
        .is_ok());

    // Notional 5_000 = 5x equity: inside the 10% margin rate's
    // implied 10x, outside the 2x symbol cap.
    let big = make_order(0, 0, 100, 50, 0);
    assert_eq!(
        pm.check_order(&account, positions.iter(), &big, &marks, 0, 0),
        Err(RejectReason::InsufficientMargin),
        "order at 5x must be refused when max_leverage is 2"
    );
}

// -- minimum order notional --

const MIN_NOTIONAL: i64 = 1_000;

fn make_pm_min(min_notional: i64) -> PortfolioMargin {
    let mut pm = make_pm(2);
    for params in pm.symbol_params.iter_mut().flatten() {
        params.min_notional = min_notional;
    }
    pm
}

fn position_of(symbol_id: u32, side: u8, qty: i64) -> Position {
    let mut position = Position::new(1, symbol_id);
    position.apply_fill(side, 100, qty, 1);
    position
}

fn check(
    pm: &PortfolioMargin,
    collateral: i64,
    positions: &[Position],
    order: &OrderRequest,
) -> Result<i64, RejectReason> {
    let account = Account::new(1, collateral);
    pm.check_order(&account, positions.iter(), order, &[100, 100], 0, 0)
}

fn reduce_only(mut order: OrderRequest, tif: u8) -> OrderRequest {
    order.reduce_only = true;
    order.tif = tif;
    order
}

#[test]
fn min_notional_exactly_at_minimum_accepts() {
    let pm = make_pm_min(MIN_NOTIONAL);
    let order = make_order(1, 0, 100, 10, 0);
    assert_eq!(order.price * order.qty, MIN_NOTIONAL);
    assert!(check(&pm, 1_000_000, &[], &order).is_ok());
}

#[test]
fn min_notional_one_raw_unit_below_rejects() {
    let pm = make_pm_min(MIN_NOTIONAL);
    let by_price = make_order(1, 0, 999, 1, 0);
    assert_eq!(
        check(&pm, 1_000_000, &[], &by_price),
        Err(RejectReason::OrderTooSmall)
    );
    let by_qty = make_order(1, 0, 100, 9, 0);
    assert_eq!(by_qty.price * by_qty.qty, MIN_NOTIONAL - 100);
    assert_eq!(
        check(&pm, 1_000_000, &[], &by_qty),
        Err(RejectReason::OrderTooSmall)
    );
    let one_short = make_order(1, 0, 999, 1, 0);
    assert_eq!(one_short.price * one_short.qty, MIN_NOTIONAL - 1);
    assert_eq!(
        check(&pm, 1_000_000, &[], &one_short),
        Err(RejectReason::OrderTooSmall)
    );
}

#[test]
fn min_notional_is_checked_before_margin() {
    // No collateral at all: the order is both too small and unfunded.
    // The size is the reason, so the user is told to resize.
    let pm = make_pm_min(MIN_NOTIONAL);
    let order = make_order(1, 0, 999, 1, 0);
    assert_eq!(check(&pm, 0, &[], &order), Err(RejectReason::OrderTooSmall));
}

#[test]
fn min_notional_applies_to_every_tif_and_side() {
    let pm = make_pm_min(MIN_NOTIONAL);
    for tif in 0..=2u8 {
        for side in 0..=1u8 {
            let mut order = make_order(1, 0, 999, 1, side);
            order.tif = tif;
            assert_eq!(
                check(&pm, 1_000_000, &[], &order),
                Err(RejectReason::OrderTooSmall),
                "tif={tif} side={side}"
            );
        }
    }
}

#[test]
fn min_notional_uses_the_orders_own_symbol() {
    let mut pm = make_pm_min(MIN_NOTIONAL);
    pm.symbol_params[1]
        .as_mut()
        .expect("configured")
        .min_notional = 10;
    let order = make_order(1, 1, 100, 1, 0);
    assert!(check(&pm, 1_000_000, &[], &order).is_ok());
    let order = make_order(1, 0, 100, 1, 0);
    assert_eq!(
        check(&pm, 1_000_000, &[], &order),
        Err(RejectReason::OrderTooSmall)
    );
}

#[test]
fn min_notional_overflowing_notional_is_a_margin_reject() {
    // price * qty does not fit i64: far above any minimum, so it is
    // never "too small" and lands on the margin path.
    let pm = make_pm_min(i64::MAX);
    let order = make_order(1, 0, i64::MAX, 2, 0);
    assert_eq!(
        check(&pm, 1_000_000, &[], &order),
        Err(RejectReason::InsufficientMargin)
    );
    let order = make_order(1, 0, i64::MAX, i64::MAX, 0);
    assert_eq!(
        check(&pm, 1_000_000, &[], &order),
        Err(RejectReason::InsufficientMargin)
    );
}

#[test]
fn min_notional_overflowing_reduce_only_gtc_clears_the_floor() {
    let pm = make_pm_min(i64::MAX);
    let order = reduce_only(make_order(1, 0, i64::MAX, 2, 1), 0);
    assert_eq!(check(&pm, 0, &[], &order), Ok(0));
}

#[test]
fn min_notional_liquidation_below_minimum_accepts() {
    let pm = make_pm_min(MIN_NOTIONAL);
    let mut order = make_order(1, 0, 1, 1, 1);
    order.is_liquidation = true;
    assert_eq!(check(&pm, 0, &[], &order), Ok(0));
}

#[test]
fn min_notional_liquidation_on_unconfigured_symbol_accepts() {
    let pm = make_pm_min(MIN_NOTIONAL);
    let mut order = make_order(1, 7, 1, 1, 1);
    order.is_liquidation = true;
    assert_eq!(check(&pm, 0, &[], &order), Ok(0));
}

#[test]
fn min_notional_reduce_only_ioc_and_fok_below_minimum_accept() {
    let pm = make_pm_min(MIN_NOTIONAL);
    let long_ten = [position_of(0, 0, 10)];
    for tif in [1u8, 2u8] {
        // Partial reduce, far below the minimum, no collateral.
        let order = reduce_only(make_order(1, 0, 1, 3, 1), tif);
        assert_eq!(check(&pm, 0, &long_ten, &order), Ok(0), "tif={tif}");
        // Even with no position at all: the ME owns the reduce-only
        // verdict, risk only waives the size floor.
        assert_eq!(check(&pm, 0, &[], &order), Ok(0), "tif={tif}");
    }
}

#[test]
fn min_notional_reduce_only_gtc_closing_whole_position_accepts() {
    let pm = make_pm_min(MIN_NOTIONAL);
    let long_ten = [position_of(0, 0, 10)];
    let close_long = reduce_only(make_order(1, 0, 1, 10, 1), 0);
    assert_eq!(check(&pm, 0, &long_ten, &close_long), Ok(0));
    let short_ten = [position_of(0, 1, 10)];
    let close_short = reduce_only(make_order(1, 0, 1, 10, 0), 0);
    assert_eq!(check(&pm, 0, &short_ten, &close_short), Ok(0));
}

#[test]
fn min_notional_reduce_only_gtc_partial_close_below_minimum_rejects() {
    let pm = make_pm_min(MIN_NOTIONAL);
    let long_ten = [position_of(0, 0, 10)];
    let partial = reduce_only(make_order(1, 0, 1, 9, 1), 0);
    assert_eq!(
        check(&pm, 0, &long_ten, &partial),
        Err(RejectReason::OrderTooSmall)
    );
    let larger = reduce_only(make_order(1, 0, 1, 11, 1), 0);
    assert_eq!(
        check(&pm, 0, &long_ten, &larger),
        Err(RejectReason::OrderTooSmall)
    );
}

#[test]
fn min_notional_reduce_only_gtc_partial_close_at_minimum_accepts() {
    let pm = make_pm_min(MIN_NOTIONAL);
    let long_ten = [position_of(0, 0, 100)];
    let partial = reduce_only(make_order(1, 0, 100, 10, 1), 0);
    assert_eq!(partial.price * partial.qty, MIN_NOTIONAL);
    assert_eq!(check(&pm, 0, &long_ten, &partial), Ok(0));
    let below = reduce_only(make_order(1, 0, 999, 1, 1), 0);
    assert_eq!(
        check(&pm, 0, &long_ten, &below),
        Err(RejectReason::OrderTooSmall)
    );
}

#[test]
fn min_notional_reduce_only_gtc_without_a_position_rejects() {
    let pm = make_pm_min(MIN_NOTIONAL);
    let order = reduce_only(make_order(1, 0, 1, 10, 1), 0);
    assert_eq!(check(&pm, 0, &[], &order), Err(RejectReason::OrderTooSmall));
}

#[test]
fn min_notional_whole_position_is_the_orders_own_symbol() {
    let pm = make_pm_min(MIN_NOTIONAL);
    let other_symbol = [position_of(1, 0, 10)];
    let order = reduce_only(make_order(1, 0, 1, 10, 1), 0);
    assert_eq!(
        check(&pm, 0, &other_symbol, &order),
        Err(RejectReason::OrderTooSmall)
    );
}

#[test]
fn min_notional_whole_position_is_net_of_both_sides() {
    let pm = make_pm_min(MIN_NOTIONAL);
    // Long 10 then short 4 fills leave a net long 6.
    let mut position = Position::new(1, 0);
    position.apply_fill(0, 100, 10, 1);
    position.apply_fill(1, 100, 4, 2);
    let positions = [position];
    let close = reduce_only(make_order(1, 0, 1, 6, 1), 0);
    assert_eq!(check(&pm, 0, &positions, &close), Ok(0));
    let stale = reduce_only(make_order(1, 0, 1, 10, 1), 0);
    assert_eq!(
        check(&pm, 0, &positions, &stale),
        Err(RejectReason::OrderTooSmall)
    );
}

#[test]
fn min_notional_non_reduce_only_never_uses_the_position_exemption() {
    let pm = make_pm_min(MIN_NOTIONAL);
    let long_ten = [position_of(0, 0, 10)];
    let order = make_order(1, 0, 1, 10, 1);
    assert_eq!(
        check(&pm, 1_000_000, &long_ten, &order),
        Err(RejectReason::OrderTooSmall)
    );
}

#[test]
fn min_notional_unconfigured_symbol_rejects_reduce_only_gtc() {
    let pm = make_pm_min(MIN_NOTIONAL);
    let order = reduce_only(make_order(1, 7, 1, 10, 1), 0);
    assert_eq!(
        check(&pm, 0, &[], &order),
        Err(RejectReason::SymbolNotConfigured)
    );
}

#[test]
fn order_too_small_maps_to_wire_reason_14() {
    use rsx_types::FailureReason;
    assert_eq!(FailureReason::from(RejectReason::OrderTooSmall) as u8, 14);
    assert_eq!(
        FailureReason::from(RejectReason::InsufficientMargin),
        FailureReason::InsufficientMargin
    );
    assert_eq!(
        FailureReason::from(RejectReason::SymbolNotConfigured),
        FailureReason::SymbolNotFound
    );
}
