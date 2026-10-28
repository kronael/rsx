use rsx_messages::CancelRequest;
use rsx_messages::OrderRequest;
use rsx_risk::persist::PersistEvent;
use rsx_risk::rings::MeCommand;
use rsx_risk::types::RejectReason;
use rsx_risk::Account;
use rsx_risk::BboUpdate;
use rsx_risk::FillEvent;
use rsx_risk::LiquidationConfig;
use rsx_risk::OrderResponse;
use rsx_risk::ReplicationConfig;
use rsx_risk::RiskShard;
use rsx_risk::ShardConfig;
use rsx_risk::SymbolRiskConfig;
use rsx_risk::SymbolRiskParams;

fn default_config() -> ShardConfig {
    ShardConfig {
        shard_id: 0,
        shard_count: 2,
        max_symbols: 4,
        symbol_params: vec![
            Some(SymbolRiskParams {
                initial_margin_rate: 1000,
                maintenance_margin_rate: 500,
                max_leverage: 10,
                min_notional: 1,
            });
            4
        ],
        taker_fee_bps: vec![5; 4],
        maker_fee_bps: vec![-1; 4],
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig::default(),
    }
}

fn make_shard() -> RiskShard {
    RiskShard::new(default_config())
}

fn set_funding(s: &mut RiskShard) {
    s.funding_interval_secs.fill(28_800);
    s.funding_min_bps.fill(-100);
    s.funding_max_bps.fill(100);
}

fn fill(taker: u32, maker: u32, sym: u32, price: i64, qty: i64, seq: u64) -> FillEvent {
    FillEvent {
        seq,
        symbol_id: sym,
        taker_user_id: taker,
        maker_user_id: maker,
        price,
        qty,
        taker_side: 0,
        timestamp_ns: 0,
    }
}

fn order(user_id: u32, symbol_id: u32, price: i64, qty: i64) -> OrderRequest {
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

// --- Fill ingestion ---

#[test]
fn fill_for_shard_user_updates_position() {
    let mut s = make_shard();
    // user 0 is in shard 0 (0 % 2 == 0)
    s.accounts.insert(0, Account::new(0, 1_000_000));
    let f = fill(0, 1, 0, 100, 10, 1);
    s.process_fill(&f);
    let pos = &s.positions[&(0, 0)];
    assert_eq!(pos.long_qty, 10);
}

#[test]
fn fill_for_other_shard_ignored() {
    let mut s = make_shard();
    // user 1 is in shard 1 (1 % 2 == 1), not shard 0
    // user 3 is in shard 1 (3 % 2 == 1)
    let f = fill(1, 3, 0, 100, 10, 1);
    s.process_fill(&f);
    assert!(s.positions.is_empty());
}

#[test]
fn fill_both_users_in_shard_updates_both() {
    let mut s = make_shard();
    // user 0 and 2 both in shard 0
    s.accounts.insert(0, Account::new(0, 1_000_000));
    s.accounts.insert(2, Account::new(2, 1_000_000));
    let f = fill(0, 2, 0, 100, 10, 1);
    s.process_fill(&f);
    let taker = &s.positions[&(0, 0)];
    let maker = &s.positions[&(2, 0)];
    assert_eq!(taker.long_qty, 10); // taker bought
    assert_eq!(maker.short_qty, 10); // maker sold
}

#[test]
fn fill_dedup_by_seq() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000));
    let f = fill(0, 1, 0, 100, 10, 1);
    s.process_fill(&f);
    s.process_fill(&f); // duplicate
    assert_eq!(s.positions[&(0, 0)].long_qty, 10);
    assert_eq!(s.fills_processed, 1);
}

#[test]
fn fill_advances_tip_per_symbol() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000));
    s.process_fill(&fill(0, 1, 0, 100, 10, 5));
    s.process_fill(&fill(0, 1, 1, 200, 5, 3));
    assert_eq!(s.tips[0], 5);
    assert_eq!(s.tips[1], 3);
}

#[test]
fn tip_monotonic_never_decreases() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000));
    s.process_fill(&fill(0, 1, 0, 100, 10, 5));
    s.process_fill(&fill(0, 1, 0, 100, 10, 3));
    assert_eq!(s.tips[0], 5);
    assert_eq!(s.fills_processed, 1); // second ignored
}

#[test]
fn fill_taker_fee_deducted() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000));
    s.process_fill(&fill(0, 1, 0, 100, 10, 1));
    // fee = 10 * 100 * 5 / 10000 = 0 (floor)
    // With bigger numbers:
    let mut s2 = make_shard();
    s2.accounts.insert(0, Account::new(0, 1_000_000_000));
    s2.process_fill(&fill(0, 1, 0, 10_000, 10_000, 1));
    // fee = 10000*10000*5/10000 = 50000
    assert_eq!(s2.accounts[&0].collateral, 1_000_000_000 - 50_000);
}

#[test]
fn fill_maker_fee_deducted() {
    let mut s = make_shard();
    // user 2 in shard 0, as maker
    s.accounts.insert(2, Account::new(2, 1_000_000_000));
    s.process_fill(&fill(1, 2, 0, 10_000, 10_000, 1));
    // maker fee = 10000*10000*(-1)/10000 = -10000
    // deduct_fee(-10000) means collateral += 10000
    assert_eq!(s.accounts[&2].collateral, 1_000_000_000 + 10_000);
}

#[test]
fn fill_self_trade_same_user() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    // User 0 is both taker and maker
    let f = fill(0, 0, 0, 10_000, 100, 1);
    s.process_fill(&f);
    let pos = &s.positions[&(0, 0)];
    // taker buys 100, then maker sells 100 -> net flat
    assert_eq!(pos.net_qty(), 0);
}

// --- Order processing ---

#[test]
fn order_accepted_margin_sufficient() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;
    let request = order(0, 0, 10_000, 10);
    let resp = s.process_order(&request);
    assert!(matches!(resp, OrderResponse::Accepted { .. }));
}

#[test]
fn order_rejected_margin_insufficient() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1)); // tiny
    s.mark_prices[0] = 10_000;
    let request = order(0, 0, 10_000, 100);
    let resp = s.process_order(&request);
    assert!(matches!(
        resp,
        OrderResponse::Rejected {
            reason: RejectReason::InsufficientMargin,
            ..
        }
    ));
}

#[test]
fn order_nonpositive_price_or_qty_rejected() {
    // RISK-NO-PRICE-QTY-GUARD: well-funded account, but a
    // negative/zero price or qty must be rejected BEFORE the
    // margin math — otherwise notional=price*qty goes negative
    // and the freeze *adds* available margin (solvency hole).
    let cases = [
        (10_000, 0),    // zero qty
        (10_000, -10),  // negative qty
        (0, 10),        // zero price
        (-10_000, 10),  // negative price
        (-10_000, -10), // both negative (positive notional!)
    ];
    for (price, qty) in cases {
        let mut s = make_shard();
        s.accounts.insert(0, Account::new(0, 1_000_000_000));
        s.mark_prices[0] = 10_000;
        let request = order(0, 0, price, qty);
        let resp = s.process_order(&request);
        assert!(
            matches!(
                resp,
                OrderResponse::Rejected {
                    reason: RejectReason::InsufficientMargin,
                    ..
                }
            ),
            "price={price} qty={qty} should be rejected, got {resp:?}"
        );
        // No freeze leaked from a rejected order.
        assert_eq!(s.frozen_for_user(0), 0);
    }
}

#[test]
fn order_not_in_shard_rejected() {
    let mut s = make_shard();
    let request = order(1, 0, 10_000, 10); // user 1 not in shard 0
    let resp = s.process_order(&request);
    assert!(matches!(
        resp,
        OrderResponse::Rejected {
            reason: RejectReason::NotInShard,
            ..
        }
    ));
}

#[test]
fn order_reduce_only_accepted() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1)); // tiny
    s.mark_prices[0] = 10_000;
    let mut request = order(0, 0, 10_000, 100);
    request.reduce_only = true;
    let resp = s.process_order(&request);
    assert!(matches!(resp, OrderResponse::Accepted { .. }));
}

#[test]
fn liquidation_order_accepted() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1)); // tiny
    s.mark_prices[0] = 10_000;
    let mut request = order(0, 0, 10_000, 100);
    request.is_liquidation = true;
    let resp = s.process_order(&request);
    assert!(matches!(resp, OrderResponse::Accepted { .. }));
}

// --- BBO + mark ---

#[test]
fn bbo_updates_index_price() {
    let mut s = make_shard();
    let bbo = BboUpdate {
        seq: 1,
        symbol_id: 0,
        bid_px: 9900,
        bid_qty: 100,
        ask_px: 10100,
        ask_qty: 100,
    };
    s.process_bbo(&bbo);
    assert_eq!(s.index_prices[0].price, 10_000);
    assert!(s.index_prices[0].valid);
}

#[test]
fn mark_update_stored() {
    let mut s = make_shard();
    s.update_mark(1, 50_000);
    assert_eq!(s.mark_prices[1], 50_000);
}

#[test]
fn missing_mark_falls_back_to_index_for_liquidation() {
    let mut s = make_shard();
    // Create an index price via BBO
    let bbo = BboUpdate {
        seq: 1,
        symbol_id: 0,
        bid_px: 10_000,
        bid_qty: 10,
        ask_px: 10_000,
        ask_qty: 10,
    };
    s.process_bbo(&bbo);
    // Mark missing for symbol 0
    s.mark_prices[0] = 0;
    // Create position for user 0
    s.process_fill(&fill(0, 2, 0, 10_000, 10, 1));
    // Collateral is small; with index fallback, should be in liquidation
    let request = order(0, 0, 10_000, 1);
    let resp = s.process_order(&request);
    assert!(matches!(
        resp,
        OrderResponse::Rejected {
            reason: RejectReason::UserInLiquidation,
            ..
        }
    ));
}

#[test]
fn stash_bbo_keeps_latest() {
    let mut s = make_shard();
    s.stash_bbo(BboUpdate {
        seq: 1,
        symbol_id: 0,
        bid_px: 100,
        bid_qty: 10,
        ask_px: 200,
        ask_qty: 10,
    });
    s.stash_bbo(BboUpdate {
        seq: 1,
        symbol_id: 0,
        bid_px: 300,
        bid_qty: 10,
        ask_px: 400,
        ask_qty: 10,
    });
    s.drain_stashed_bbos(0);
    // Second BBO wins: mid = (300*10 + 400*10) / 20 = 350
    assert_eq!(s.index_prices[0].price, 350);
}

// --- Funding + loop ---

#[test]
fn funding_zero_sum_across_users_uneven_split() {
    // Invariant #9: Σ applied funding per symbol == 0 even
    // for an uneven position split [+1,+1,+1,-3]. With odd
    // qtys/rate, per-user rounding would leak; settle_symbol
    // parks the residual so the column sums to zero exactly.
    let mut cfg = default_config();
    // Zero fees so collateral deltas isolate funding only.
    cfg.taker_fee_bps = vec![0; 4];
    cfg.maker_fee_bps = vec![0; 4];
    let mut s = RiskShard::new(cfg);
    set_funding(&mut s);
    // Shard-0 users: 0, 2, 4 (long 1); 6 (short 3).
    for u in [0u32, 2, 4, 6] {
        s.accounts.insert(u, Account::new(u, 1_000_000_000));
    }
    let mark = 1000;
    let mut seq = 0u64;
    let mut buy = |s: &mut RiskShard, u: u32, qty: i64| {
        seq += 1;
        // taker=u (in shard), maker=1 (shard 1, ignored).
        let mut f = fill(u, 1, 0, mark, qty, seq);
        f.taker_side = 0; // buy -> long
        s.process_fill(&f);
    };
    buy(&mut s, 0, 1);
    buy(&mut s, 2, 1);
    buy(&mut s, 4, 1);
    seq += 1;
    let mut sell = fill(6, 1, 0, mark, 3, seq);
    sell.taker_side = 1; // sell -> short
    s.process_fill(&sell);
    // Diverge mark from index so the rate is non-zero.
    s.mark_prices[0] = mark + 33;
    s.index_prices[0].price = mark;
    s.index_prices[0].valid = true;
    let before: i64 = [0u32, 2, 4, 6]
        .iter()
        .map(|u| s.accounts[u].collateral)
        .sum();
    s.maybe_settle_funding(28_800);
    let after: i64 = [0u32, 2, 4, 6]
        .iter()
        .map(|u| s.accounts[u].collateral)
        .sum();
    // Σ collateral delta == -Σ payments == 0.
    assert_eq!(before, after, "funding must be zero-sum per symbol");
    assert_eq!(s.funding_settled[0], Some(1));
}

#[test]
fn funding_settles_at_interval() {
    let mut s = make_shard();
    set_funding(&mut s);
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_100;
    s.index_prices[0].price = 10_000;
    s.index_prices[0].valid = true;
    // Give user a position
    s.process_fill(&fill(0, 1, 0, 10_000, 100, 1));
    let before = s.accounts[&0].collateral;
    // Settle at interval 1 (28800 secs)
    s.maybe_settle_funding(28_800);
    assert_eq!(s.funding_settled[0], Some(1));
    let after = s.accounts[&0].collateral;
    assert_ne!(before, after); // funding was applied
}

fn make_rings() -> (
    rsx_risk::ShardRings,
    rtrb::Consumer<OrderResponse>,
    rtrb::Consumer<MeCommand>,
) {
    let (resp_p, resp_c) = rtrb::RingBuffer::<OrderResponse>::new(4);
    let (me_p, me_c) = rtrb::RingBuffer::<MeCommand>::new(4);
    let rings = rsx_risk::ShardRings {
        response_producer: resp_p,
        me_producer: me_p,
    };
    (rings, resp_c, me_c)
}

fn pop_order(me_c: &mut rtrb::Consumer<MeCommand>, what: &str) -> OrderRequest {
    match me_c.pop() {
        Ok(MeCommand::Order(order, _)) => order,
        other => panic!("{what}: expected an order, got {other:?}"),
    }
}

fn drain_cancels(me_c: &mut rtrb::Consumer<MeCommand>) -> Vec<CancelRequest> {
    let mut cancels = Vec::new();
    while let Ok(command) = me_c.pop() {
        if let MeCommand::Cancel(cancel) = command {
            cancels.push(cancel);
        }
    }
    cancels
}

#[test]
fn tick_empty_no_crash() {
    let mut s = make_shard();
    let (mut rings, _resp_c, _me_c) = make_rings();
    s.tick(&mut rings, 0);
    assert_eq!(s.fills_processed, 0);
}

#[test]
fn fills_processed_before_orders() {
    // The fills-before-orders invariant now holds at the main
    // loop level (me_receiver block runs before gw_receiver
    // block), not inside `tick`. This test asserts the invariant
    // directly: process the fill first, then the order, and
    // confirm the order's pre-trade margin check sees the
    // position the fill created (no crash, order accepted with
    // the freed/updated margin state).
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;

    // Fills drained first.
    s.process_fill(&fill(0, 1, 0, 10_000, 100, 1));
    assert_eq!(s.fills_processed, 1);
    assert_eq!(s.positions[&(0, 0)].long_qty, 100);

    // Then the order's margin check runs against post-fill state.
    let resp = s.process_order(&order(0, 0, 10_000, 10));
    assert_eq!(s.orders_processed, 1);
    assert!(matches!(resp, OrderResponse::Accepted { .. }));
}

// --- Phase 2 fill edge cases ---

#[test]
fn fill_seq_gap_still_advances_tip() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000));
    // seq 1, then skip to seq 5
    s.process_fill(&fill(0, 1, 0, 100, 10, 1));
    s.process_fill(&fill(0, 1, 0, 100, 10, 5));
    assert_eq!(s.tips[0], 5);
    assert_eq!(s.fills_processed, 2);
}

#[test]
fn fill_seq_zero_first_ever() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000));
    // tips start at 0, seq 0 should be skipped (<=)
    s.process_fill(&fill(0, 1, 0, 100, 10, 0));
    assert_eq!(s.fills_processed, 0);
    // seq 1 should work
    s.process_fill(&fill(0, 1, 0, 100, 10, 1));
    assert_eq!(s.fills_processed, 1);
    assert_eq!(s.tips[0], 1);
}

#[test]
fn fill_for_unknown_symbol_advances_tip_only() {
    let mut s = make_shard();
    // Neither user in shard (1%2=1, 3%2=1)
    let f = fill(1, 3, 2, 100, 10, 1);
    s.process_fill(&f);
    // Tip should still advance even if no users in shard
    assert_eq!(s.tips[2], 1);
    assert_eq!(s.fills_processed, 1);
    assert!(s.positions.is_empty());
}

#[test]
fn fill_taker_in_shard_maker_not() {
    let mut s = make_shard();
    // taker=0 (0%2=0, in shard), maker=1 (1%2=1, not)
    s.accounts.insert(0, Account::new(0, 1_000_000));
    s.process_fill(&fill(0, 1, 0, 100, 10, 1));
    assert!(s.positions.contains_key(&(0, 0)));
    assert!(!s.positions.contains_key(&(1, 0)));
}

#[test]
fn fill_maker_in_shard_taker_not() {
    let mut s = make_shard();
    // taker=1 (not in shard), maker=2 (2%2=0, in shard)
    s.accounts.insert(2, Account::new(2, 1_000_000));
    s.process_fill(&fill(1, 2, 0, 100, 10, 1));
    assert!(!s.positions.contains_key(&(1, 0)));
    assert!(s.positions.contains_key(&(2, 0)));
    assert_eq!(s.positions[&(2, 0)].short_qty, 10);
}

#[test]
fn fill_rapid_sequence_same_symbol() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000));
    for seq in 1..=100u64 {
        s.process_fill(&fill(0, 1, 0, 100, 1, seq));
    }
    assert_eq!(s.fills_processed, 100);
    assert_eq!(s.positions[&(0, 0)].long_qty, 100);
    assert_eq!(s.tips[0], 100);
}

#[test]
fn fill_interleaved_symbols() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000));
    // Interleave fills across symbols 0, 1, 2
    for i in 1..=30u64 {
        let sym = ((i - 1) % 3) as u32;
        let seq = i.div_ceil(3); // 1,1,1,2,2,2,...
        s.process_fill(&fill(0, 1, sym, 100, 1, seq));
    }
    assert_eq!(s.positions[&(0, 0)].long_qty, 10);
    assert_eq!(s.positions[&(0, 1)].long_qty, 10);
    assert_eq!(s.positions[&(0, 2)].long_qty, 10);
    assert_eq!(s.tips[0], 10);
    assert_eq!(s.tips[1], 10);
    assert_eq!(s.tips[2], 10);
}

#[test]
fn tip_not_advanced_on_duplicate_fill() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000));
    s.process_fill(&fill(0, 1, 0, 100, 10, 5));
    assert_eq!(s.tips[0], 5);
    // Replay same seq
    s.process_fill(&fill(0, 1, 0, 100, 10, 5));
    assert_eq!(s.tips[0], 5);
    assert_eq!(s.fills_processed, 1);
}

fn symbol_config(
    symbol_id: u32,
    version: u64,
    taker_fee_bps: i64,
    maker_fee_bps: i64,
) -> SymbolRiskConfig {
    SymbolRiskConfig {
        symbol_id,
        config_version: version,
        params: SymbolRiskParams {
            initial_margin_rate: 2000,
            maintenance_margin_rate: 1000,
            max_leverage: 5,
            min_notional: 1,
        },
        taker_fee_bps,
        maker_fee_bps,
        tick_size: 1,
        funding_interval_secs: 28_800,
        funding_min_bps: -75,
        funding_max_bps: 75,
    }
}

#[test]
fn process_config_applied_requests_reload_of_newer_versions() {
    let mut s = make_shard();
    let (prod, mut cons) = rtrb::RingBuffer::<PersistEvent>::new(8);
    s.set_persist_producer(prod);
    assert_eq!(s.config_versions[0], 0);
    s.process_config_applied(0, 5);
    assert!(matches!(
        cons.pop(),
        Ok(PersistEvent::ConfigReload {
            symbol_id: 0,
            config_version: 5
        })
    ));
    s.apply_symbol_config(&symbol_config(0, 5, 11, -2));
    assert_eq!(s.config_versions[0], 5);
    // Applied, same, or older versions request nothing
    s.process_config_applied(0, 5);
    s.process_config_applied(0, 4);
    assert!(cons.pop().is_err());
    // Out of range symbol is a no-op
    s.process_config_applied(999, 1);
    assert!(cons.pop().is_err());
}

#[test]
fn fill_row_is_persisted_only_by_the_takers_shard() {
    let mut s = make_shard();
    let (prod, mut cons) = rtrb::RingBuffer::<PersistEvent>::new(16);
    s.set_persist_producer(prod);
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    // Shard 0 owns the maker only: no fill row.
    s.process_fill(&fill(1, 0, 0, 10_000, 10_000, 1));
    while let Ok(event) = cons.pop() {
        assert!(!matches!(event, PersistEvent::Fill(_)));
    }
    // Shard 0 owns the taker: one fill row carrying both fees.
    s.process_fill(&fill(0, 1, 0, 10_000, 10_000, 2));
    let mut rows = Vec::new();
    while let Ok(event) = cons.pop() {
        if let PersistEvent::Fill(row) = event {
            rows.push(row);
        }
    }
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].taker_fee, 50_000);
    assert_eq!(rows[0].maker_fee, -10_000);
}

// --- Config tests ---

#[test]
fn applied_symbol_config_sets_fees_margin_and_version() {
    let mut s = make_shard();
    s.apply_symbol_config(&symbol_config(0, 1, 11, -2));
    assert_eq!(s.config_versions[0], 1);
    let f = fill(0, 2, 0, 10_000, 10_000, 1);
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.accounts.insert(2, Account::new(2, 1_000_000_000));
    s.process_fill(&f);
    // taker fee = 10000*10000*11/10000 = 110000
    assert_eq!(s.accounts[&0].collateral, 1_000_000_000 - 110_000);
    // maker rebate = 10000*10000*2/10000 = 20000
    assert_eq!(s.accounts[&2].collateral, 1_000_000_000 + 20_000);
    // Other symbols unaffected
    assert_eq!(s.config_versions[2], 0);
    // Beyond max_symbols is ignored
    s.apply_symbol_config(&symbol_config(999, 1, 0, 0));
}

#[test]
fn tick_drains_config_updates_before_other_work() {
    let mut s = make_shard();
    let (mut prod, cons) = rtrb::RingBuffer::<SymbolRiskConfig>::new(4);
    s.set_config_consumer(cons);
    prod.push(symbol_config(1, 7, 3, -1)).unwrap();
    let (mut rings, _resp_c, _me_c) = make_rings();
    s.tick(&mut rings, 0);
    assert_eq!(s.config_versions[1], 7);
    assert_eq!(s.config_versions[0], 0);
}

#[test]
fn order_on_unconfigured_symbol_rejected() {
    let mut config = default_config();
    config.symbol_params[1] = None;
    let mut s = RiskShard::new(config);
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;
    s.mark_prices[1] = 10_000;
    assert!(matches!(
        s.process_order(&order(0, 1, 10_000, 10)),
        OrderResponse::Rejected {
            reason: RejectReason::SymbolNotConfigured,
            ..
        }
    ));
    assert!(matches!(
        s.process_order(&order(0, 0, 10_000, 10)),
        OrderResponse::Accepted { .. }
    ));
}

fn symbol_config_min(symbol_id: u32, version: u64, min_notional: i64) -> SymbolRiskConfig {
    let mut config = symbol_config(symbol_id, version, 5, -1);
    config.params.min_notional = min_notional;
    config.params.max_leverage = 10;
    config.params.initial_margin_rate = 1000;
    config.params.maintenance_margin_rate = 500;
    config
}

fn order_with_id(user_id: u32, price: i64, qty: i64, order_id_lo: u64) -> OrderRequest {
    let mut request = order(user_id, 0, price, qty);
    request.order_id_lo = order_id_lo;
    request
}

fn is_too_small(resp: &OrderResponse) -> bool {
    matches!(
        resp,
        OrderResponse::Rejected {
            reason: RejectReason::OrderTooSmall,
            ..
        }
    )
}

#[test]
fn order_below_minimum_notional_rejected_without_a_freeze() {
    let mut s = make_shard();
    s.apply_symbol_config(&symbol_config_min(0, 1, 100_001));
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;
    let resp = s.process_order(&order_with_id(0, 10_000, 10, 1));
    assert!(is_too_small(&resp));
    assert_eq!(s.frozen_for_user(0), 0);
    assert_eq!(s.orders_processed, 0);
}

#[test]
fn config_reload_changes_the_minimum_for_new_orders_only() {
    let mut s = make_shard();
    s.apply_symbol_config(&symbol_config_min(0, 1, 100_000));
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;

    // Exactly at the minimum: accepted and frozen.
    let resting = s.process_order(&order_with_id(0, 10_000, 10, 1));
    assert!(matches!(resting, OrderResponse::Accepted { .. }));
    let frozen = s.frozen_for_user(0);
    assert!(frozen > 0);

    // The minimum rises by one raw unit: the same order is refused,
    // the resting one keeps its freeze.
    s.apply_symbol_config(&symbol_config_min(0, 2, 100_001));
    assert!(is_too_small(
        &s.process_order(&order_with_id(0, 10_000, 10, 2))
    ));
    assert_eq!(s.frozen_for_user(0), frozen);

    // The minimum falls back: the order is accepted again.
    s.apply_symbol_config(&symbol_config_min(0, 3, 100_000));
    assert!(matches!(
        s.process_order(&order_with_id(0, 10_000, 10, 3)),
        OrderResponse::Accepted { .. }
    ));
    assert_eq!(s.frozen_for_user(0), 2 * frozen);
}

#[test]
fn config_reload_over_the_ring_changes_the_minimum() {
    let mut s = make_shard();
    s.apply_symbol_config(&symbol_config_min(0, 1, 1));
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;
    let (mut prod, cons) = rtrb::RingBuffer::<SymbolRiskConfig>::new(4);
    s.set_config_consumer(cons);
    prod.push(symbol_config_min(0, 2, 100_001)).unwrap();
    let (mut rings, _resp_c, _me_c) = make_rings();
    s.tick(&mut rings, 0);
    assert!(is_too_small(
        &s.process_order(&order_with_id(0, 10_000, 10, 1))
    ));
}

#[test]
fn reduce_only_closing_whole_position_is_exempt_at_any_size() {
    let mut s = make_shard();
    s.apply_symbol_config(&symbol_config_min(0, 1, 1_000_000));
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;
    s.process_fill(&fill(0, 1, 0, 10_000, 10, 1));
    assert_eq!(s.positions[&(0, 0)].net_qty(), 10);

    let mut partial = order_with_id(0, 10_000, 9, 1);
    partial.side = 1;
    partial.reduce_only = true;
    assert!(is_too_small(&s.process_order(&partial)));

    let mut whole = order_with_id(0, 10_000, 10, 2);
    whole.side = 1;
    whole.reduce_only = true;
    assert!(matches!(
        s.process_order(&whole),
        OrderResponse::Accepted {
            margin_reserved: 0,
            ..
        }
    ));

    let mut partial_ioc = partial;
    partial_ioc.order_id_lo = 3;
    partial_ioc.tif = 1;
    assert!(matches!(
        s.process_order(&partial_ioc),
        OrderResponse::Accepted { .. }
    ));
}

// --- Frozen margin release ---

#[test]
fn partial_fill_releases_remaining_frozen() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;
    let mut request = order(0, 0, 10_000, 10);
    request.order_id_hi = 0;
    request.order_id_lo = 42;
    let resp = s.process_order(&request);
    assert!(matches!(resp, OrderResponse::Accepted { .. }));
    let frozen_before = s.frozen_for_user(0);
    assert!(frozen_before > 0);
    // Partial fill of 6 out of 10
    s.process_fill(&fill(0, 1, 0, 10_000, 6, 1));
    // Order done -> release remaining frozen
    s.release_frozen_for_order(0, 0, 42);
    assert_eq!(s.frozen_for_user(0), 0);
}

// --- Liquidation integration in shard ---

#[test]
fn order_while_user_being_liquidated_rejected() {
    let mut s = make_shard();
    // User 0 in shard 0. Tiny collateral, big position.
    s.accounts.insert(0, Account::new(0, 100));
    s.mark_prices[0] = 10_000;
    // Create a position that puts user underwater
    // (long 100 @ 10000, mark drops to 10000, mm=5%)
    // notional = 100*10000 = 1_000_000
    // mm = 1_000_000 * 500/10000 = 50_000
    // equity = 100 + upnl. upnl = 100*(10000-10000)=0
    // equity(100) < mm(50000) -> liquidation
    s.process_fill(&fill(0, 1, 0, 10_000, 100, 1));
    // After fill, check_liquidation_for enqueues user
    assert!(s.liquidation.is_in_liquidation(0, 0));
    // Non-liquidation order should be rejected
    let request = order(0, 0, 10_000, 1);
    let resp = s.process_order(&request);
    assert!(matches!(
        resp,
        OrderResponse::Rejected {
            reason: RejectReason::UserInLiquidation,
            ..
        }
    ));
}

#[test]
fn liquidation_enqueued_via_index_when_mark_silent() {
    // FIX 2: check_liquidation_for must price off the
    // fallback (mark, else index). A SHORT at entry 100 looks
    // *profitable* at mark==0 (the old, raw-mark code would
    // not liquidate), but a valid index of 10_000 from BBO
    // makes it deeply underwater. Liquidation must not be
    // inert when the mark feed is silent — it must use the
    // index fallback, the same source as the pre-trade check.
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 100)); // tiny
                                                // No mark for symbol 0 (silent feed).
    s.mark_prices[0] = 0;
    // Valid index via BBO (mid = 10_000), far above entry.
    s.process_bbo(&BboUpdate {
        seq: 1,
        symbol_id: 0,
        bid_px: 10_000,
        bid_qty: 10,
        ask_px: 10_000,
        ask_qty: 10,
    });
    // User 0 SELLS 100 @ 100 (taker_side=1 -> short).
    let mut f = fill(0, 1, 0, 100, 100, 1);
    f.taker_side = 1;
    s.process_fill(&f); // triggers check_liquidation_for(0)
                        // Sanity: at the silent mark (0) the short is in profit,
                        // so only the index fallback exposes the loss.
    assert!(
        s.liquidation.is_in_liquidation(0, 0),
        "short underwater-by-index must enqueue when mark==0"
    );
}

// --- Liquidation orders (LIQUIDATOR.md §10) ---

fn liquidating_shard(tick_size: i64, side: u8, mark: i64) -> RiskShard {
    let mut s = make_shard();
    let mut config = symbol_config(0, 1, 5, -1);
    config.tick_size = tick_size;
    s.apply_symbol_config(&config);
    s.accounts.insert(0, Account::new(0, 100));
    s.mark_prices[0] = mark;
    let mut opening = fill(0, 1, 0, 10_000, 100, 1);
    opening.taker_side = side;
    s.process_fill(&opening);
    assert!(s.liquidation.is_in_liquidation(0, 0));
    s
}

#[test]
fn consecutive_liquidation_rounds_get_distinct_order_ids() {
    let mut s = liquidating_shard(1, 0, 10_000);
    let (mut rings, _resp_c, mut me_c) = make_rings();
    s.tick(&mut rings, 0);
    s.tick(&mut rings, 1);
    let first = pop_order(&mut me_c, "round 1");
    let second = pop_order(&mut me_c, "round 2");
    assert_ne!((first.order_id_hi, first.order_id_lo), (0, 0));
    assert_ne!(
        (first.order_id_hi, first.order_id_lo),
        (second.order_id_hi, second.order_id_lo)
    );
}

#[test]
fn liquidation_sell_price_floors_to_tick() {
    let mut s = liquidating_shard(50, 0, 10_030);
    let (mut rings, _resp_c, mut me_c) = make_rings();
    s.tick(&mut rings, 0);
    let order = pop_order(&mut me_c, "sell");
    assert_eq!(order.side, 1);
    assert_eq!(order.price, 10_000);
}

#[test]
fn liquidation_buy_price_ceils_to_tick() {
    let mut s = liquidating_shard(50, 1, 10_030);
    let (mut rings, _resp_c, mut me_c) = make_rings();
    s.tick(&mut rings, 0);
    let order = pop_order(&mut me_c, "buy");
    assert_eq!(order.side, 0);
    assert_eq!(order.price, 10_050);
}

#[test]
fn rejected_liquidation_order_leaves_next_round_due() {
    let mut s = liquidating_shard(1, 0, 10_000);
    let (mut rings, _resp_c, mut me_c) = make_rings();
    s.tick(&mut rings, 0);
    let failed = pop_order(&mut me_c, "round 1");
    s.release_frozen_for_order(failed.user_id, failed.order_id_hi, failed.order_id_lo);
    s.tick(&mut rings, 1);
    let retry = pop_order(&mut me_c, "round 2");
    assert!(retry.price < failed.price);
}

#[test]
fn each_liquidation_round_notifies_the_user() {
    let mut s = liquidating_shard(1, 0, 10_000);
    let (mut rings, mut resp_c, mut me_c) = make_rings();
    s.tick(&mut rings, 0);
    s.tick(&mut rings, 1);
    let first = pop_order(&mut me_c, "round 1");
    let second = pop_order(&mut me_c, "round 2");
    let notice = |order: &OrderRequest, round: u32, slip_bps: i64| OrderResponse::Liquidation {
        user_id: 0,
        symbol_id: 0,
        status: 0,
        round,
        side: order.side,
        qty: order.qty,
        price: order.price,
        slip_bps,
    };
    assert_eq!(resp_c.pop().unwrap(), notice(&first, 1, 1));
    assert_eq!(resp_c.pop().unwrap(), notice(&second, 2, 4));
    assert!(resp_c.pop().is_err());
}

// --- Entering liquidation cancels margin-reserving orders (LIQUIDATOR.md §4) ---

fn two_symbol_shard_with_resting_orders() -> RiskShard {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 2_000));
    s.update_mark(0, 100);
    s.update_mark(1, 100);
    for (sid, order_id_lo) in [(0, 1), (1, 2)] {
        let mut resting = order(0, sid, 100, 10);
        resting.order_id_lo = order_id_lo;
        assert!(matches!(
            s.process_order(&resting),
            OrderResponse::Accepted { .. }
        ));
    }
    let mut reduce_only = order(0, 0, 100, 10);
    reduce_only.order_id_lo = 3;
    reduce_only.reduce_only = true;
    assert!(matches!(
        s.process_order(&reduce_only),
        OrderResponse::Accepted { .. }
    ));
    s.process_fill(&fill(0, 1, 0, 100, 100, 1));
    s.process_fill(&fill(0, 1, 1, 100, 100, 1));
    assert!(!s.liquidation.is_in_liquidation(0, 0));
    s
}

fn bbo(symbol_id: u32, px: i64) -> BboUpdate {
    BboUpdate {
        seq: 1,
        symbol_id,
        bid_px: px,
        bid_qty: 10,
        ask_px: px,
        ask_qty: 10,
    }
}

#[test]
fn entering_liquidation_cancels_each_margin_reserving_order_once() {
    let mut s = two_symbol_shard_with_resting_orders();
    let (mut rings, _resp_c, mut me_c) = make_rings();
    s.update_mark(0, 80);
    s.update_mark(1, 80);
    s.stash_bbo(bbo(0, 80));
    s.tick(&mut rings, 0);
    assert!(s.liquidation.is_in_liquidation(0, 0));
    s.tick(&mut rings, 0);
    let mut cancels: Vec<(u32, u64)> = drain_cancels(&mut me_c)
        .iter()
        .map(|cancel| (cancel.symbol_id, cancel.order_id_lo))
        .collect();
    cancels.sort();
    assert_eq!(cancels, vec![(0, 1), (1, 2)]);

    s.stash_bbo(bbo(1, 80));
    s.tick(&mut rings, 0);
    s.tick(&mut rings, 0);
    assert!(drain_cancels(&mut me_c).is_empty());
}

#[test]
fn released_order_is_not_cancelled_on_liquidation_entry() {
    let mut s = two_symbol_shard_with_resting_orders();
    let (mut rings, _resp_c, mut me_c) = make_rings();
    s.update_mark(0, 80);
    s.update_mark(1, 80);
    s.stash_bbo(bbo(0, 80));
    s.tick(&mut rings, 0);
    s.release_frozen_for_order(0, 0, 1);
    s.tick(&mut rings, 0);
    let cancels = drain_cancels(&mut me_c);
    assert_eq!(cancels.len(), 1);
    assert_eq!((cancels[0].symbol_id, cancels[0].order_id_lo), (1, 2));
}

// --- Backpressure: positions == sum(fills) under flood ---

#[test]
fn fill_flood_position_matches_sum_of_fills() {
    // Mirrors the main-loop pattern: the recv handler calls
    // process_fill directly (no input ring), then tick runs the
    // periodic work + egress backpressure. Verifies position
    // state equals the sum of all qty fed in, with no fill lost.
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000_000));

    let (mut rings, _resp_c, _me_c) = make_rings();

    // 100 fills of 7 each = expected long_qty 700.
    let mut delivered: i64 = 0;
    let total_fills = 100u64;
    for seq in 1..=total_fills {
        s.process_fill(&fill(0, 1, 0, 100, 7, seq));
        s.tick(&mut rings, 0);
        delivered += 7;
    }

    let pos = &s.positions[&(0, 0)];
    assert_eq!(
        pos.long_qty, delivered,
        "INVARIANT: position == sum(fills); \
         lost fills under backpressure"
    );
    assert_eq!(
        s.fills_processed, total_fills,
        "every fill must be processed exactly once",
    );
}

#[test]
fn release_frozen_owner_mismatch_keeps_freeze() {
    // An order-lifecycle record whose user_id differs from the
    // freeze owner (impossible-state signal) must NOT release the
    // owner's margin: removing the primary while the owner's
    // index still counts it would permanently over-reserve.
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;
    let mut request = order(0, 0, 10_000, 10);
    request.order_id_lo = 42;
    assert!(matches!(
        s.process_order(&request),
        OrderResponse::Accepted { .. }
    ));
    let frozen = s.frozen_for_user(0);
    assert!(frozen > 0);
    // User 2 (also in shard 0) claims user 0's order id.
    s.release_frozen_for_order(2, 0, 42);
    assert_eq!(
        s.frozen_for_user(0),
        frozen,
        "mismatched release must keep the owner's freeze"
    );
    // The true owner can still release it afterwards.
    s.release_frozen_for_order(0, 0, 42);
    assert_eq!(s.frozen_for_user(0), 0);
}

// --- Persist ring: stall, never drop (WAL.md hard backpressure) ---

#[test]
fn push_persist_stalls_until_drained_never_drops() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000));
    s.accounts.insert(2, Account::new(2, 1_000_000));
    // Tiny ring: one both-sides fill emits 6 events (fill, 2x
    // position, 2x account, tip) and MUST overflow a cap-4 ring.
    // The producer has to stall until the drainer thread frees
    // slots — a drop would leave a permanent hole in PG that
    // replay-from-tip can never re-cover.
    let (prod, mut cons) = rtrb::RingBuffer::<rsx_risk::persist::PersistEvent>::new(4);
    s.set_persist_producer(prod);
    let drainer = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut got = 0u32;
        while got < 6 {
            match cons.pop() {
                Ok(_) => got += 1,
                Err(_) => {
                    if std::time::Instant::now() > deadline {
                        break;
                    }
                    std::thread::yield_now();
                }
            }
        }
        got
    });
    s.process_fill(&fill(0, 2, 0, 100, 10, 1));
    let got = drainer.join().unwrap();
    assert_eq!(
        got, 6,
        "every persist event must be delivered: ring-full stalls \
         the producer, never drops",
    );
    assert!(
        !s.backpressured,
        "persist pressure must not halt tick() (liquidation/funding)",
    );
}

#[test]
fn push_persist_gives_up_when_worker_gone() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1_000_000));
    s.accounts.insert(2, Account::new(2, 1_000_000));
    let (prod, cons) = rtrb::RingBuffer::<rsx_risk::persist::PersistEvent>::new(2);
    s.set_persist_producer(prod);
    // Worker died (circuit open): consumer side dropped.
    drop(cons);
    // Must neither spin forever nor halt the tick: fills the 2
    // slots, detects the abandoned consumer, drops the rest
    // loudly. The live loop demotes on worker death; replay from
    // the frozen tip re-covers the dropped events.
    s.process_fill(&fill(0, 2, 0, 100, 10, 1));
    assert_eq!(s.fills_processed, 1);
    assert!(!s.backpressured, "worker death must not halt tick()");
}

#[test]
fn persist_ring_used_reports_occupancy() {
    let mut s = make_shard();
    assert_eq!(s.persist_ring_used(), 0, "no producer attached");
    let (prod, _cons) = rtrb::RingBuffer::<rsx_risk::persist::PersistEvent>::new(8);
    s.set_persist_producer(prod);
    assert_eq!(s.persist_ring_used(), 0, "attached, empty");
    s.accounts.insert(0, Account::new(0, 1_000_000));
    s.accounts.insert(2, Account::new(2, 1_000_000));
    // One both-sides fill emits 6 events.
    s.process_fill(&fill(0, 2, 0, 100, 10, 1));
    assert_eq!(s.persist_ring_used(), 6, "occupancy visible to /metrics");
}

// --- Socialized loss (LIQUIDATOR.md §9) ---

fn socializing_shard() -> (RiskShard, rtrb::Consumer<PersistEvent>) {
    let mut config = default_config();
    config.liquidation_config = LiquidationConfig {
        max_rounds: 0,
        ..LiquidationConfig::default()
    };
    let mut s = RiskShard::new(config);
    let (prod, mut cons) = rtrb::RingBuffer::<PersistEvent>::new(64);
    s.set_persist_producer(prod);
    s.accounts.insert(0, Account::new(0, 100));
    s.mark_prices[0] = 9_999;
    s.process_bbo(&BboUpdate {
        seq: 1,
        symbol_id: 0,
        bid_px: 9_998,
        bid_qty: 100,
        ask_px: 10_000,
        ask_qty: 100,
    });
    s.maybe_settle_funding(0);
    assert_eq!(s.funding_settled[0], Some(0));
    s.process_fill(&fill(0, 1, 0, 10_000, 250, 1));
    assert!(s.liquidation.is_in_liquidation(0, 0));
    s.accounts.get_mut(&0).unwrap().collateral = 100;
    while cons.pop().is_ok() {}
    (s, cons)
}

fn drain_persist(cons: &mut rtrb::Consumer<PersistEvent>) -> Vec<PersistEvent> {
    let mut events = Vec::new();
    while let Ok(event) = cons.pop() {
        events.push(event);
    }
    events
}

#[test]
fn socialized_loss_moves_the_deficit_to_the_account() {
    let (mut s, mut cons) = socializing_shard();
    let (mut rings, _resp_c, _me_c) = make_rings();
    assert_eq!(s.positions[&(0, 0)].unrealized_pnl(9_999).unwrap(), -250);

    s.tick(&mut rings, 0);

    let collateral = s.accounts[&0].collateral;
    let fund = s.insurance_funds[&0].balance;
    assert_eq!(fund, -150);
    assert_eq!(collateral, 250);
    let upnl = s.positions[&(0, 0)].unrealized_pnl(9_999).unwrap();
    assert_eq!(collateral + upnl, 0, "equity is zero");
    assert_eq!(fund + collateral, 100, "the transfer is zero-sum");

    let events = drain_persist(&mut cons);
    let rows: Vec<u8> = events
        .iter()
        .filter_map(|event| match event {
            PersistEvent::Liquidation(row) => Some(row.status),
            _ => None,
        })
        .collect();
    assert_eq!(rows, vec![3]);
    assert!(events
        .iter()
        .any(|event| matches!(event, PersistEvent::Account(a) if a.collateral == 250)));
    assert!(events
        .iter()
        .any(|event| matches!(event, PersistEvent::InsuranceFund(f) if f.balance == -150)));
}

#[test]
fn socialized_loss_notifies_the_user_once() {
    let (mut s, _cons) = socializing_shard();
    let (mut rings, mut resp_c, _me_c) = make_rings();
    s.tick(&mut rings, 0);
    assert_eq!(
        resp_c.pop().unwrap(),
        OrderResponse::Liquidation {
            user_id: 0,
            symbol_id: 0,
            status: 3,
            round: 1,
            side: 1,
            qty: 250,
            price: 9_999,
            slip_bps: 0,
        }
    );
    assert!(resp_c.pop().is_err());
}

#[test]
fn socialized_loss_after_the_cooldown_moves_only_new_deficit() {
    let (mut s, mut cons) = socializing_shard();
    let (mut rings, _resp_c, _me_c) = make_rings();
    s.tick(&mut rings, 0);
    drain_persist(&mut cons);

    s.stash_bbo(BboUpdate {
        seq: 1,
        symbol_id: 0,
        bid_px: 9_900,
        bid_qty: 100,
        ask_px: 10_100,
        ask_qty: 100,
    });
    s.tick(&mut rings, 121);
    assert!(s.liquidation.is_in_liquidation(0, 0));
    s.tick(&mut rings, 122);

    assert_eq!(s.insurance_funds[&0].balance, -150);
    assert_eq!(s.accounts[&0].collateral, 250);
    assert!(drain_persist(&mut cons).is_empty());

    s.update_mark(0, 9_998);
    s.stash_bbo(BboUpdate {
        seq: 2,
        symbol_id: 0,
        bid_px: 9_900,
        bid_qty: 100,
        ask_px: 10_100,
        ask_qty: 100,
    });
    s.tick(&mut rings, 123);
    s.tick(&mut rings, 124);
    assert_eq!(s.insurance_funds[&0].balance, -150, "cooldown holds");

    s.stash_bbo(BboUpdate {
        seq: 3,
        symbol_id: 0,
        bid_px: 9_900,
        bid_qty: 100,
        ask_px: 10_100,
        ask_qty: 100,
    });
    s.tick(&mut rings, 245);
    s.tick(&mut rings, 246);
    assert_eq!(s.insurance_funds[&0].balance, -400);
    assert_eq!(s.accounts[&0].collateral, 500);
}

#[test]
fn socialized_loss_of_a_solvent_account_moves_nothing() {
    let (mut s, mut cons) = socializing_shard();
    s.accounts.get_mut(&0).unwrap().collateral = 1_000_000;
    let (mut rings, _resp_c, _me_c) = make_rings();

    s.tick(&mut rings, 0);

    assert!(!s.insurance_funds.contains_key(&0));
    assert_eq!(s.accounts[&0].collateral, 1_000_000);
    assert!(drain_persist(&mut cons).is_empty());
}
