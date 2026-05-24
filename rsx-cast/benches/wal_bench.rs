//! WAL micro-ops: in-memory append (~35 ns), ~115 KB flush+fsync (~1 ms), 10 K sequential read (~14 ms), 100 K replay (~138 ms).
//!
//! Worker thread pinned to core 2 for measurement stability.
//!
//! See `docs/benches.md` for the full bench index +
//! production-leg attribution.

use core_affinity::CoreId;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_cast::*;
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
        seq: 1,
        ts_ns: 1000,
        symbol_id: 1,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 200,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100,
        price: Price(50000),
        qty: Qty(100),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
taker_ts_ns: 0,
    }
}

fn bench_wal_append_in_memory(c: &mut Criterion) {
    pin_worker();
    let tmp = TempDir::new().unwrap();
    // u64::MAX makes the backpressure ceiling usize::MAX (via
    // saturating_mul) so warmup never hits WouldBlock regardless
    // of iteration count.
    let mut writer = WalWriter::new(
        1, tmp.path(), u64::MAX,
    )
    .unwrap();

    // Pre-build the record outside the timed loop; append mutates
    // its seq each call, so re-using one instance is fine.
    // reset_write_buf() at the start of each iter discards the
    // accumulated in-memory buffer so Criterion warmup (~100M
    // iters × 144B) doesn't exhaust RAM.
    let mut record = fill_record();
    c.bench_function("wal_append_in_memory", |b| {
        b.iter(|| {
            writer.reset_write_buf();
            let framed = writer
                .prepare(&mut record)
                .expect("INVARIANT: WAL prepare must not fail mid-bench");
            writer
                .append_framed(&framed)
                .expect("INVARIANT: WAL append must not fail mid-bench");
        });
    });
}

fn bench_wal_flush_fsync(c: &mut Criterion) {
    pin_worker();
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), u64::MAX,
    )
    .unwrap();

    // Pre-build the record. Each iter appends 800 records, then
    // flush+fsync. 800 * 144 B (128 B FillRecord + 16 B WalHeader)
    // ≈ 112.5 KiB per flush — see fn name. Previous "64kb" name
    // was inaccurate (assumed 64 B record).
    let mut record = fill_record();
    c.bench_function("wal_flush_fsync_115kb", |b| {
        b.iter(|| {
            for _ in 0..800 {
                let framed = writer
                    .prepare(&mut record)
                    .expect("WAL prepare must not fail mid-bench");
                writer
                    .append_framed(&framed)
                    .expect("WAL append must not fail mid-bench");
            }
            writer.flush().expect("WAL flush must not fail mid-bench");
        });
    });
}

fn bench_wal_read_sequential(c: &mut Criterion) {
    pin_worker();
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    // write 10k records
    for _ in 0..10_000 {
        let mut record = fill_record();
        {
            let framed = writer.prepare(&mut record).unwrap();
            writer.append_framed(&framed).unwrap();
        }
    }
    writer.flush().unwrap();

    c.bench_function("wal_read_sequential_10k", |b| {
        b.iter(|| {
            let mut reader = WalReader::open_from_seq(
                1,
                0,
                tmp.path(),
            )
            .unwrap();
            let mut count = 0;
            while let Ok(Some(_)) = reader.next() {
                count += 1;
            }
            assert_eq!(count, 10_000);
        });
    });
}

fn bench_replay_100k_records(c: &mut Criterion) {
    pin_worker();
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    for _ in 0..100_000 {
        let mut record = fill_record();
        {
            let framed = writer.prepare(&mut record).unwrap();
            writer.append_framed(&framed).unwrap();
        }
    }
    writer.flush().unwrap();

    c.bench_function("replay_100k_records", |b| {
        b.iter(|| {
            let mut reader = WalReader::open_from_seq(
                1,
                0,
                tmp.path(),
            )
            .unwrap();
            let mut count = 0;
            while let Ok(Some(_)) = reader.next() {
                count += 1;
            }
            assert_eq!(count, 100_000);
        });
    });
}

criterion_group!(
    benches,
    bench_wal_append_in_memory,
    bench_wal_flush_fsync,
    bench_wal_read_sequential,
    bench_replay_100k_records
);
criterion_main!(benches);
