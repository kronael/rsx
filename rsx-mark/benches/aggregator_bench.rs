//! Push synthetic Binance + Coinbase ticks through the real
//! aggregator path. The existing `mark_bench.rs` covers
//! source-mask + single/multi-source median; this bench
//! simulates the actual two-source production scenario the
//! mark process runs (Binance + Coinbase BTC/USDT):
//! alternating ticks, real staleness check, real median.
//!
//! Source IDs 0 and 1 correspond to Binance and Coinbase
//! in the mark process's source.rs registration order.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_mark::aggregator::aggregate_with_staleness;
use rsx_mark::aggregator::STALENESS_NS;
use rsx_mark::types::SourcePrice;
use rsx_mark::types::SymbolMarkState;

const BINANCE: u8 = 0;
const COINBASE: u8 = 1;
const SYMBOL_ID: u32 = 1;

fn tick(source_id: u8, price: i64, ts_ns: u64) -> SourcePrice {
    SourcePrice {
        symbol_id: SYMBOL_ID,
        source_id,
        price,
        timestamp_ns: ts_ns,
    }
}

/// Steady-state two-source mark price aggregation. Each iter
/// alternates Binance/Coinbase ticks, which is the production
/// pattern (independent WS streams, interleaved arrivals).
fn bench_binance_plus_coinbase(c: &mut Criterion) {
    c.bench_function(
        "mark_binance_plus_coinbase",
        |b| {
            let mut state = SymbolMarkState::new();
            // Prime with one tick from each source.
            let t0 = 1_700_000_000_000_000_000u64;
            aggregate_with_staleness(
                &mut state,
                tick(BINANCE, 50_000_00, t0),
                t0,
                SYMBOL_ID,
                STALENESS_NS,
            );
            aggregate_with_staleness(
                &mut state,
                tick(COINBASE, 50_005_00, t0),
                t0,
                SYMBOL_ID,
                STALENESS_NS,
            );

            let mut i: u64 = 0;
            b.iter(|| {
                i += 1;
                // Alternate source IDs each iter to match
                // the cross-exchange interleave.
                let src = if i % 2 == 0 {
                    BINANCE
                } else {
                    COINBASE
                };
                let ts = t0 + i * 1_000_000;
                // Wobble price by ±50 ticks around the
                // anchor so the median actually changes.
                let px = 50_000_00 + ((i % 100) as i64 - 50);
                let ev = aggregate_with_staleness(
                    black_box(&mut state),
                    black_box(tick(src, px, ts)),
                    ts,
                    SYMBOL_ID,
                    STALENESS_NS,
                );
                black_box(ev);
            });
        },
    );
}

/// 5-source aggregation (e.g. Binance + Coinbase + Kraken
/// + OKX + Bitstamp). Real median sort cost.
fn bench_5_source_steady_state(c: &mut Criterion) {
    c.bench_function(
        "mark_5_sources_steady_state",
        |b| {
            let mut state = SymbolMarkState::new();
            let t0 = 1_700_000_000_000_000_000u64;
            for src in 0..5u8 {
                aggregate_with_staleness(
                    &mut state,
                    tick(src, 50_000_00 + src as i64, t0),
                    t0,
                    SYMBOL_ID,
                    STALENESS_NS,
                );
            }
            let mut i: u64 = 0;
            b.iter(|| {
                i += 1;
                let src = (i % 5) as u8;
                let ts = t0 + i * 1_000_000;
                let px = 50_000_00 + ((i % 100) as i64 - 50);
                let ev = aggregate_with_staleness(
                    black_box(&mut state),
                    black_box(tick(src, px, ts)),
                    ts,
                    SYMBOL_ID,
                    STALENESS_NS,
                );
                black_box(ev);
            });
        },
    );
}

criterion_group!(
    benches,
    bench_binance_plus_coinbase,
    bench_5_source_steady_state,
);
criterion_main!(benches);
