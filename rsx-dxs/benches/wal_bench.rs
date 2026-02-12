use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_dxs::*;
use rsx_types::Price;
use rsx_types::Qty;
use std::mem;
use tempfile::TempDir;

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
    }
}

fn bench_wal_append_in_memory(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        None,
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();

    c.bench_function("wal_append_in_memory", |b| {
        b.iter(|| {
            let mut record = fill_record();
            let _ = writer.append(&mut record);
        });
    });
}

fn bench_wal_flush_fsync(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        None,
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();

    c.bench_function("wal_flush_fsync_64kb", |b| {
        b.iter(|| {
            // fill ~64KB of buffer
            for _ in 0..800 {
                let mut record = fill_record();
                let _ = writer.append(&mut record);
            }
            writer.flush().unwrap();
        });
    });
}

fn bench_wal_read_sequential(c: &mut Criterion) {
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
