use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_mark::aggregator::aggregate;
use rsx_mark::aggregator::compute_mask;
use rsx_mark::aggregator::sweep_stale;
use rsx_mark::aggregator::STALENESS_NS;
use rsx_mark::types::SourcePrice;
use rsx_mark::types::SymbolMarkState;

fn sp(id: u8, price: i64, ts: u64) -> SourcePrice {
    SourcePrice {
        symbol_id: 0,
        source_id: id,
        price,
        timestamp_ns: ts,
    }
}

/// <100ns target
fn bench_aggregate_single_source(c: &mut Criterion) {
    c.bench_function(
        "aggregate_single_source",
        |b| {
            let now = 1_000_000_000u64;
            let update = sp(0, 50000, now);
            b.iter(|| {
                let mut state = SymbolMarkState::new();
                black_box(aggregate(
                    &mut state,
                    black_box(update),
                    now,
                    1,
                ))
            })
        },
    );
}

/// <500ns target
fn bench_aggregate_3_sources_median(c: &mut Criterion) {
    c.bench_function(
        "aggregate_3_sources_median",
        |b| {
            let now = 1_000_000_000u64;
            b.iter(|| {
                let mut state = SymbolMarkState::new();
                aggregate(
                    &mut state,
                    sp(0, 100, now),
                    now,
                    0,
                );
                aggregate(
                    &mut state,
                    sp(1, 300, now),
                    now,
                    0,
                );
                black_box(aggregate(
                    &mut state,
                    sp(2, 200, now),
                    now,
                    0,
                ))
            })
        },
    );
}

/// <500ns target
fn bench_aggregate_8_sources_median(c: &mut Criterion) {
    c.bench_function(
        "aggregate_8_sources_median",
        |b| {
            let now = 1_000_000_000u64;
            b.iter(|| {
                let mut state = SymbolMarkState::new();
                for i in 0..7u8 {
                    aggregate(
                        &mut state,
                        sp(i, 1000 + i as i64 * 10, now),
                        now,
                        0,
                    );
                }
                black_box(aggregate(
                    &mut state,
                    sp(7, 1070, now),
                    now,
                    0,
                ))
            })
        },
    );
}

/// <50us target
fn bench_staleness_sweep_100_symbols(c: &mut Criterion) {
    c.bench_function(
        "staleness_sweep_100_symbols",
        |b| {
            let t0 = 1_000_000_000u64;
            let t1 = t0 + 500_000_000;
            b.iter_batched(
                || {
                    let mut states: Vec<SymbolMarkState> =
                        (0..100)
                            .map(|_| SymbolMarkState::new())
                            .collect();
                    for (i, s) in
                        states.iter_mut().enumerate()
                    {
                        aggregate(
                            s,
                            sp(0, 1000 + i as i64, t0),
                            t0,
                            i as u32,
                        );
                    }
                    states
                },
                |mut states| {
                    for (i, s) in
                        states.iter_mut().enumerate()
                    {
                        black_box(sweep_stale(
                            s,
                            t1,
                            i as u32,
                        ));
                    }
                },
                criterion::BatchSize::SmallInput,
            )
        },
    );
}

/// <100us target. Source update through aggregation
/// for 100 symbols (simulating full pipeline without
/// actual WAL/network).
fn bench_source_to_publish_e2e(c: &mut Criterion) {
    c.bench_function("source_to_publish_e2e", |b| {
        let now = 1_000_000_000u64;
        b.iter_batched(
            || {
                let mut states: Vec<SymbolMarkState> =
                    (0..100)
                        .map(|_| SymbolMarkState::new())
                        .collect();
                // Pre-populate 3 sources per symbol
                for (i, s) in
                    states.iter_mut().enumerate()
                {
                    for j in 0..3u8 {
                        aggregate(
                            s,
                            sp(
                                j,
                                1000
                                    + i as i64 * 10
                                    + j as i64,
                                now,
                            ),
                            now,
                            i as u32,
                        );
                    }
                }
                states
            },
            |mut states| {
                let t = now + 1;
                for (i, s) in
                    states.iter_mut().enumerate()
                {
                    black_box(aggregate(
                        s,
                        sp(0, 2000 + i as i64, t),
                        t,
                        i as u32,
                    ));
                }
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

// bench_wal_append_mark_event: requires WalWriter which
// needs filesystem I/O setup. Skipped -- WAL append is
// benchmarked in rsx-dxs/benches/wal_bench.rs already.

// bench_main_loop_idle: requires full runtime setup with
// SPSC rings, WalWriter, CmpSender. The main loop is in
// the binary, not exposed as a library function. Skipped.

/// <50ns target
fn bench_source_mask_computation(c: &mut Criterion) {
    c.bench_function("source_mask_computation", |b| {
        let now = 1_000_000_000u64;
        let mut state = SymbolMarkState::new();
        for i in 0..8u8 {
            state.sources[i as usize] =
                Some(sp(i, 100 + i as i64, now));
        }
        b.iter(|| {
            black_box(compute_mask(
                black_box(&state),
                now,
                STALENESS_NS,
            ))
        })
    });
}

criterion_group!(
    benches,
    bench_aggregate_single_source,
    bench_aggregate_3_sources_median,
    bench_aggregate_8_sources_median,
    bench_staleness_sweep_100_symbols,
    bench_source_to_publish_e2e,
    bench_source_mask_computation,
);
criterion_main!(benches);
