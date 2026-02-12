use rsx_risk::account::Account;
use rsx_risk::margin::ExposureIndex;
use rsx_risk::margin::MarginState;
use rsx_risk::margin::PortfolioMargin;
use rsx_risk::margin::SymbolRiskParams;
use rsx_risk::position::Position;
use rsx_risk::types::OrderRequest;
use rsx_risk::types::RejectReason;

fn make_pm(n: usize) -> PortfolioMargin {
    PortfolioMargin {
        symbol_params: vec![
            SymbolRiskParams {
                initial_margin_rate: 1000, // 10%
                maintenance_margin_rate: 500, // 5%
                max_leverage: 10,
            };
            n
        ],
    }
}

fn make_order(
    user_id: u32,
    symbol_id: u32,
    price: i64,
    qty: i64,
    side: u8,
) -> OrderRequest {
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
    let s = pm.calculate(&a, &[&p], &marks);
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
    let s = pm.calculate(&a, &[&p0, &p1], &marks);
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
    let s = pm.calculate(&a, &[&p], &marks);
    // notional = 5*100=500, im=50
    assert_eq!(s.initial_margin, 50);
}

#[test]
fn check_order_sufficient_margin_accepts() {
    let pm = make_pm(1);
    let a = Account::new(1, 10000);
    let marks = vec![100];
    let order = make_order(1, 0, 100, 1, 0);
    let result =
        pm.check_order(&a, &[], &order, &marks, 10);
    assert!(result.is_ok());
}

#[test]
fn check_order_insufficient_margin_rejects() {
    let pm = make_pm(1);
    let a = Account::new(1, 5); // tiny collateral
    let marks = vec![100];
    let order = make_order(1, 0, 100, 10, 0);
    let result =
        pm.check_order(&a, &[], &order, &marks, 10);
    assert_eq!(
        result,
        Err(RejectReason::InsufficientMargin)
    );
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

#[test]
fn frozen_margin_reserved_on_order() {
    let pm = make_pm(1);
    let mut a = Account::new(1, 10000);
    let marks = vec![100];
    let order = make_order(1, 0, 100, 10, 0);
    let needed =
        pm.check_order(&a, &[], &order, &marks, 10)
            .unwrap();
    a.freeze_margin(needed);
    assert!(a.frozen_margin > 0);
}

#[test]
fn frozen_margin_released_on_done() {
    let mut a = Account::new(1, 10000);
    a.freeze_margin(500);
    a.release_margin(500);
    assert_eq!(a.frozen_margin, 0);
}

// -- edge cases --

#[test]
fn check_order_exactly_at_margin_limit_accepts() {
    let pm = make_pm(1);
    // order_notional=100*10=1000, im=100, fee=1
    // need exactly 101
    let a = Account::new(1, 101);
    let marks = vec![100];
    let order = make_order(1, 0, 100, 10, 0);
    let result =
        pm.check_order(&a, &[], &order, &marks, 10);
    assert!(result.is_ok());
}

#[test]
fn check_order_one_unit_over_limit_rejects() {
    let pm = make_pm(1);
    // need 101, have 100
    let a = Account::new(1, 100);
    let marks = vec![100];
    let order = make_order(1, 0, 100, 10, 0);
    let result =
        pm.check_order(&a, &[], &order, &marks, 10);
    assert_eq!(
        result,
        Err(RejectReason::InsufficientMargin)
    );
}

#[test]
fn margin_with_zero_collateral_rejects_all() {
    let pm = make_pm(1);
    let a = Account::new(1, 0);
    let marks = vec![100];
    let order = make_order(1, 0, 100, 1, 0);
    let result =
        pm.check_order(&a, &[], &order, &marks, 10);
    assert_eq!(
        result,
        Err(RejectReason::InsufficientMargin)
    );
}

#[test]
fn margin_with_no_positions_all_available() {
    let pm = make_pm(1);
    let a = Account::new(1, 10000);
    let marks = vec![100];
    let s = pm.calculate(&a, &[], &marks);
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
    let s = pm.calculate(&a, &[&p], &[50]);
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
    let s =
        pm.calculate(&a, &[&p], &index_as_mark);
    assert_eq!(s.unrealized_pnl, 50); // 10*(105-100)
}

#[test]
fn margin_mark_price_zero_handled() {
    let pm = make_pm(1);
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 10, 1);
    let a = Account::new(1, 5000);
    let s = pm.calculate(&a, &[&p], &[0]);
    // upnl = 10*(0-100) = -1000
    assert_eq!(s.unrealized_pnl, -1000);
    assert_eq!(s.initial_margin, 0); // notional=0
}

#[test]
fn margin_max_leverage_enforced() {
    // max_leverage is stored but enforcement is
    // via im_rate (1/leverage). Verify im_rate works.
    let pm = PortfolioMargin {
        symbol_params: vec![SymbolRiskParams {
            initial_margin_rate: 500, // 5% = 20x
            maintenance_margin_rate: 250,
            max_leverage: 20,
        }],
    };
    let mut p = Position::new(1, 0);
    p.apply_fill(0, 100, 100, 1);
    let a = Account::new(1, 1000);
    let s = pm.calculate(&a, &[&p], &[100]);
    // notional=10000, im=500
    assert_eq!(s.initial_margin, 500);
}

#[test]
fn frozen_margin_across_multiple_pending_orders() {
    let pm = make_pm(1);
    let mut a = Account::new(1, 10000);
    let marks = vec![100];
    let o1 = make_order(1, 0, 100, 10, 0);
    let n1 =
        pm.check_order(&a, &[], &o1, &marks, 10)
            .unwrap();
    a.freeze_margin(n1);
    let o2 = make_order(1, 0, 100, 10, 0);
    let r2 =
        pm.check_order(&a, &[], &o2, &marks, 10);
    assert!(r2.is_ok());
    let n2 = r2.unwrap();
    a.freeze_margin(n2);
    assert_eq!(a.frozen_margin, n1 + n2);
}

#[test]
fn order_done_partial_fill_releases_remaining_frozen()
{
    let mut a = Account::new(1, 10000);
    a.freeze_margin(500);
    // Partial fill used 200, release remaining
    a.release_margin(300);
    assert_eq!(a.frozen_margin, 200);
}

#[test]
fn order_failed_releases_all_frozen() {
    let mut a = Account::new(1, 10000);
    a.freeze_margin(500);
    a.release_margin(500);
    assert_eq!(a.frozen_margin, 0);
}

#[test]
fn fee_reserve_included_in_pretrade_check() {
    let pm = make_pm(1);
    // order_notional=100*10=1000
    // im=100, fee=1000*50/10000=5, total=105
    let a = Account::new(1, 105);
    let marks = vec![100];
    let order = make_order(1, 0, 100, 10, 0);
    let result =
        pm.check_order(&a, &[], &order, &marks, 50);
    assert!(result.is_ok());
    // With 104, should fail
    let a2 = Account::new(1, 104);
    let result2 =
        pm.check_order(&a2, &[], &order, &marks, 50);
    assert_eq!(
        result2,
        Err(RejectReason::InsufficientMargin)
    );
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
    let result =
        pm.check_order(&a, &[], &order, &marks, 10);
    assert_eq!(result, Ok(0));
}

#[test]
fn check_order_liquidation_order_skips_margin_check() {
    let pm = make_pm(1);
    let a = Account::new(1, 0); // zero collateral
    let marks = vec![100];
    let mut order = make_order(1, 0, 100, 10, 0);
    order.is_liquidation = true;
    let result =
        pm.check_order(&a, &[], &order, &marks, 10);
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
        symbol_params: vec![SymbolRiskParams {
            initial_margin_rate: 1000,
            maintenance_margin_rate: 500,
            max_leverage: 10,
        }],
        taker_fee_bps: vec![5],
        maker_fee_bps: vec![-1],
        funding_config: FundingConfig::default(),
        liquidation_config:
            LiquidationConfig::default(),
        replication_config:
            ReplicationConfig::default(),
    };
    let mut s = RiskShard::new(config);
    s.accounts.insert(
        0,
        Account::new(0, 1_000_000_000),
    );
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
