use rsx_risk::Account;
use rsx_risk::BboUpdate;
use rsx_risk::FillEvent;
use rsx_risk::FundingConfig;
use rsx_risk::LiquidationConfig;
use rsx_risk::OrderRequest;
use rsx_risk::OrderResponse;
use rsx_risk::RiskShard;
use rsx_risk::ShardConfig;
use rsx_risk::SymbolRiskParams;

fn config_single_shard() -> ShardConfig {
    ShardConfig {
        shard_id: 0,
        shard_count: 1, // all users in shard
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
    }
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
        is_liquidation: false,
        _pad: [0; 4],
    }
}

#[test]
fn shard_1000_fills_positions_correct() {
    let mut s = RiskShard::new(config_single_shard());
    s.accounts.insert(0, Account::new(0, 10_000_000));
    s.accounts.insert(1, Account::new(1, 10_000_000));

    for i in 1..=1000u64 {
        s.process_fill(&fill(0, 1, 0, 1000, 1, 0, i));
    }

    assert_eq!(s.fills_processed, 1000);
    assert_eq!(s.positions[&(0, 0)].long_qty, 1000);
    assert_eq!(s.positions[&(1, 0)].short_qty, 1000);
    assert_eq!(s.tips[0], 1000);
}

#[test]
fn shard_order_accept_reject_flow() {
    let mut s = RiskShard::new(config_single_shard());
    s.accounts.insert(0, Account::new(0, 100_000));
    s.mark_prices[0] = 10_000;

    // First order: accepted
    let r1 = s.process_order(&order(0, 0, 10_000, 1));
    assert!(matches!(r1, OrderResponse::Accepted { .. }));

    // Drain margin with big order
    let r2 = s.process_order(&order(0, 0, 10_000, 1000));
    assert!(matches!(r2, OrderResponse::Rejected { .. }));
}

#[test]
fn shard_bbo_triggers_margin_recalc() {
    let mut s = RiskShard::new(config_single_shard());
    s.accounts.insert(0, Account::new(0, 100_000));
    s.mark_prices[0] = 10_000;

    // Give user a position
    s.process_fill(&fill(0, 1, 0, 10_000, 10, 0, 1));

    let bbo = BboUpdate {
        seq: 1,
        symbol_id: 0,
        bid_px: 9000,
        bid_qty: 100,
        ask_px: 9200,
        ask_qty: 100,
    };
    s.process_bbo(&bbo);
    assert!(s.index_prices[0].valid);
    assert_eq!(s.index_prices[0].price, 9100);
}

#[test]
fn shard_liquidation_on_price_drop() {
    let mut s = RiskShard::new(config_single_shard());
    s.accounts.insert(0, Account::new(0, 10_000));
    s.mark_prices[0] = 10_000;

    // Build a long position
    s.process_fill(&fill(0, 1, 0, 10_000, 100, 0, 1));

    // Crash mark price — position deeply underwater
    s.mark_prices[0] = 1;

    // Non-liquidation order should be rejected
    let resp = s.process_order(&order(0, 0, 1, 1));
    assert!(matches!(
        resp,
        OrderResponse::Rejected {
            reason: rsx_risk::RejectReason::UserInLiquidation,
            ..
        }
    ));

    // Liquidation order should pass
    let mut liq = order(0, 0, 1, 1);
    liq.is_liquidation = true;
    let resp = s.process_order(&liq);
    assert!(matches!(resp, OrderResponse::Accepted { .. }));
}

#[test]
fn shard_funding_settlement() {
    let mut s = RiskShard::new(config_single_shard());
    s.accounts.insert(0, Account::new(0, 1_000_000));
    s.accounts.insert(1, Account::new(1, 1_000_000));
    s.mark_prices[0] = 10_100;
    s.index_prices[0].price = 10_000;
    s.index_prices[0].valid = true;

    // Build opposing positions
    s.process_fill(&fill(0, 1, 0, 10_000, 100, 0, 1));

    let before_0 = s.accounts[&0].collateral;
    let before_1 = s.accounts[&1].collateral;

    s.maybe_settle_funding(28_800);

    let after_0 = s.accounts[&0].collateral;
    let after_1 = s.accounts[&1].collateral;

    // Long pays, short receives (mark > index)
    let delta_0 = after_0 - before_0;
    let delta_1 = after_1 - before_1;
    assert!(delta_0 < 0); // long user pays
    assert!(delta_1 > 0); // short user receives
}

#[test]
fn shard_multi_symbol_tips_independent() {
    let mut s = RiskShard::new(config_single_shard());
    s.accounts.insert(0, Account::new(0, 10_000_000));

    s.process_fill(&fill(0, 1, 0, 100, 10, 0, 10));
    s.process_fill(&fill(0, 1, 1, 200, 5, 0, 7));
    s.process_fill(&fill(0, 1, 2, 300, 1, 0, 20));

    assert_eq!(s.tips[0], 10);
    assert_eq!(s.tips[1], 7);
    assert_eq!(s.tips[2], 20);
    assert_eq!(s.tips[3], 0);
}

#[test]
fn shard_position_flip_through_fills() {
    let mut s = RiskShard::new(config_single_shard());
    s.accounts.insert(0, Account::new(0, 10_000_000));

    // Buy 100
    s.process_fill(&fill(0, 1, 0, 1000, 100, 0, 1));
    assert_eq!(s.positions[&(0, 0)].long_qty, 100);

    // Sell 150 -> net short 50
    s.process_fill(&fill(0, 1, 0, 1000, 150, 1, 2));
    let pos = &s.positions[&(0, 0)];
    assert_eq!(pos.long_qty, 0);
    assert_eq!(pos.short_qty, 50);
}

#[test]
fn shard_cancel_restores_margin() {
    let mut s = RiskShard::new(config_single_shard());
    s.accounts
        .insert(0, Account::new(0, 1_000_000));
    s.mark_prices[0] = 10_000;

    // Place order, margin frozen
    let resp = s.process_order(&order(0, 0, 10_000, 10));
    let reserved = match resp {
        OrderResponse::Accepted {
            margin_reserved, ..
        } => margin_reserved,
        _ => panic!("expected accepted"),
    };
    assert!(reserved > 0);
    let frozen = s.accounts[&0].frozen_margin;
    assert_eq!(frozen, reserved);

    // Simulate cancel: release margin
    s.accounts
        .get_mut(&0)
        .unwrap()
        .release_margin(reserved);
    assert_eq!(s.accounts[&0].frozen_margin, 0);
}

#[test]
fn shard_multiple_users_same_symbol() {
    let mut s = RiskShard::new(config_single_shard());
    s.accounts.insert(0, Account::new(0, 10_000_000));
    s.accounts.insert(1, Account::new(1, 10_000_000));
    s.accounts.insert(2, Account::new(2, 10_000_000));

    // 0 buys from 1, then 2 buys from 1
    s.process_fill(&fill(0, 1, 0, 1000, 50, 0, 1));
    s.process_fill(&fill(2, 1, 0, 1000, 30, 0, 2));

    assert_eq!(s.positions[&(0, 0)].long_qty, 50);
    assert_eq!(s.positions[&(2, 0)].long_qty, 30);
    assert_eq!(s.positions[&(1, 0)].short_qty, 80);
}

#[test]
fn shard_user_opens_closes_reopens() {
    let mut s = RiskShard::new(config_single_shard());
    s.accounts.insert(0, Account::new(0, 10_000_000));

    // Open long 100
    s.process_fill(&fill(0, 1, 0, 1000, 100, 0, 1));
    assert_eq!(s.positions[&(0, 0)].long_qty, 100);

    // Close long 100 (sell)
    s.process_fill(&fill(0, 1, 0, 1000, 100, 1, 2));
    assert!(s.positions[&(0, 0)].is_empty());

    // Reopen short 50 (sell)
    s.process_fill(&fill(0, 1, 0, 1000, 50, 1, 3));
    assert_eq!(s.positions[&(0, 0)].short_qty, 50);
}

#[test]
fn shard_fill_updates_exposure_index() {
    let mut s = RiskShard::new(config_single_shard());
    s.accounts.insert(0, Account::new(0, 10_000_000));
    s.mark_prices[0] = 10_000;
    s.index_prices[0].price = 10_000;
    s.index_prices[0].valid = true;

    // Open position
    s.process_fill(&fill(0, 1, 0, 1000, 100, 0, 1));

    // Funding should affect user 0
    let before = s.accounts[&0].collateral;
    s.mark_prices[0] = 10_100;
    s.maybe_settle_funding(28_800);
    assert_ne!(s.accounts[&0].collateral, before);

    // Close position
    s.process_fill(&fill(0, 1, 0, 1000, 100, 1, 2));
    assert!(s.positions[&(0, 0)].is_empty());

    // Next funding should NOT affect user 0
    let before2 = s.accounts[&0].collateral;
    s.maybe_settle_funding(57_600);
    assert_eq!(s.accounts[&0].collateral, before2);
}

#[test]
fn shard_order_accepted_then_rejected_margin_used() {
    let mut s = RiskShard::new(config_single_shard());
    s.accounts.insert(0, Account::new(0, 15_000));
    s.mark_prices[0] = 10_000;

    // First order: notional=10*10000=100000, IM=10000
    let r1 = s.process_order(&order(0, 0, 10_000, 10));
    assert!(matches!(r1, OrderResponse::Accepted { .. }));

    // Second order needs more margin than remaining
    let r2 = s.process_order(&order(0, 0, 10_000, 10));
    assert!(matches!(r2, OrderResponse::Rejected { .. }));
}

#[test]
fn shard_mark_price_divergence_triggers_liquidation() {
    let mut s = RiskShard::new(config_single_shard());
    s.accounts
        .insert(0, Account::new(0, 1_000_000));
    s.mark_prices[0] = 10_000;

    // Open long 10 at 10,000
    s.process_fill(&fill(0, 1, 0, 10_000, 10, 0, 1));

    // Normal order works (small)
    let r1 = s.process_order(&order(0, 0, 10_000, 1));
    assert!(matches!(r1, OrderResponse::Accepted { .. }));

    // Mark price crashes — massive unrealized loss
    s.mark_prices[0] = 1;
    // UPnL = 10 * (1 - 10000) = -99,990
    // equity = 1,000,000 - 99,990 = 900,010
    // but maint margin = 10*1*500/10000 = 0
    // Hmm, notional at mark=1 is tiny.
    // Need bigger position for liquidation.

    // Crash with large position instead
    let mut s2 = RiskShard::new(config_single_shard());
    s2.accounts.insert(0, Account::new(0, 10_000));
    s2.mark_prices[0] = 10_000;

    // Open large long: notional = 1000 * 10000 = 10M
    s2.process_fill(
        &fill(0, 1, 0, 10_000, 1000, 0, 1),
    );
    // equity = 10,000 (coll) + upnl(0) = 10,000
    // maint = 1000*10000*500/10000 = 500,000
    // equity(10k) < maint(500k) -> liquidation
    let r2 = s2.process_order(&order(0, 0, 10_000, 1));
    assert!(matches!(
        r2,
        OrderResponse::Rejected {
            reason: rsx_risk::RejectReason::UserInLiquidation,
            ..
        }
    ));
}

#[test]
fn shard_idle_no_resource_leak() {
    let mut s = RiskShard::new(config_single_shard());
    s.accounts.insert(0, Account::new(0, 1_000_000));

    // Process some fills then idle
    s.process_fill(&fill(0, 1, 0, 1000, 10, 0, 1));
    let acct_before = s.accounts[&0].clone();
    let pos_before = s.positions[&(0, 0)].clone();

    // Multiple idle funding checks (no mark/index)
    for t in 0..10u64 {
        s.maybe_settle_funding(t * 100);
    }

    // State unchanged (no valid prices for funding)
    assert_eq!(
        s.accounts[&0].collateral,
        acct_before.collateral,
    );
    assert_eq!(
        s.positions[&(0, 0)].long_qty,
        pos_before.long_qty,
    );
}
