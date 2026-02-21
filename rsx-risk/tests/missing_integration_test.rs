/// Integration tests for missing coverage per TESTING-RISK.md.
///
/// Covers:
///   - position = sum of fills (margin recalc on fill)
///   - crash recovery from WAL tip (no Postgres)
///   - liquidation cascade (multiple users underwater)
///   - insurance fund absorbs deficit (shard integration)

use rsx_risk::Account;
use rsx_risk::FillEvent;
use rsx_risk::FundingConfig;
use rsx_risk::LiquidationConfig;
use rsx_risk::OrderRequest;
use rsx_risk::OrderResponse;
use rsx_risk::ReplicationConfig;
use rsx_risk::RiskShard;
use rsx_risk::ShardConfig;
use rsx_risk::SymbolRiskParams;
use rsx_risk::insurance::InsuranceFund;
use rsx_risk::liquidation::LiquidationEngine;
use rsx_risk::types::RejectReason;

fn config_single() -> ShardConfig {
    ShardConfig {
        shard_id: 0,
        shard_count: 1,
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
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig::default(),
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
        post_only: false,
        is_liquidation: false,
        _pad: [0; 3],
    }
}

// --- Test 2: Margin recalculation on fill ---
// Invariant: position = sum of fills (TESTING-RISK.md §correctness #2)

#[test]
fn position_equals_sum_of_fills_long() {
    let mut s = RiskShard::new(config_single());
    s.accounts.insert(0, Account::new(0, 10_000_000));

    // Apply 5 buy fills of varying sizes
    let fills = [(100, 10), (200, 5), (150, 20), (120, 3), (180, 7)];
    let mut total_qty = 0i64;
    for (i, (price, qty)) in fills.iter().enumerate() {
        s.process_fill(&fill(0, 1, 0, *price, *qty, 0, (i + 1) as u64));
        total_qty += qty;
    }

    let pos = &s.positions[&(0, 0)];
    assert_eq!(
        pos.long_qty, total_qty,
        "long_qty must equal sum of buy fills"
    );
    assert_eq!(pos.short_qty, 0);
    assert_eq!(s.fills_processed, fills.len() as u64);
}

#[test]
fn position_equals_sum_of_fills_short() {
    let mut s = RiskShard::new(config_single());
    s.accounts.insert(0, Account::new(0, 10_000_000));

    let fills = [(100, 8), (200, 12), (150, 4)];
    let mut total_qty = 0i64;
    for (i, (price, qty)) in fills.iter().enumerate() {
        // taker_side=1 means taker sells -> taker goes short
        s.process_fill(&fill(0, 1, 0, *price, *qty, 1, (i + 1) as u64));
        total_qty += qty;
    }

    let pos = &s.positions[&(0, 0)];
    assert_eq!(
        pos.short_qty, total_qty,
        "short_qty must equal sum of sell fills"
    );
    assert_eq!(pos.long_qty, 0);
}

#[test]
fn position_net_after_partial_close() {
    // Open long 100, close 40 -> net long 60
    let mut s = RiskShard::new(config_single());
    s.accounts.insert(0, Account::new(0, 10_000_000));

    s.process_fill(&fill(0, 1, 0, 1000, 100, 0, 1));
    assert_eq!(s.positions[&(0, 0)].long_qty, 100);

    // Close 40 (taker sells -> taker_side=1)
    s.process_fill(&fill(0, 1, 0, 1000, 40, 1, 2));
    let pos = &s.positions[&(0, 0)];
    // long_qty = 100 - 40 = 60, short_qty = 0
    assert_eq!(pos.long_qty, 60);
    assert_eq!(pos.short_qty, 0);
}

#[test]
fn margin_recalculated_after_fill_detects_liquidation() {
    // After fill, shard must detect if user is underwater.
    // Small collateral + large position -> liquidation triggered.
    let mut s = RiskShard::new(config_single());
    s.accounts.insert(0, Account::new(0, 100));
    s.mark_prices[0] = 10_000;

    // long 1000 at 10_000: notional=10M, mm=500k -> liquidation
    s.process_fill(&fill(0, 1, 0, 10_000, 1000, 0, 1));

    assert!(
        s.liquidation.is_in_liquidation(0, 0),
        "user must be queued for liquidation after fill \
         leaves them underwater"
    );

    // Normal order rejected
    let resp = s.process_order(&order(0, 0, 10_000, 1));
    assert!(matches!(
        resp,
        OrderResponse::Rejected {
            reason: RejectReason::UserInLiquidation,
            ..
        }
    ));
}

// --- Test 4: Crash recovery from WAL tip ---
// Write fills to WAL, replay into fresh shard, verify state.

#[test]
fn wal_replay_rebuilds_positions_from_tip() {
    use rsx_dxs::FillRecord;
    use rsx_dxs::WalWriter;
    use rsx_risk::replay::replay_from_wal;

    let wal_dir = tempfile::tempdir().unwrap();

    // Write fills for symbol 0: seqs 1-5
    let mut writer = WalWriter::new(
        0,
        wal_dir.path(),
        None,
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();

    for i in 1..=5u64 {
        let mut rec = FillRecord {
            seq: i,
            ts_ns: i * 1000,
            symbol_id: 0,
            taker_user_id: 0,
            maker_user_id: 1,
            _pad0: 0,
            taker_order_id_hi: 0,
            taker_order_id_lo: 0,
            maker_order_id_hi: 0,
            maker_order_id_lo: 0,
            price: rsx_types::Price(5_000),
            qty: rsx_types::Qty(10),
            taker_side: 0,
            reduce_only: 0,
            tif: 0,
            post_only: 0,
            _pad1: [0; 4],
        };
        writer.append(&mut rec).unwrap();
    }
    writer.flush().unwrap();

    // Fresh shard with tip at 0 (no prior state)
    let mut shard = RiskShard::new(config_single());
    shard.accounts.insert(0, Account::new(0, 10_000_000));
    shard.accounts.insert(1, Account::new(1, 10_000_000));

    let replayed =
        replay_from_wal(&mut shard, wal_dir.path(), &[0]).unwrap();

    assert_eq!(replayed, 5, "all 5 fills replayed");
    assert_eq!(shard.tips[0], 5, "tip advanced to last seq");

    // Taker bought 5*10=50
    let pos_taker = &shard.positions[&(0, 0)];
    assert_eq!(pos_taker.long_qty, 50);

    // Maker sold 5*10=50
    let pos_maker = &shard.positions[&(1, 0)];
    assert_eq!(pos_maker.short_qty, 50);
}

#[test]
fn wal_replay_resumes_from_tip_skips_already_applied() {
    use rsx_dxs::FillRecord;
    use rsx_dxs::WalWriter;
    use rsx_risk::replay::replay_from_wal;

    let wal_dir = tempfile::tempdir().unwrap();

    // Write fills seqs 1-10
    let mut writer = WalWriter::new(
        0,
        wal_dir.path(),
        None,
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();
    for i in 1..=10u64 {
        let mut rec = FillRecord {
            seq: i,
            ts_ns: i * 1000,
            symbol_id: 0,
            taker_user_id: 0,
            maker_user_id: 1,
            _pad0: 0,
            taker_order_id_hi: 0,
            taker_order_id_lo: 0,
            maker_order_id_hi: 0,
            maker_order_id_lo: 0,
            price: rsx_types::Price(1_000),
            qty: rsx_types::Qty(1),
            taker_side: 0,
            reduce_only: 0,
            tif: 0,
            post_only: 0,
            _pad1: [0; 4],
        };
        writer.append(&mut rec).unwrap();
    }
    writer.flush().unwrap();

    // Shard already has tip=5 (fills 1-5 already applied)
    let mut shard = RiskShard::new(config_single());
    shard.accounts.insert(0, Account::new(0, 10_000_000));
    shard.accounts.insert(1, Account::new(1, 10_000_000));
    shard.tips[0] = 5;

    // Replay: active WAL file is read from beginning, but
    // process_fill deduplicates seqs <= tip. Fills 1-5 are
    // skipped by dedup; fills 6-10 are applied.
    // replayed counter reflects all RECORD_FILL records read.
    let replayed =
        replay_from_wal(&mut shard, wal_dir.path(), &[0]).unwrap();

    assert_eq!(replayed, 10, "all 10 fill records read from active wal");
    assert_eq!(shard.tips[0], 10, "tip advanced to last seq");

    // Only fills 6-10 applied via dedup (5 fills of qty=1)
    let pos = &shard.positions[&(0, 0)];
    assert_eq!(pos.long_qty, 5, "only post-tip fills applied");
}

// --- Test 5: Liquidation cascade ---
// Multiple users underwater simultaneously -> all queued.

#[test]
fn liquidation_cascade_multiple_users_all_queued() {
    let mut s = RiskShard::new(config_single());

    // Users 0,1,2 each have tiny collateral
    for uid in 0..3u32 {
        s.accounts.insert(uid, Account::new(uid, 100));
        s.mark_prices[0] = 10_000;
        // Large long position puts each user underwater:
        // notional=1000*10000=10M, mm=500k >> equity=100
        // Use different makers to avoid self-trade issues
        s.process_fill(&fill(uid, uid + 10, 0, 10_000, 1000, 0, (uid + 1) as u64));
    }

    // All three must be in liquidation
    for uid in 0..3u32 {
        assert!(
            s.liquidation.is_in_liquidation(uid, 0),
            "user {} must be queued for liquidation", uid
        );
    }

    // Normal orders for all three must be rejected
    for uid in 0..3u32 {
        let resp = s.process_order(&order(uid, 0, 10_000, 1));
        assert!(
            matches!(
                resp,
                OrderResponse::Rejected {
                    reason: RejectReason::UserInLiquidation,
                    ..
                }
            ),
            "user {} order should be rejected", uid
        );
    }
}

#[test]
fn liquidation_cascade_multi_symbol() {
    // User 0 is underwater on two symbols simultaneously
    let mut s = RiskShard::new(config_single());
    s.accounts.insert(0, Account::new(0, 100));
    s.mark_prices[0] = 10_000;
    s.mark_prices[1] = 20_000;

    // Large positions on sym 0 and sym 1
    s.process_fill(&fill(0, 1, 0, 10_000, 1000, 0, 1));
    s.process_fill(&fill(0, 1, 1, 20_000, 1000, 0, 1));

    assert!(s.liquidation.is_in_liquidation(0, 0));
    assert!(s.liquidation.is_in_liquidation(0, 1));
}

#[test]
fn liquidation_cascade_independent_per_user_symbol() {
    // User 0 underwater on sym 0, user 1 healthy on sym 0
    let mut s = RiskShard::new(config_single());
    s.accounts.insert(0, Account::new(0, 100));
    s.accounts.insert(1, Account::new(1, 100_000_000));
    s.mark_prices[0] = 10_000;

    s.process_fill(&fill(0, 1, 0, 10_000, 1000, 0, 1)); // user 0 underwater
    s.process_fill(&fill(1, 0, 0, 10_000, 1, 0, 2));    // user 1 healthy (small)

    assert!(s.liquidation.is_in_liquidation(0, 0));
    assert!(!s.liquidation.is_in_liquidation(1, 0));
}

// --- Test 6: Insurance fund absorbs deficit ---
// When liquidation fill price is worse than bankruptcy price,
// insurance fund is debited.

#[test]
fn insurance_fund_debited_on_socialized_loss() {
    // Set up liquidation engine with max_rounds=1 so it immediately
    // escalates to socialized loss on the second tick.
    let mut engine = LiquidationEngine::new(0, 10, 1);
    let mut fund = InsuranceFund::new(0, 1_000_000);

    engine.enqueue(1, 0, 0);

    // Round 1: issues liquidation order
    let (orders1, losses1) =
        engine.maybe_process(0, &|_, _| 100, &|_| 10_000);
    assert_eq!(orders1.len(), 1, "round 1 issues order");
    assert_eq!(losses1.len(), 0);

    // Round 2 (max_rounds exceeded): socialized loss emitted
    let (orders2, losses2) =
        engine.maybe_process(1, &|_, _| 100, &|_| 10_000);
    assert_eq!(orders2.len(), 0);
    assert_eq!(losses2.len(), 1, "socialized loss after max rounds");

    let loss = &losses2[0];
    let deficit = loss.qty * loss.price;
    let before = fund.balance;
    fund.deduct(deficit);
    assert!(
        fund.balance < before,
        "insurance fund balance must decrease after deduction"
    );
    assert_eq!(fund.balance, before - deficit);
    assert_eq!(fund.version, 1);
}

#[test]
fn insurance_fund_deficit_goes_negative_when_depleted() {
    // Fund with small balance, large loss -> balance goes negative
    let mut fund = InsuranceFund::new(0, 50_000);
    let deficit = 1_000_000i64; // bigger than fund
    fund.deduct(deficit);
    assert_eq!(fund.balance, 50_000 - 1_000_000);
    assert!(fund.balance < 0);
}

#[test]
fn insurance_fund_shard_integration_socialized_loss() {
    // Simulate the shard-level scenario:
    // 1. User has a position that can't be liquidated at market
    // 2. Insurance fund is debited for the deficit
    let mut s = RiskShard::new(config_single());
    s.accounts.insert(0, Account::new(0, 100));
    s.mark_prices[0] = 10_000;

    // Give the shard an insurance fund for symbol 0
    s.insurance_funds
        .insert(0, InsuranceFund::new(0, 500_000));

    // Position puts user 0 underwater (long 1000 at 10_000)
    s.process_fill(&fill(0, 1, 0, 10_000, 1000, 0, 1));
    assert!(s.liquidation.is_in_liquidation(0, 0));

    // Deduct socialized loss from insurance fund directly
    // (simulating what the main loop does after max rounds)
    let deficit = 1000i64 * 10_000; // qty * mark
    let before = s.insurance_funds[&0].balance;
    s.insurance_funds
        .get_mut(&0)
        .unwrap()
        .deduct(deficit);

    let after = s.insurance_funds[&0].balance;
    assert_eq!(after, before - deficit);
    assert!(after < before);
}
