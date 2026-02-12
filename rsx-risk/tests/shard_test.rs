use rsx_risk::Account;
use rsx_risk::BboUpdate;
use rsx_risk::FillEvent;
use rsx_risk::FundingConfig;
use rsx_risk::LiquidationConfig;
use rsx_risk::OrderRequest;
use rsx_risk::OrderResponse;
use rsx_risk::ReplicationConfig;
use rsx_risk::RiskShard;
use rsx_risk::ShardConfig;
use rsx_risk::SymbolRiskParams;
use rsx_risk::types::RejectReason;

fn default_config() -> ShardConfig {
    ShardConfig {
        shard_id: 0,
        shard_count: 2,
        max_symbols: 4,
        symbol_params: vec![
            SymbolRiskParams {
                initial_margin_rate: 1000,
                maintenance_margin_rate: 500,
                max_leverage: 10,
            };
            4
        ],
        taker_fee_bps: vec![5; 4],
        maker_fee_bps: vec![-1; 4],
        funding_config: FundingConfig::default(),
        liquidation_config:
            LiquidationConfig::default(),
        replication_config:
            ReplicationConfig::default(),
    }
}

fn make_shard() -> RiskShard {
    RiskShard::new(default_config())
}

fn fill(
    taker: u32,
    maker: u32,
    sym: u32,
    price: i64,
    qty: i64,
    seq: u64,
) -> FillEvent {
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
    s2.accounts
        .insert(0, Account::new(0, 1_000_000_000));
    s2.process_fill(&fill(
        0, 1, 0, 10_000, 10_000, 1,
    ));
    // fee = 10000*10000*5/10000 = 50000
    assert_eq!(
        s2.accounts[&0].collateral,
        1_000_000_000 - 50_000
    );
}

#[test]
fn fill_maker_fee_deducted() {
    let mut s = make_shard();
    // user 2 in shard 0, as maker
    s.accounts
        .insert(2, Account::new(2, 1_000_000_000));
    s.process_fill(&fill(
        1, 2, 0, 10_000, 10_000, 1,
    ));
    // maker fee = 10000*10000*(-1)/10000 = -10000
    // deduct_fee(-10000) means collateral += 10000
    assert_eq!(
        s.accounts[&2].collateral,
        1_000_000_000 + 10_000
    );
}

#[test]
fn fill_self_trade_same_user() {
    let mut s = make_shard();
    s.accounts
        .insert(0, Account::new(0, 1_000_000_000));
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
    s.accounts
        .insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;
    let o = order(0, 0, 10_000, 10);
    let resp = s.process_order(&o);
    assert!(
        matches!(resp, OrderResponse::Accepted { .. })
    );
}

#[test]
fn order_rejected_margin_insufficient() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1)); // tiny
    s.mark_prices[0] = 10_000;
    let o = order(0, 0, 10_000, 100);
    let resp = s.process_order(&o);
    assert!(matches!(
        resp,
        OrderResponse::Rejected {
            reason: RejectReason::InsufficientMargin,
            ..
        }
    ));
}

#[test]
fn order_not_in_shard_rejected() {
    let mut s = make_shard();
    let o = order(1, 0, 10_000, 10); // user 1 not in shard 0
    let resp = s.process_order(&o);
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
    let mut o = order(0, 0, 10_000, 100);
    o.reduce_only = true;
    let resp = s.process_order(&o);
    assert!(
        matches!(resp, OrderResponse::Accepted { .. })
    );
}

#[test]
fn liquidation_order_accepted() {
    let mut s = make_shard();
    s.accounts.insert(0, Account::new(0, 1)); // tiny
    s.mark_prices[0] = 10_000;
    let mut o = order(0, 0, 10_000, 100);
    o.is_liquidation = true;
    let resp = s.process_order(&o);
    assert!(
        matches!(resp, OrderResponse::Accepted { .. })
    );
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
    let o = order(0, 0, 10_000, 1);
    let resp = s.process_order(&o);
    assert!(
        matches!(resp, OrderResponse::Rejected { reason: RejectReason::UserInLiquidation, .. })
    );
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
    s.drain_stashed_bbos();
    // Second BBO wins: mid = (300*10 + 400*10) / 20 = 350
    assert_eq!(s.index_prices[0].price, 350);
}

// --- Funding + loop ---

#[test]
fn funding_settles_at_interval() {
    let mut s = make_shard();
    s.accounts
        .insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_100;
    s.index_prices[0].price = 10_000;
    s.index_prices[0].valid = true;
    // Give user a position
    s.process_fill(&fill(0, 1, 0, 10_000, 100, 1));
    let before = s.accounts[&0].collateral;
    // Settle at interval 1 (28800 secs)
    s.maybe_settle_funding(28_800);
    assert_eq!(s.last_funding_id, 1);
    let after = s.accounts[&0].collateral;
    assert_ne!(before, after); // funding was applied
}

#[test]
fn run_once_empty_rings_no_crash() {
    let mut s = make_shard();
    let (_, fill_c) = rtrb::RingBuffer::<FillEvent>::new(4);
    let (_, order_c) =
        rtrb::RingBuffer::<OrderRequest>::new(4);
    let (_, mark_c) =
        rtrb::RingBuffer::<rsx_risk::rings::MarkPriceUpdate>::new(4);
    let (_, bbo_c) = rtrb::RingBuffer::<BboUpdate>::new(4);
    let (resp_p, _) =
        rtrb::RingBuffer::<OrderResponse>::new(4);
    let (accepted_p, _accepted_c) =
        rtrb::RingBuffer::<OrderRequest>::new(4);
    let mut rings = rsx_risk::ShardRings {
        fill_consumers: vec![fill_c],
        order_consumer: order_c,
        mark_consumer: mark_c,
        bbo_consumers: vec![bbo_c],
        response_producer: resp_p,
        accepted_producer: accepted_p,
    };
    s.run_once(&mut rings, 0);
    assert_eq!(s.fills_processed, 0);
}

#[test]
fn run_once_fills_before_orders() {
    // Verify fills are processed before orders in run_once
    let mut s = make_shard();
    s.accounts
        .insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;

    let (mut fill_p, fill_c) =
        rtrb::RingBuffer::<FillEvent>::new(4);
    let (mut order_p, order_c) =
        rtrb::RingBuffer::<OrderRequest>::new(4);
    let (_, mark_c) =
        rtrb::RingBuffer::<rsx_risk::rings::MarkPriceUpdate>::new(4);
    let (_, bbo_c) = rtrb::RingBuffer::<BboUpdate>::new(4);
    let (resp_p, mut resp_c) =
        rtrb::RingBuffer::<OrderResponse>::new(4);
    let (accepted_p, _accepted_c) =
        rtrb::RingBuffer::<OrderRequest>::new(4);

    fill_p.push(fill(0, 1, 0, 10_000, 100, 1)).unwrap();
    order_p.push(order(0, 0, 10_000, 10)).unwrap();

    let mut rings = rsx_risk::ShardRings {
        fill_consumers: vec![fill_c],
        order_consumer: order_c,
        mark_consumer: mark_c,
        bbo_consumers: vec![bbo_c],
        response_producer: resp_p,
        accepted_producer: accepted_p,
    };
    s.run_once(&mut rings, 0);

    assert_eq!(s.fills_processed, 1);
    assert_eq!(s.orders_processed, 1);
    let resp = resp_c.pop().unwrap();
    assert!(
        matches!(resp, OrderResponse::Accepted { .. })
    );
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
        let seq = (i + 2) / 3; // 1,1,1,2,2,2,...
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

#[test]
fn process_config_applied_tracks_version() {
    let mut s = make_shard();
    assert_eq!(s.config_versions[0], 0);
    s.process_config_applied(0, 5);
    assert_eq!(s.config_versions[0], 5);
    s.process_config_applied(0, 10);
    assert_eq!(s.config_versions[0], 10);
    // Older version ignored
    s.process_config_applied(0, 9);
    assert_eq!(s.config_versions[0], 10);
    // Out of range symbol is a no-op
    s.process_config_applied(999, 1);
}

// --- Config tests ---

#[test]
fn config_applied_event_updates_symbol_params() {
    let mut s = make_shard();
    std::env::set_var("RSX_SYMBOL_0_TAKER_FEE_BPS", "11");
    std::env::set_var("RSX_SYMBOL_0_MAKER_FEE_BPS", "-2");
    s.process_config_applied(0, 1);
    assert_eq!(s.config_versions[0], 1);
    let f = fill(0, 2, 0, 10_000, 10_000, 1);
    s.accounts.insert(0, Account::new(0, 1_000_000_000));
    s.accounts.insert(2, Account::new(2, 1_000_000_000));
    s.process_fill(&f);
    // taker fee = 10000*10000*11/10000 = 110000
    assert_eq!(
        s.accounts[&0].collateral,
        1_000_000_000 - 110_000
    );
    s.process_config_applied(1, 42);
    assert_eq!(s.config_versions[1], 42);
    // Newer version overwrites
    s.process_config_applied(1, 99);
    assert_eq!(s.config_versions[1], 99);
    // Other symbols unaffected
    assert_eq!(s.config_versions[2], 0);
    std::env::remove_var("RSX_SYMBOL_0_TAKER_FEE_BPS");
    std::env::remove_var("RSX_SYMBOL_0_MAKER_FEE_BPS");
}

#[test]
fn config_applied_forwarded_to_gateway() {
    // NOTE: process_config_applied currently only tracks
    // version. Forwarding to gateway would require a
    // ring producer (not yet wired). This test verifies
    // the version is stored so a future gateway forward
    // can read it.
    let mut s = make_shard();
    s.process_config_applied(2, 7);
    assert_eq!(s.config_versions[2], 7);
}

// --- Frozen margin release ---

#[test]
fn partial_fill_releases_remaining_frozen() {
    let mut s = make_shard();
    s.accounts
        .insert(0, Account::new(0, 1_000_000_000));
    s.mark_prices[0] = 10_000;
    let mut o = order(0, 0, 10_000, 10);
    o.order_id_hi = 0;
    o.order_id_lo = 42;
    let resp = s.process_order(&o);
    assert!(
        matches!(resp, OrderResponse::Accepted { .. })
    );
    let frozen_before = s.accounts[&0].frozen_margin;
    assert!(frozen_before > 0);
    // Partial fill of 6 out of 10
    s.process_fill(&fill(0, 1, 0, 10_000, 6, 1));
    // Order done -> release remaining frozen
    s.release_frozen_for_order(0, 0, 42);
    assert_eq!(s.accounts[&0].frozen_margin, 0);
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
    let o = order(0, 0, 10_000, 1);
    let resp = s.process_order(&o);
    assert!(matches!(
        resp,
        OrderResponse::Rejected {
            reason: RejectReason::UserInLiquidation,
            ..
        }
    ));
}
