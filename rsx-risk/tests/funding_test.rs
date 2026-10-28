use rsx_risk::funding::calculate_payment;
use rsx_risk::funding::calculate_rate;
use rsx_risk::funding::interval_id;
use rsx_risk::funding::is_settlement_due;
use rsx_risk::funding::settle_symbol;

#[test]
fn funding_rate_mark_above_index_positive() {
    let rate = calculate_rate(10100, 10000, -100, 100);
    // (10100-10000)*10000/10000 = 100 bps
    assert_eq!(rate, 100);
}

#[test]
fn funding_rate_mark_below_index_negative() {
    let rate = calculate_rate(9900, 10000, -100, 100);
    assert_eq!(rate, -100);
}

#[test]
fn funding_rate_clamped_to_bounds() {
    let rate = calculate_rate(11000, 10000, -50, 50);
    // 1000 bps unclamped, capped to 50
    assert_eq!(rate, 50);
    let rate2 = calculate_rate(9000, 10000, -50, 50);
    assert_eq!(rate2, -50);
}

#[test]
fn funding_rate_clamps_to_the_symbol_bounds() {
    assert_eq!(calculate_rate(10_200, 10_000, -75, 75), 75);
    assert_eq!(calculate_rate(9_800, 10_000, -75, 75), -75);
    assert_eq!(calculate_rate(10_200, 10_000, -10, 300), 200);
    assert_eq!(calculate_rate(9_800, 10_000, -10, 300), -10);
}

#[test]
fn funding_payment_long_pays_when_positive() {
    // long (net_qty>0), positive rate -> pays
    let p = calculate_payment(10, 1000, 100);
    // 10 * 1000 * 100 / 10000 = 100
    assert_eq!(p, 100);
    assert!(p > 0); // pays
}

#[test]
fn funding_payment_short_pays_when_negative() {
    // short (net_qty<0), negative rate -> pays
    // -10 * 1000 * (-100) / 10000 = 100 (pays)
    let p = calculate_payment(-10, 1000, -100);
    assert_eq!(p, 100);
    assert!(p > 0);
}

#[test]
fn funding_zero_position_no_payment() {
    let p = calculate_payment(0, 1000, 100);
    assert_eq!(p, 0);
}

#[test]
fn funding_rate_mark_equals_index_zero() {
    let rate = calculate_rate(10000, 10000, -100, 100);
    assert_eq!(rate, 0);
}

#[test]
fn funding_zero_sum_across_all_users() {
    let rate = 50; // 50 bps
    let mark = 1000;
    // User A: long 10, User B: short 10
    let pa = calculate_payment(10, mark, rate);
    let pb = calculate_payment(-10, mark, rate);
    assert_eq!(pa + pb, 0);
}

#[test]
fn funding_with_position_opened_mid_interval() {
    // Position opened mid-interval still gets full
    // funding at settlement. Funding applies to
    // current position at settlement time.
    let rate = 100;
    let mark = 1000;
    let p = calculate_payment(5, mark, rate);
    assert_eq!(p, 50);
}

#[test]
fn funding_extreme_divergence_clamped() {
    // mark=20000, index=10000 -> 10000 bps unclamped
    let rate = calculate_rate(20000, 10000, -100, 100);
    assert_eq!(rate, 100);
}

#[test]
fn funding_settlement_idempotent() {
    let id1 = interval_id(28800, 28800);
    let id2 = interval_id(28800 + 100, 28800);
    // Same interval
    assert_eq!(id1, id2);
    assert!(!is_settlement_due(id1, 28800 + 100, 28800));
}

#[test]
fn funding_index_price_zero_handled() {
    let rate = calculate_rate(10000, 0, -100, 100);
    assert_eq!(rate, 0);
}

#[test]
fn funding_missed_interval_settled_on_startup() {
    // last_id=0, current=28800*3 -> 3 intervals passed
    assert!(is_settlement_due(0, 28800 * 3, 28800));
    assert!(is_settlement_due(1, 28800 * 3, 28800));
    assert!(is_settlement_due(2, 28800 * 3, 28800));
}

#[test]
fn funding_settlement_uses_latest_mark_price() {
    // Payment depends on mark price at settlement time.
    // Two settlements with different marks -> different
    // payments for the same position.
    let qty = 10;
    let rate = 50; // 50 bps
    let p1 = calculate_payment(qty, 1000, rate);
    let p2 = calculate_payment(qty, 2000, rate);
    assert_eq!(p1, 50); // 10*1000*50/10000
    assert_eq!(p2, 100); // 10*2000*50/10000
    assert_ne!(p1, p2);
}

#[test]
fn funding_payment_formula_qty_times_mark_times_rate() {
    // Explicit known-value formula verification.
    // qty=10, mark=1000, rate=50 bps
    // payment = 10 * 1000 * 50 / 10000 = 50
    let p = calculate_payment(10, 1000, 50);
    assert_eq!(p, 50);
}

#[test]
fn funding_payment_floors_toward_negative_infinity() {
    // Project rule: floor always. A short receiving funding has a
    // negative scaled value; -3 * 333 * 50 = -49950, /10000 = -4.995.
    // Floor = -5 (truncating `/` would wrongly give -4).
    let p = calculate_payment(-3, 333, 50);
    assert_eq!(p, -5);
    // The mirrored long pays floor(+4.995) = +4.
    let q = calculate_payment(3, 333, 50);
    assert_eq!(q, 4);
}

#[test]
fn funding_zero_sum_uneven_position_split() {
    // Invariant #9 (P0). Three longs of qty 1 and one short of qty 3.
    // Σ net_qty = 0. Independent per-user floor leaks -1 unit:
    //   long  +1 -> floor(0.1665e2/... ) per user, short -3 -> -4.995.
    // settle_symbol must net EXACTLY to zero.
    let net_qtys = [1, 1, 1, -3];
    let payments = settle_symbol(&net_qtys, 333, 50);
    let total: i64 = payments.iter().sum();
    assert_eq!(total, 0, "funding must be zero-sum (invariant #9)");
}

#[test]
fn funding_zero_sum_negative_rate_uneven_split() {
    // Same shape, negative rate (shorts pay longs). Must still net 0.
    let net_qtys = [1, 1, 1, -3];
    let payments = settle_symbol(&net_qtys, 333, -50);
    let total: i64 = payments.iter().sum();
    assert_eq!(total, 0);
}

#[test]
fn funding_zero_sum_many_users_pathological_marks() {
    // Stress: many users, marks/rates chosen to maximize remainders.
    // Σ net_qty held at 0; settle_symbol must net exactly to zero
    // regardless of how the floors fall.
    for &mark in &[1, 7, 333, 99_991, 1_000_003] {
        for &rate in &[1, 3, 17, 50, -1, -37, -100] {
            // 5 longs summing to +37, 4 shorts summing to -37.
            let net_qtys = [3, 7, 11, 5, 11, -13, -9, -8, -7];
            assert_eq!(net_qtys.iter().sum::<i64>(), 0);
            let payments = settle_symbol(&net_qtys, mark, rate);
            let total: i64 = payments.iter().sum();
            assert_eq!(total, 0, "zero-sum failed at mark={mark} rate={rate}");
        }
    }
}

#[test]
fn funding_settle_matches_calculate_payment_when_exact() {
    // When every payment divides evenly, settle_symbol must equal
    // the per-user calculate_payment values (no spurious adjustment).
    let net_qtys = [10, -10];
    let payments = settle_symbol(&net_qtys, 1000, 50);
    assert_eq!(payments[0], calculate_payment(10, 1000, 50));
    assert_eq!(payments[1], calculate_payment(-10, 1000, 50));
    assert_eq!(payments.iter().sum::<i64>(), 0);
}

#[test]
fn funding_settle_empty_no_panic() {
    let payments = settle_symbol(&[], 1000, 50);
    assert!(payments.is_empty());
}

#[test]
fn funding_settle_residual_parked_on_largest_position() {
    // The short (qty 3, |3| largest) should absorb the residual so the
    // small longs keep their natural floored value.
    let net_qtys = [1, 1, 1, -3];
    let payments = settle_symbol(&net_qtys, 333, 50);
    // Each long floors to +0? 1*333*50=16650 -> floor(1.665)=1.
    assert_eq!(payments[0], 1);
    assert_eq!(payments[1], 1);
    assert_eq!(payments[2], 1);
    // Short carries -3 to balance the three +1 longs.
    assert_eq!(payments[3], -3);
    assert_eq!(payments.iter().sum::<i64>(), 0);
}

#[test]
fn settle_symbol_zero_sum_balanced_uneven_split() {
    // Globally balanced uneven split [+1,+1,+1,-3] (Σ net=0,
    // i.e. all counterparties live on this shard): per-user
    // truncation would otherwise leak; settle_symbol must sum
    // to exactly 0.
    let net = [1i64, 1, 1, -3];
    let mark = 1000;
    let rate = 33; // odd rate forces division residuals
    let pays = settle_symbol(&net, mark, rate);
    assert_eq!(pays.len(), net.len());
    assert_eq!(pays.iter().sum::<i64>(), 0);
}

#[test]
fn settle_symbol_one_sided_shard_keeps_net_funding() {
    // A shard owning only the long side (Σ net != 0) must NOT
    // be forced to zero — the offsetting users are on other
    // shards. Funding still flows; only the intra-shard
    // rounding leak is removed, so Σpays == aggregate payment.
    let net = [7i64, -1, -1, -1, -1, -1, -1]; // Σ = 1
    let mark = 1000;
    let rate = 17;
    let pays = settle_symbol(&net, mark, rate);
    let agg = calculate_payment(1, mark, rate);
    assert_eq!(pays.iter().sum::<i64>(), agg);
    assert_ne!(agg, 0); // one-sided shard still owes funding
}

#[test]
fn settle_symbol_matches_per_user_when_no_rounding() {
    // When every per-user payment divides cleanly, settle_symbol
    // equals plain per-user calculate_payment (no residual).
    let net = [100i64, -40, -60];
    let mark = 1000;
    let rate = 50;
    let pays = settle_symbol(&net, mark, rate);
    let expected: Vec<i64> = net
        .iter()
        .map(|&q| calculate_payment(q, mark, rate))
        .collect();
    assert_eq!(pays, expected);
}

#[test]
fn settle_symbol_empty_is_empty() {
    let pays = settle_symbol(&[], 1000, 50);
    assert!(pays.is_empty());
}

mod loaded_state {
    use rsx_risk::persist::FundingSettlement;
    use rsx_risk::persist::PersistEvent;
    use rsx_risk::replay::ColdStartState;
    use rsx_risk::Account;
    use rsx_risk::LiquidationConfig;
    use rsx_risk::Position;
    use rsx_risk::ReplicationConfig;
    use rsx_risk::RiskShard;
    use rsx_risk::ShardConfig;
    use rsx_risk::SymbolRiskConfig;
    use rsx_risk::SymbolRiskParams;
    use rtrb::Consumer;
    use rustc_hash::FxHashMap;

    const INTERVAL_SECS: u64 = 28_800;

    fn shard() -> RiskShard {
        funded(shard_config())
    }

    fn funded(config: ShardConfig) -> RiskShard {
        let mut shard = RiskShard::new(config);
        shard.funding_interval_secs.fill(INTERVAL_SECS);
        shard.funding_min_bps.fill(-100);
        shard.funding_max_bps.fill(100);
        shard
    }

    fn symbol_config(symbol_id: u32, config_version: u64, interval_secs: u64) -> SymbolRiskConfig {
        SymbolRiskConfig {
            symbol_id,
            config_version,
            params: SymbolRiskParams {
                initial_margin_rate: 1000,
                maintenance_margin_rate: 500,
                max_leverage: 10,
                min_notional: 1,
            },
            taker_fee_bps: 5,
            maker_fee_bps: -1,
            tick_size: 1,
            funding_interval_secs: interval_secs,
            funding_min_bps: -75,
            funding_max_bps: 75,
        }
    }

    fn shard_config() -> ShardConfig {
        ShardConfig {
            shard_id: 0,
            shard_count: 1,
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

    fn loaded_long_and_short() -> RiskShard {
        let mut shard = shard();
        let mut accounts = FxHashMap::default();
        accounts.insert(0, Account::new(0, 1_000_000_000));
        accounts.insert(1, Account::new(1, 1_000_000_000));
        let mut positions = FxHashMap::default();
        let mut long = Position::new(0, 0);
        long.long_qty = 100;
        positions.insert((0, 0), long);
        let mut short = Position::new(1, 0);
        short.short_qty = 100;
        positions.insert((1, 0), short);
        shard.set_state(ColdStartState {
            accounts,
            positions,
            tips: vec![0; 4],
            insurance_funds: FxHashMap::default(),
            frozen_orders: FxHashMap::default(),
            funding_settled: vec![None; 4],
        });
        shard.update_mark(0, 10_200);
        shard.index_prices[0].price = 10_000;
        shard.index_prices[0].valid = true;
        shard
    }

    #[test]
    fn positions_loaded_from_state_are_funded() {
        let mut shard = loaded_long_and_short();
        shard.maybe_settle_funding(INTERVAL_SECS);
        // 100 * 10_200 * 100 bps / 10_000 = 10_200 each way.
        assert_eq!(shard.accounts[&0].collateral, 1_000_000_000 - 10_200);
        assert_eq!(shard.accounts[&1].collateral, 1_000_000_000 + 10_200);
    }

    fn drain(ring: &mut Consumer<PersistEvent>) -> Vec<FundingSettlement> {
        let mut settlements = Vec::new();
        while let Ok(event) = ring.pop() {
            if let PersistEvent::FundingSettlement(settlement) = event {
                settlements.push(settlement);
            }
        }
        settlements
    }

    fn symbol_zero_intervals(settlements: &[FundingSettlement]) -> Vec<u64> {
        settlements
            .iter()
            .filter(|settlement| settlement.symbol_id == 0)
            .map(|settlement| settlement.interval_id)
            .collect()
    }

    fn with_ring(shard: &mut RiskShard) -> Consumer<PersistEvent> {
        let (producer, consumer) = rtrb::RingBuffer::<PersistEvent>::new(1024);
        shard.set_persist_producer(producer);
        consumer
    }

    #[test]
    fn missed_intervals_settle_once_each_in_order() {
        let mut shard = loaded_long_and_short();
        shard.funding_settled[0] = Some(3);
        let mut ring = with_ring(&mut shard);

        shard.maybe_settle_funding(7 * INTERVAL_SECS + 5);

        let settlements = drain(&mut ring);
        assert_eq!(symbol_zero_intervals(&settlements), vec![4, 5, 6, 7]);
        assert_eq!(shard.funding_settled[0], Some(7));
        assert_eq!(shard.accounts[&0].collateral, 1_000_000_000 - 4 * 10_200);
        for settlement in settlements.iter().filter(|s| s.symbol_id == 0) {
            let sum: i64 = settlement.payments.iter().map(|p| p.amount).sum();
            assert_eq!(sum, 0, "interval {}", settlement.interval_id);
            assert_eq!(settlement.accounts.len(), 2);
        }
    }

    #[test]
    fn a_settled_interval_is_not_settled_again() {
        let mut shard = loaded_long_and_short();
        shard.funding_settled[0] = Some(3);
        let mut ring = with_ring(&mut shard);

        shard.maybe_settle_funding(3 * INTERVAL_SECS + 100);
        shard.maybe_settle_funding(3 * INTERVAL_SECS + 101);

        assert!(symbol_zero_intervals(&drain(&mut ring)).is_empty());
        assert_eq!(shard.accounts[&0].collateral, 1_000_000_000);
    }

    #[test]
    fn a_symbol_with_no_record_settles_the_current_interval_once() {
        let mut shard = loaded_long_and_short();
        let mut ring = with_ring(&mut shard);

        shard.maybe_settle_funding(5 * INTERVAL_SECS);
        shard.maybe_settle_funding(5 * INTERVAL_SECS + 1);

        assert_eq!(symbol_zero_intervals(&drain(&mut ring)), vec![5]);
        assert_eq!(shard.funding_settled[0], Some(5));
    }

    #[test]
    fn open_positions_without_prices_wait_and_are_not_lost() {
        let mut shard = loaded_long_and_short();
        shard.funding_settled[0] = Some(3);
        shard.mark_prices[0] = 0;
        shard.index_prices[0].price = 0;
        let mut ring = with_ring(&mut shard);

        shard.maybe_settle_funding(5 * INTERVAL_SECS);
        assert!(symbol_zero_intervals(&drain(&mut ring)).is_empty());
        assert_eq!(shard.funding_settled[0], Some(3));

        shard.update_mark(0, 10_200);
        shard.index_prices[0].price = 10_000;
        shard.maybe_settle_funding(5 * INTERVAL_SECS + 1);

        assert_eq!(symbol_zero_intervals(&drain(&mut ring)), vec![4, 5]);
        assert_eq!(shard.funding_settled[0], Some(5));
    }

    #[test]
    fn a_symbol_without_config_is_not_settled() {
        let mut config = shard_config();
        config.symbol_params[1] = None;
        let mut shard = funded(config);
        let mut ring = with_ring(&mut shard);

        shard.maybe_settle_funding(2 * INTERVAL_SECS);

        let settled: Vec<u32> = drain(&mut ring)
            .iter()
            .map(|settlement| settlement.symbol_id)
            .collect();
        assert_eq!(settled, vec![0, 2, 3]);
        assert_eq!(shard.funding_settled[1], None);
    }

    #[test]
    fn each_symbol_settles_on_its_own_interval() {
        let mut shard = loaded_long_and_short();
        shard.funding_interval_secs[0] = 3_600;
        shard.funding_settled[0] = Some(0);
        shard.funding_settled[1] = Some(0);
        let mut ring = with_ring(&mut shard);

        shard.maybe_settle_funding(8 * 3_600);

        let settlements = drain(&mut ring);
        assert_eq!(
            symbol_zero_intervals(&settlements),
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
        let symbol_one: Vec<u64> = settlements
            .iter()
            .filter(|settlement| settlement.symbol_id == 1)
            .map(|settlement| settlement.interval_id)
            .collect();
        assert_eq!(symbol_one, vec![1]);
    }

    #[test]
    fn the_rate_clamps_to_the_symbols_own_bounds() {
        let mut shard = loaded_long_and_short();
        shard.funding_min_bps[0] = -75;
        shard.funding_max_bps[0] = 75;
        shard.update_mark(0, 10_200);
        let mut ring = with_ring(&mut shard);
        shard.maybe_settle_funding(INTERVAL_SECS);

        shard.update_mark(0, 9_800);
        shard.funding_settled[0] = Some(1);
        shard.maybe_settle_funding(2 * INTERVAL_SECS);

        let rates: Vec<i64> = drain(&mut ring)
            .iter()
            .filter(|settlement| settlement.symbol_id == 0)
            .map(|settlement| settlement.payments[0].rate)
            .collect();
        assert_eq!(rates, vec![75, -75]);
    }

    #[test]
    fn applying_a_config_installs_the_symbols_funding_parameters() {
        let mut shard = RiskShard::new(shard_config());
        shard.apply_symbol_config(&symbol_config(2, 1, 3_600));

        assert_eq!(shard.funding_interval_secs[2], 3_600);
        assert_eq!(shard.funding_min_bps[2], -75);
        assert_eq!(shard.funding_max_bps[2], 75);
        assert_eq!(shard.funding_interval_secs[1], 0);
    }

    #[test]
    fn a_config_that_changes_the_interval_is_refused() {
        let mut shard = RiskShard::new(shard_config());
        shard.apply_symbol_config(&symbol_config(2, 1, 3_600));

        shard.apply_symbol_config(&symbol_config(2, 2, 28_800));

        assert_eq!(shard.funding_interval_secs[2], 3_600);
        assert_eq!(shard.config_versions[2], 1);
    }
}
