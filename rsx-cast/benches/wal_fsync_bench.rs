//! WAL flush+fsync amortization sweep: per-record cost vs batch size.
//!
//! Worker thread pinned to core 2 so the fsync work doesn't bounce
//! between cores mid-sample.
//!
//! What this measures
//! -----------------
//! Sweeps batch sizes [1, 10, 100, 1 000, 10 000] records per flush.
//! For each size, Criterion reports both wall-clock time per flush AND
//! per-record throughput (Throughput::Elements). This surfaces the
//! amortization curve: how does per-record cost fall as the batch grows?
//!
//! The "knee" — where doubling the batch no longer halves the per-record
//! cost — is the optimal flush interval for a given fsync budget.
//!
//! In production, WalWriter is flushed on a 10 ms timer. This bench
//! shows whether the batch size at that cadence is on the flat part of
//! the curve (fsync amortized, append-dominated) or still on the slope
//! (fsync still dominates per-record cost).
//!
//! Assumptions / caveats
//! --------------------
//! - TempDir on Linux can land on tmpfs (no real I/O) or on a real disk.
//!   On tmpfs, fsync is near-zero and per-record costs will look flat.
//!   On a real SSD, expect 20–200 µs fsync latency and a visible knee
//!   between batch=100 and batch=1000.
//! - FillRecord: 128 B payload + 16 B header = 144 B per record.
//! - Each iteration appends to a growing WAL file without rotation
//!   (max_file_size = u64::MAX). File growth per iter is bounded by
//!   batch * 144 B, which is small enough not to affect fsync timing.

use core_affinity::CoreId;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::BenchmarkId;
use criterion::Criterion;
use criterion::Throughput;
use rsx_cast::WalWriter;
use rsx_messages::FillRecord;
use rsx_types::Price;
use rsx_types::Qty;
use tempfile::TempDir;

fn pin_worker() {
    let ids = core_affinity::get_core_ids().unwrap_or_default();
    let core = ids.get(2).copied().unwrap_or(CoreId { id: 0 });
    core_affinity::set_for_current(core);
}

fn fill_record() -> FillRecord {
    FillRecord {
        seq: 0,
        ts_ns: 0,
        symbol_id: 1,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 200,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100,
        price: Price(50_000),
        qty: Qty(100),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        taker_ts_ns: 0,
    }
}

fn bench_flush_interval(c: &mut Criterion) {
    pin_worker();

    let batches: &[(u64, &str)] = &[
        (1, "1rec"),
        (10, "10rec"),
        (100, "100rec"),
        (1_000, "1k_rec"),
        (10_000, "10k_rec"),
    ];

    let mut group = c.benchmark_group("wal_flush_interval");

    for &(batch, label) in batches {
        let tmp = TempDir::new().unwrap();
        let mut writer = WalWriter::new(1, tmp.path(), u64::MAX).unwrap();
        let mut rec = fill_record();

        group.throughput(Throughput::Elements(batch));
        group.bench_with_input(BenchmarkId::from_parameter(label), &batch, |b, &batch| {
            b.iter(|| {
                for _ in 0..batch {
                    let framed = writer
                        .prepare(&mut rec)
                        .expect("WAL prepare must not fail mid-bench");
                    writer
                        .append_framed(&framed)
                        .expect("WAL append must not fail mid-bench");
                }
                writer.flush().expect("WAL flush must not fail mid-bench");
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_flush_interval);
criterion_main!(benches);
