//! WAL micro-ops: in-memory append (the 31 ns figure), ~115 KB flush+fsync, 10 K sequential read, 100 K replay.
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
    // max_file_size = 1 GiB. WalWriter::append returns WouldBlock
    // once the in-memory buf exceeds 2 * max_file_size. At 144 B per
    // append that's ~14.9M iters before backpressure, well above any
    // Criterion sample. Previous 64 MiB cap (= ~880k iters) was
    // marginal and the silent `let _ =` hid the failure.
    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        None,
        1024 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();

    // Pre-build the record outside the timed loop; append mutates
    // its seq each call, so re-using one instance is fine.
    let mut record = fill_record();
    c.bench_function("wal_append_in_memory", |b| {
        b.iter(|| {
            writer
                .append(&mut record)
                .expect("INVARIANT: WAL append must not fail mid-bench");
        });
    });
}

fn bench_wal_flush_fsync(c: &mut Criterion) {
    pin_worker();
    let tmp = TempDir::new().unwrap();
    // 1 GiB cap so the writer never rotates inside the bench.
    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        None,
        1024 * 1024 * 1024,
        600_000_000_000,
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
                writer
                    .append(&mut record)
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
        1,
        tmp.path(),
        None,
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();

    // write 10k records
    for _ in 0..10_000 {
        let mut record = fill_record();
        writer.append(&mut record).unwrap();
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
        1,
        tmp.path(),
        None,
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();

    for _ in 0..100_000 {
        let mut record = fill_record();
        writer.append(&mut record).unwrap();
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
