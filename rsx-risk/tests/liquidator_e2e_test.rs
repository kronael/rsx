/// E2E liquidator tests for the 6 required behaviors.
/// Complements liquidation_test.rs (unit) and
/// shard_e2e_test.rs (shard integration).

use rsx_risk::liquidation::LiquidationEngine;
use rsx_risk::liquidation::LiquidationStatus;
use rsx_risk::Account;
use rsx_risk::FillEvent;
use rsx_risk::FundingConfig;
use rsx_risk::LiquidationConfig;
use rsx_risk::OrderRequest;
use rsx_risk::OrderResponse;
use rsx_risk::RejectReason;
use rsx_risk::ReplicationConfig;
use rsx_risk::RiskShard;
use rsx_risk::ShardConfig;
use rsx_risk::SymbolRiskParams;

// ----------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------

fn shard() -> RiskShard {
    RiskShard::new(ShardConfig {
        shard_id: 0,
        shard_count: 1,
        max_symbols: 4,
        symbol_params: vec![
            SymbolRiskParams {
                initial_margin_rate: 1000,       // 10%
                maintenance_margin_rate: 500,    // 5%
                max_leverage: 10,
            };
            4
        ],
        taker_fee_bps: vec![5; 4],
        maker_fee_bps: vec![-1; 4],
        funding_config: FundingConfig::default(),
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig::default(),
    })
}

fn fill(
    taker: u32,
    maker: u32,
    sym: u32,
    price: i64,
    qty: i64,
    side: u8,
    seq: u64,
) -> FillEvent {
    FillEvent {
        seq,
        symbol_id: sym,
        taker_user_id: taker,
        maker_user_id: maker,
        price,
        qty,
        taker_side: side,
        timestamp_ns: 0,
    }
}

fn order(
    user_id: u32,
    symbol_id: u32,
    price: i64,
    qty: i64,
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
        side: 0,
        tif: 0,
        reduce_only: false,
        post_only: false,
        is_liquidation: false,
        _pad: [0; 3],
    }
}

fn engine_no_delay() -> LiquidationEngine {
    LiquidationEngine::new(0, 10, 10)
}

// ----------------------------------------------------------------
// 1. Largest position liquidated first
//
// The engine liquidates all enqueued positions per maybe_process
// call. When a user has multiple symbols, all get orders in the
// same round. The order quantity equals the full position for each
// symbol, so the largest notional position receives the largest
// qty*mark order. Verify all symbols are liquidated and the largest
// notional order is present.
// ----------------------------------------------------------------

#[test]
fn largest_position_order_qty_reflects_position_size() {
    let mut e = engine_no_delay();
    // sym 0: qty=500, sym 1: qty=50, sym 2: qty=10
    e.enqueue(1, 0, 0);
    e.enqueue(1, 1, 0);
    e.enqueue(1, 2, 0);

    let mark = 1000i64;
    let (orders, _) = e.maybe_process(
        0,
        &|_, sid| match sid {
            0 => 500,
            1 => 50,
            _ => 10,
        },
        &|_| mark,
    );

    assert_eq!(orders.len(), 3);

    let mut qty_by_sym: Vec<(u32, i64)> =
        orders.iter().map(|o| (o.symbol_id, o.qty)).collect();
    qty_by_sym.sort_by_key(|&(sid, _)| sid);

    assert_eq!(qty_by_sym[0], (0, 500)); // largest
    assert_eq!(qty_by_sym[1], (1, 50));
    assert_eq!(qty_by_sym[2], (2, 10));  // smallest

    // Notional = qty * mark; largest notional = sym 0
    let max_notional = qty_by_sym
        .iter()
        .map(|&(_, q)| q * mark)
        .max()
        .unwrap();
    assert_eq!(max_notional, 500 * mark);
}

#[test]
fn multi_symbol_all_get_liquidation_orders() {
    // Engine generates one order per symbol per round.
    // All three symbols receive orders simultaneously.
    let mut e = engine_no_delay();
    e.enqueue(1, 0, 0);
    e.enqueue(1, 1, 0);
    e.enqueue(1, 2, 0);

    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 100,
        &|_| 50_000,
    );

    let syms: Vec<u32> =
        orders.iter().map(|o| o.symbol_id).collect();
    assert_eq!(orders.len(), 3);
    assert!(syms.contains(&0));
    assert!(syms.contains(&1));
    assert!(syms.contains(&2));
}

// ----------------------------------------------------------------
// 2. Partial liquidation reduces to target
//
// When a partial fill reduces the position, the next round sends
// an order for the remaining quantity only. When position reaches
// zero the engine marks Done and no further orders are generated.
// ----------------------------------------------------------------

#[test]
fn partial_fill_sends_remaining_qty_next_round() {
    let mut e = engine_no_delay();
    e.enqueue(1, 0, 0);

    // Round 1: full position = 200
    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 200,
        &|_| 10_000,
    );
    assert_eq!(orders[0].qty, 200);

    // Partial fill: position reduced to 80
    let (orders, _) = e.maybe_process(
        1,
        &|_, _| 80,
        &|_| 10_000,
    );
    assert_eq!(orders[0].qty, 80);

    // State still Active (position not zero yet)
    assert_eq!(
        e.active[0].status,
        LiquidationStatus::Active
    );
}

#[test]
fn position_reaches_zero_marks_done_no_further_orders() {
    let mut e = engine_no_delay();
    e.enqueue(1, 0, 0);

    // Round 1
    let (_, _) = e.maybe_process(
        0,
        &|_, _| 100,
        &|_| 10_000,
    );
    // Position fully filled (reduced to 0)
    let (orders, _) = e.maybe_process(
        1,
        &|_, _| 0,
        &|_| 10_000,
    );
    assert_eq!(orders.len(), 0);
    assert_eq!(
        e.active[0].status,
        LiquidationStatus::Done
    );

    // No more orders after Done
    let (orders, _) = e.maybe_process(
        2,
        &|_, _| 0,
        &|_| 10_000,
    );
    assert_eq!(orders.len(), 0);
}

#[test]
fn partial_recovery_one_symbol_continues_others() {
    // sym 0 position closes first; sym 1 continues.
    let mut e = engine_no_delay();
    e.enqueue(1, 0, 0);
    e.enqueue(1, 1, 0);

    // Round 1: both have positions
    let (orders, _) = e.maybe_process(
        0,
        &|_, sid| if sid == 0 { 50 } else { 100 },
        &|_| 10_000,
    );
    assert_eq!(orders.len(), 2);

    // Round 2: sym 0 position closed, sym 1 still open
    let (orders, _) = e.maybe_process(
        1,
        &|_, sid| if sid == 0 { 0 } else { 100 },
        &|_| 10_000,
    );
    // Only sym 1 order generated
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].symbol_id, 1);
}

// ----------------------------------------------------------------
// 3. New orders rejected during liquidation
//
// When a user's equity falls below maintenance margin, the shard
// checks needs_liquidation on each process_order call and rejects
// non-liquidation orders with UserInLiquidation.
// ----------------------------------------------------------------

#[test]
fn new_order_rejected_when_user_in_liquidation() {
    let mut s = shard();
    // Small collateral, large position -> underwater
    s.accounts.insert(0, Account::new(0, 10_000));
    s.mark_prices[0] = 10_000;

    // Long 1000 contracts at 10,000 -> notional = 10,000,000
    // MM = 10,000,000 * 500/10000 = 500,000
    // equity = 10,000 (no upnl since position taken at mark)
    // equity(10k) << maint(500k) -> liquidation
    s.process_fill(&fill(0, 1, 0, 10_000, 1000, 0, 1));

    let resp = s.process_order(&order(0, 0, 10_000, 1));
    assert!(
        matches!(
            resp,
            OrderResponse::Rejected {
                reason: RejectReason::UserInLiquidation,
                ..
            }
        ),
        "expected UserInLiquidation, got {:?}",
        resp
    );
}

#[test]
fn liquidation_order_accepted_when_user_in_liquidation() {
    let mut s = shard();
    s.accounts.insert(0, Account::new(0, 10_000));
    s.mark_prices[0] = 10_000;

    s.process_fill(&fill(0, 1, 0, 10_000, 1000, 0, 1));

    let mut liq = order(0, 0, 10_000, 1);
    liq.is_liquidation = true;
    let resp = s.process_order(&liq);
    assert!(
        matches!(resp, OrderResponse::Accepted { .. }),
        "expected Accepted, got {:?}",
        resp
    );
}

#[test]
fn engine_is_in_liquidation_blocks_conceptually() {
    // At engine level: is_in_liquidation returns true for
    // active entries. Shard uses this for logging/tracking;
    // the actual order gate is process_order needs_liquidation.
    let mut e = engine_no_delay();
    e.enqueue(7, 3, 0);
    assert!(e.is_in_liquidation(7, 3));
    assert!(!e.is_in_liquidation(7, 0)); // different symbol
    assert!(!e.is_in_liquidation(8, 3)); // different user
}

// ----------------------------------------------------------------
// 4. Price drop triggers liquidation
//
// When mark price falls such that equity < maintenance margin,
// a subsequent process_order must be rejected with
// UserInLiquidation. Uses update_mark to simulate price drop.
// ----------------------------------------------------------------

#[test]
fn mark_price_drop_triggers_liquidation_rejection() {
    let mut s = shard();
    // 200_000 collateral: IM for 100@10000 = 100,000
    // leaving 100,000 available for the test order.
    s.accounts.insert(0, Account::new(0, 200_000));
    s.mark_prices[0] = 10_000;

    // Long 100 at 10,000 -> notional = 1,000,000
    // IM = 100,000 (10%); MM = 50,000 (5%)
    // equity = 200,000 -> healthy (available = 100,000)
    s.process_fill(&fill(0, 1, 0, 10_000, 100, 0, 1));

    // Confirm order accepted before price drop
    let ok = s.process_order(&order(0, 0, 10_000, 1));
    assert!(
        matches!(ok, OrderResponse::Accepted { .. }),
        "expected Accepted before drop"
    );
    // Release the frozen margin from above
    s.accounts.get_mut(&0).unwrap().release_margin(
        match ok {
            OrderResponse::Accepted {
                margin_reserved, ..
            } => margin_reserved,
            _ => 0,
        },
    );

    // Drop mark price — upnl = 100*(1-10000) = -999,900
    // equity = 200,000 + (-999,900) = -799,900
    // maint = 100*1*500/10000 = 5 (tiny at new mark)
    // equity(-799,900) < maint(5) -> liquidation
    s.update_mark(0, 1);

    let rejected = s.process_order(&order(0, 0, 1, 1));
    assert!(
        matches!(
            rejected,
            OrderResponse::Rejected {
                reason: RejectReason::UserInLiquidation,
                ..
            }
        ),
        "expected UserInLiquidation after drop, got {:?}",
        rejected
    );
}

#[test]
fn gradual_price_drop_crosses_mm_threshold() {
    let mut s = shard();
    // Taker fee = qty*price*5/10000 = 100*10000*5/10000 = 500
    // Need post-fee equity = 50,001 to be 1 above MM=50,000.
    // So collateral = 50,001 + 500 (fee) = 50,501.
    // After fill: collateral = 50,001, upnl = 0,
    //   equity = 50,001, mm = 50,000 -> NOT liquidated.
    s.accounts.insert(0, Account::new(0, 50_501));
    s.mark_prices[0] = 10_000;
    s.process_fill(&fill(0, 1, 0, 10_000, 100, 0, 1));

    // Confirm not in liquidation at borderline.
    // reduce_only bypasses margin check; side=1 (sell) reduces long.
    let mut ro = order(0, 0, 10_000, 1);
    ro.reduce_only = true;
    ro.side = 1;
    let resp_before = s.process_order(&ro);
    assert!(
        matches!(resp_before, OrderResponse::Accepted { .. }),
        "reduce_only at borderline should be accepted, \
         got {:?}",
        resp_before
    );

    // Drop by 1 tick: upnl = 100*(9999-10000) = -100
    // equity = 50,001 - 100 = 49,901
    // mm at new mark = 100*9999*500/10000 = 49,995
    // equity(49,901) < mm(49,995) -> liquidation
    s.update_mark(0, 9999);
    let rejected = s.process_order(&order(0, 0, 9999, 1));
    assert!(
        matches!(
            rejected,
            OrderResponse::Rejected {
                reason: RejectReason::UserInLiquidation,
                ..
            }
        ),
        "expected UserInLiquidation after 1-tick drop, \
         got {:?}",
        rejected
    );
}

// ----------------------------------------------------------------
// 5. Liquidation round escalation
//
// Slippage formula: round^2 * base_slip_bps.
// Round 1: slip = 1*1*10 = 10 bps
// Round 2: slip = 2*2*10 = 40 bps
// Round 3: slip = 3*3*10 = 90 bps
// For a long position (sell order): price = mark*(10000-slip)/10000
// ----------------------------------------------------------------

#[test]
fn slippage_round_1_value() {
    let mut e = LiquidationEngine::new(0, 10, 10);
    e.enqueue(1, 0, 0);
    let mark = 100_000i64;
    let (orders, _) =
        e.maybe_process(0, &|_, _| 100, &|_| mark);
    // round=1, slip=1*1*10=10 bps
    // sell price = 100000*(10000-10)/10000 = 99900
    assert_eq!(orders[0].price, 99_900);
    assert_eq!(orders[0].side, 1); // sell
}

#[test]
fn slippage_round_2_value() {
    let mut e = LiquidationEngine::new(0, 10, 10);
    e.enqueue(1, 0, 0);
    let mark = 100_000i64;
    // Round 1
    let (_, _) = e.maybe_process(0, &|_, _| 100, &|_| mark);
    // Round 2: slip=2*2*10=40 bps
    // sell price = 100000*(10000-40)/10000 = 99600
    let (orders, _) =
        e.maybe_process(1, &|_, _| 100, &|_| mark);
    assert_eq!(orders[0].price, 99_600);
}

#[test]
fn slippage_round_3_value() {
    let mut e = LiquidationEngine::new(0, 10, 10);
    e.enqueue(1, 0, 0);
    let mark = 100_000i64;
    // Rounds 1 and 2
    e.maybe_process(0, &|_, _| 100, &|_| mark);
    e.maybe_process(1, &|_, _| 100, &|_| mark);
    // Round 3: slip=3*3*10=90 bps
    // sell price = 100000*(10000-90)/10000 = 99100
    let (orders, _) =
        e.maybe_process(2, &|_, _| 100, &|_| mark);
    assert_eq!(orders[0].price, 99_100);
}

#[test]
fn slippage_monotonically_increases_across_rounds() {
    let mut e = LiquidationEngine::new(0, 10, 20);
    e.enqueue(1, 0, 0);
    let mark = 1_000_000i64;
    let mut last_price = mark; // sell price starts at mark or below
    for i in 0..10 {
        let (orders, _) = e.maybe_process(
            i as u64,
            &|_, _| 100,
            &|_| mark,
        );
        if orders.is_empty() {
            break;
        }
        let price = orders[0].price;
        assert!(
            price <= last_price,
            "round {}: price {} should be <= prev {}",
            i + 1,
            price,
            last_price
        );
        last_price = price;
    }
}

#[test]
fn slippage_capped_at_9999_bps() {
    // With base_slip=1000 and high rounds, slip caps at 9999
    let mut e = LiquidationEngine::new(0, 1000, 20);
    e.enqueue(1, 0, 0);
    let mark = 100_000i64;

    // Run enough rounds that round^2*1000 > 9999
    // round 4: 16*1000=16000 > 9999, so capped
    for i in 0..4 {
        let (orders, _) = e.maybe_process(
            i as u64,
            &|_, _| 100,
            &|_| mark,
        );
        if i == 3 {
            // slip capped at 9999: price = 100000*1/10000 = 10
            assert_eq!(orders[0].price, 10);
        }
    }
}

// ----------------------------------------------------------------
// 6. ORDER_FAILED retry with higher slippage
//
// When an order fails (e.g. no liquidity on a symbol), the symbol
// is halted. On resume, the round counter has already advanced, so
// the next order uses the higher slippage of the next round.
// ----------------------------------------------------------------

#[test]
fn order_failed_halt_resume_uses_higher_slippage() {
    let mut e = LiquidationEngine::new(0, 10, 10);
    e.enqueue(1, 0, 0);
    let mark = 100_000i64;

    // Round 1 fires: slip=10bps -> price=99900
    let (orders, _) =
        e.maybe_process(0, &|_, _| 100, &|_| mark);
    assert_eq!(orders[0].price, 99_900);

    // Simulate ORDER_FAILED: halt the symbol
    e.halt_symbol(0);

    // While halted: no orders emitted
    let (orders, _) =
        e.maybe_process(1, &|_, _| 100, &|_| mark);
    assert!(orders.is_empty());
    // State still Active
    assert_eq!(
        e.active[0].status,
        LiquidationStatus::Active
    );

    // Resume: round is now 2 (already incremented after r1)
    e.resume_symbol(0);
    let (orders, _) =
        e.maybe_process(2, &|_, _| 100, &|_| mark);
    // Round 2: slip=40bps -> price=99600 (higher slippage)
    assert_eq!(orders[0].price, 99_600);
    assert!(!e.is_halted(0));
}

#[test]
fn order_failed_round_not_reset_on_resume() {
    // After halt+resume, round continues from where it left off.
    let mut e = LiquidationEngine::new(0, 10, 10);
    e.enqueue(1, 0, 0);
    let mark = 100_000i64;

    // Advance to round 3
    e.maybe_process(0, &|_, _| 10, &|_| mark); // r1
    e.maybe_process(1, &|_, _| 10, &|_| mark); // r2

    let round_before = e.active[0].round;
    assert_eq!(round_before, 3);

    // Halt then resume
    e.halt_symbol(0);
    e.resume_symbol(0);

    // Round unchanged after halt/resume
    assert_eq!(e.active[0].round, round_before);

    // Next order uses round 3 slippage (90bps)
    let (orders, _) =
        e.maybe_process(2, &|_, _| 10, &|_| mark);
    // slip=3^2*10=90bps -> 100000*(10000-90)/10000=99100
    assert_eq!(orders[0].price, 99_100);
}

#[test]
fn multiple_symbols_halt_only_failed_symbol() {
    // sym 0 halted (order failed), sym 1 continues.
    let mut e = LiquidationEngine::new(0, 10, 10);
    e.enqueue(1, 0, 0);
    e.enqueue(1, 1, 0);

    // Round 1: both get orders
    let (orders, _) = e.maybe_process(
        0,
        &|_, _| 100,
        &|_| 50_000,
    );
    assert_eq!(orders.len(), 2);

    // Sym 0 order failed -> halt sym 0
    e.halt_symbol(0);

    // Round 2: only sym 1 gets an order
    let (orders, _) = e.maybe_process(
        1,
        &|_, _| 100,
        &|_| 50_000,
    );
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].symbol_id, 1);
}
