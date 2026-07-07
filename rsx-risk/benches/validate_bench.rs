//! Risk validate-and-forward in isolation.
//!
//! What this measures
//! -----------------
//! `RiskShard::process_order(&order)` on a pre-warmed shard
//! with one account, one symbol, and a populated mark price.
//! This is the per-order CPU work between `risk_in` and the
//! handoff to ME — pre-trade margin check, frozen-margin
//! insert into the `frozen_orders` / `frozen_by_user` maps,
//! response construction.
//!
//! In production this leg sits between the gateway→risk cast
//! recv and the risk→ME cast send (the "risk_in → me_in"
//! macro-stage in `SPEED-OFFHOT.md`). Measured macro-stage
//! delta = 181 µs; that includes cast recv, two SPSC ring
//! hops, this validate path, and the cast send to ME. This
//! bench isolates the validate slice alone.
//!
//! What's included
//! - `user_in_shard` check
//! - margin state recompute (`PortfolioMargin::calculate`)
//! - liquidation check (`needs_liquidation`)
//! - `check_order` margin sufficiency
//! - frozen-margin map insert
//! - persist event push (no real Postgres)
//! - `OrderResponse` construction
//!
//! What's excluded
//! - cast recv on the inbound side
//! - SPSC ring producer/consumer in/out of the shard
//! - cast send on the outbound (risk → ME)
//! - WAL append on the risk side (Recorder is the persister)
//!
//! Iteration shape: each iter calls `process_order` with a
//! fresh `order_id_lo`, then releases the frozen margin so
//! the frozen map doesn't grow unbounded across iterations.
//! The release is NOT part of the timed cost in the same
//! sense as a single producer→consumer leg; it's symmetric
//! upkeep to keep the bench stable.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_risk::Account;
use rsx_risk::FundingConfig;
use rsx_risk::LiquidationConfig;
use rsx_risk::OrderRequest;
use rsx_risk::ReplicationConfig;
use rsx_risk::RiskShard;
use rsx_risk::ShardConfig;
use rsx_risk::SymbolRiskParams;

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

fn bench_validate_and_forward(c: &mut Criterion) {
    let mut shard = make_shard(4);
    shard.accounts.insert(0, Account::new(0, 1_000_000));
    shard.mark_prices[0] = 50_000;

    let template = OrderRequest {
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
    c.bench_function("risk_validate_and_forward", |b| {
        b.iter(|| {
            oid += 1;
            let mut o = template;
            o.order_id_lo = oid;
            let resp = black_box(shard.process_order(black_box(&o)));
            // Symmetric release so frozen maps stay bounded
            // across iterations. Not part of the validate
            // leg; subtracts about a hash-map remove from
            // the reported number. See bench docstring.
            shard.release_frozen_for_order(0, 0, oid);
            resp
        });
    });
}

criterion_group!(benches, bench_validate_and_forward);
criterion_main!(benches);
