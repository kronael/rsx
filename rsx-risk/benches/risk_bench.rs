use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use criterion::black_box;
use rsx_risk::Account;
use rsx_risk::ExposureIndex;
use rsx_risk::FundingConfig;
use rsx_risk::LiquidationConfig;
use rsx_risk::OrderRequest;
use rsx_risk::PortfolioMargin;
use rsx_risk::Position;
use rsx_risk::ReplicationConfig;
use rsx_risk::RiskShard;
use rsx_risk::ShardConfig;
use rsx_risk::SymbolRiskParams;
use rsx_risk::liquidation::LiquidationEngine;
use rsx_risk::price::calculate_index;
use rsx_risk::types::BboUpdate;

fn make_shard(max_symbols: usize) -> RiskShard {
    let mut params = Vec::with_capacity(max_symbols);
    let mut taker = Vec::with_capacity(max_symbols);
    let mut maker = Vec::with_capacity(max_symbols);
    for _ in 0..max_symbols {
        params.push(SymbolRiskParams {
            initial_margin_rate: 1000,
            maintenance_margin_rate: 500,
            max_leverage: 10,
        });
        taker.push(5i64);
        maker.push(-1i64);
    }
    RiskShard::new(ShardConfig {
        shard_id: 0,
        shard_count: 1,
        max_symbols,
        symbol_params: params,
        taker_fee_bps: taker,
        maker_fee_bps: maker,
        funding_config: FundingConfig::default(),
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig::default(),
    })
}

// --- Phase 1: Pure math, no I/O ---

fn bench_apply_fill_to_position(c: &mut Criterion) {
    let mut pos = Position::new(1, 0);
    let mut seq = 1u64;
    c.bench_function(
        "apply_fill_to_position",
        |b| {
            b.iter(|| {
                seq += 1;
                black_box(&mut pos).apply_fill(
                    black_box(0),
                    black_box(50_000),
                    black_box(100),
                    black_box(seq),
                );
            })
        },
    );
}

fn bench_portfolio_margin_10(c: &mut Criterion) {
    let params: Vec<SymbolRiskParams> = (0..10)
        .map(|_| SymbolRiskParams {
            initial_margin_rate: 1000,
            maintenance_margin_rate: 500,
            max_leverage: 10,
        })
        .collect();
    let pm = PortfolioMargin {
        symbol_params: params,
    };
    let account = Account::new(1, 1_000_000_000);
    let positions: Vec<Position> = (0..10)
        .map(|i| {
            let mut p = Position::new(1, i as u32);
            p.apply_fill(0, 50_000, 100, 1);
            p
        })
        .collect();
    let pos_refs: Vec<&Position> =
        positions.iter().collect();
    let marks: Vec<i64> =
        (0..10).map(|_| 50_000i64).collect();

    c.bench_function(
        "portfolio_margin_10_positions",
        |b| {
            b.iter(|| {
                black_box(pm.calculate(
                    black_box(&account),
                    black_box(&pos_refs),
                    black_box(&marks),
                ))
            })
        },
    );
}

fn bench_portfolio_margin_50(c: &mut Criterion) {
    let params: Vec<SymbolRiskParams> = (0..50)
        .map(|_| SymbolRiskParams {
            initial_margin_rate: 1000,
            maintenance_margin_rate: 500,
            max_leverage: 10,
        })
        .collect();
    let pm = PortfolioMargin {
        symbol_params: params,
    };
    let account = Account::new(1, 10_000_000_000);
    let positions: Vec<Position> = (0..50)
        .map(|i| {
            let mut p = Position::new(1, i as u32);
            p.apply_fill(0, 50_000, 100, 1);
            p
        })
        .collect();
    let pos_refs: Vec<&Position> =
        positions.iter().collect();
    let marks: Vec<i64> =
        (0..50).map(|_| 50_000i64).collect();

    c.bench_function(
        "portfolio_margin_50_positions",
        |b| {
            b.iter(|| {
                black_box(pm.calculate(
                    black_box(&account),
                    black_box(&pos_refs),
                    black_box(&marks),
                ))
            })
        },
    );
}

fn bench_index_price_calculation(c: &mut Criterion) {
    c.bench_function("index_price_calculation", |b| {
        b.iter(|| {
            black_box(calculate_index(
                black_box(49_990),
                black_box(500),
                black_box(50_010),
                black_box(300),
                black_box(50_000),
            ))
        })
    });
}

fn bench_exposure_lookup_100(c: &mut Criterion) {
    let mut idx = ExposureIndex::new(4);
    for uid in 0..100u32 {
        idx.add_user(0, uid);
    }
    c.bench_function(
        "exposure_lookup_100_users",
        |b| {
            b.iter(|| {
                black_box(idx.users_for_symbol(
                    black_box(0),
                ))
            })
        },
    );
}

fn bench_exposure_lookup_1000(c: &mut Criterion) {
    let mut idx = ExposureIndex::new(4);
    for uid in 0..1000u32 {
        idx.add_user(0, uid);
    }
    c.bench_function(
        "exposure_lookup_1000_users",
        |b| {
            b.iter(|| {
                black_box(idx.users_for_symbol(
                    black_box(0),
                ))
            })
        },
    );
}

// --- Phase 2: Shard-level, mocked ---

fn bench_pretrade_check_latency(c: &mut Criterion) {
    let mut shard = make_shard(4);
    shard.accounts.insert(0, Account::new(0, 1_000_000));
    shard.mark_prices[0] = 50_000;

    let order = OrderRequest {
        seq: 0,
        user_id: 0,
        symbol_id: 0,
        price: 50_000,
        qty: 10,
        order_id_hi: 0,
        order_id_lo: 1,
        timestamp_ns: 0,
        side: 0,
        tif: 0,
        reduce_only: false,
        post_only: false,
        is_liquidation: false,
        _pad: [0; 3],
    };

    let mut oid = 1u64;
    c.bench_function("pretrade_check_latency", |b| {
        b.iter(|| {
            let mut o = order.clone();
            oid += 1;
            o.order_id_lo = oid;
            let resp = black_box(
                shard.process_order(black_box(&o)),
            );
            // Release frozen margin to keep it passing
            shard.release_frozen_for_order(
                0, 0, oid,
            );
            resp
        })
    });
}

fn bench_bbo_processing(c: &mut Criterion) {
    let mut shard = make_shard(4);
    let bbo = BboUpdate {
        seq: 1,
        symbol_id: 0,
        bid_px: 49_990,
        bid_qty: 500,
        ask_px: 50_010,
        ask_qty: 300,
    };
    c.bench_function("bbo_processing", |b| {
        b.iter(|| {
            shard.process_bbo(black_box(&bbo));
        })
    });
}

// --- Liquidation benchmarks ---

fn bench_enqueue_liquidation(c: &mut Criterion) {
    c.bench_function("enqueue_liquidation", |b| {
        b.iter(|| {
            let mut engine =
                LiquidationEngine::new(100_000_000, 1, 10, 9999);
            engine.enqueue(
                black_box(1),
                black_box(0),
                black_box(1_000_000),
            );
        })
    });
}

fn bench_round_escalation(c: &mut Criterion) {
    let mut engine =
        LiquidationEngine::new(100_000_000, 1, 10, 9999);
    engine.enqueue(1, 0, 0);

    let get_pos = |_uid: u32, _sid: u32| -> i64 { 100 };
    let get_mark = |_sid: u32| -> i64 { 50_000 };

    c.bench_function("round_escalation", |b| {
        b.iter(|| {
            // Reset state for each iter
            for s in &mut engine.active {
                s.round = 1;
                s.last_order_ns = 0;
            }
            black_box(engine.maybe_process(
                black_box(1_000_000_000),
                &get_pos,
                &get_mark,
            ))
        })
    });
}

criterion_group!(
    phase1,
    bench_apply_fill_to_position,
    bench_portfolio_margin_10,
    bench_portfolio_margin_50,
    bench_index_price_calculation,
    bench_exposure_lookup_100,
    bench_exposure_lookup_1000,
);

criterion_group!(
    phase2,
    bench_pretrade_check_latency,
    bench_bbo_processing,
);

criterion_group!(
    liquidation,
    bench_enqueue_liquidation,
    bench_round_escalation,
);

criterion_main!(phase1, phase2, liquidation);
