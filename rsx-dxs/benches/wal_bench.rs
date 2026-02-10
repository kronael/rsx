use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_dxs::*;
use std::mem;
use tempfile::TempDir;

fn fill_payload() -> Vec<u8> {
    let record = FillRecord {
        preamble: PayloadPreamble {
            seq: 1,
            ver: 1,
            kind: 0,
            _pad0: 0,
            len: mem::size_of::<FillRecord>() as u32,
        },
        ts_ns: 1000,
        symbol_id: 1,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 200,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100,
        price: 50000,
        qty: 100,
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    unsafe {
        std::slice::from_raw_parts(
            &record as *const FillRecord as *const u8,
            mem::size_of::<FillRecord>(),
        )
    }
    .to_vec()
}

fn bench_wal_append_in_memory(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();
    let payload = fill_payload();

    c.bench_function("wal_append_in_memory", |b| {
        b.iter(|| {
            let _ = writer
                .append(RECORD_FILL, &payload);
        });
    });
}

fn bench_wal_flush_fsync(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();
    let payload = fill_payload();

    c.bench_function("wal_flush_fsync_64kb", |b| {
        b.iter(|| {
            // fill ~64KB of buffer
            for _ in 0..800 {
                let _ = writer
                    .append(RECORD_FILL, &payload);
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
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();
    let payload = fill_payload();

    // write 10k records
    for _ in 0..10_000 {
        writer.append(RECORD_FILL, &payload).unwrap();
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
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();
    let payload = fill_payload();

    for _ in 0..100_000 {
        writer.append(RECORD_FILL, &payload).unwrap();
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
