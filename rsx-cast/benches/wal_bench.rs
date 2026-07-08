//! WAL micro-ops with per-record throughput reporting.
//!
//! Groups:
//!   wal_write — append_1rec (~31 ns), flush_800rec (~1 ms, 115 KB),
//!               write_1m_no_flush (~35 ms)
//!   wal_read  — replay at 10k / 100k / 1m record scale (~720 Kelem/s)
//!
//! All benches use Throughput::Elements so Criterion reports
//! both wall-clock time AND records/second.
//!
//! Worker thread pinned to core 2 for measurement stability.

use core_affinity::CoreId;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::BenchmarkId;
use criterion::Criterion;
use criterion::Throughput;
use rsx_cast::WalReader;
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
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    }
}

fn write_records(writer: &mut WalWriter, count: u64) {
    let mut record = fill_record();
    for _ in 0..count {
        let framed = writer.prepare(&mut record).unwrap();
        writer.append_framed(&framed).unwrap();
    }
}

fn bench_wal_write(c: &mut Criterion) {
    pin_worker();
    let mut group = c.benchmark_group("wal_write");

    // Single in-memory append: prepare + append_framed, no I/O.
    // reset_write_buf() prevents unbounded allocation across Criterion warmup.
    {
        let tmp = TempDir::new().unwrap();
        let mut writer = WalWriter::new(1, tmp.path(), u64::MAX).unwrap();
        let mut record = fill_record();
        group.throughput(Throughput::Elements(1));
        group.bench_function("append_1rec", |b| {
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

    // 800 records buffered then flush+fsync (~115 KB).
    // 800 * (128 B FillRecord + 16 B WalHeader) = 112.5 KiB per flush.
    {
        let tmp = TempDir::new().unwrap();
        let mut writer = WalWriter::new(1, tmp.path(), u64::MAX).unwrap();
        let mut record = fill_record();
        group.throughput(Throughput::Elements(800));
        group.bench_function("flush_800rec", |b| {
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

    // 1M records, no flush — pure in-memory write throughput.
    // reset_write_buf() per iter keeps allocation bounded.
    // At ~31 ns/rec: ~31 ms per iteration.
    {
        let tmp = TempDir::new().unwrap();
        let mut writer = WalWriter::new(1, tmp.path(), u64::MAX).unwrap();
        let mut record = fill_record();
        group.throughput(Throughput::Elements(1_000_000));
        group.bench_function("write_1m_no_flush", |b| {
            b.iter(|| {
                writer.reset_write_buf();
                for _ in 0..1_000_000 {
                    let framed = writer
                        .prepare(&mut record)
                        .expect("WAL prepare must not fail mid-bench");
                    writer
                        .append_framed(&framed)
                        .expect("WAL append must not fail mid-bench");
                }
            });
        });
    }

    group.finish();
}

fn bench_wal_read(c: &mut Criterion) {
    pin_worker();
    let mut group = c.benchmark_group("wal_read");

    let counts: &[(u64, &str)] = &[(10_000, "10k"), (100_000, "100k"), (1_000_000, "1m")];

    for &(count, label) in counts {
        let tmp = TempDir::new().unwrap();
        let mut writer = WalWriter::new(1, tmp.path(), 512 * 1024 * 1024).unwrap();
        write_records(&mut writer, count);
        writer.flush().unwrap();

        group.throughput(Throughput::Elements(count));
        group.bench_with_input(BenchmarkId::new("replay", label), &count, |b, &expected| {
            b.iter(|| {
                let mut reader = WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
                let mut n = 0u64;
                while let Ok(Some(_)) = reader.next() {
                    n += 1;
                }
                assert_eq!(n, expected);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_wal_write, bench_wal_read);
criterion_main!(benches);
