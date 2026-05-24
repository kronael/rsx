//! WAL flush+fsync cost per single record.
//!
//! Worker thread pinned to core 2 so the fsync work doesn't bounce
//! between cores mid-sample.
//!
//! What this measures
//! -----------------
//! `WalWriter::append(&mut record)` followed by an explicit
//! `flush()` (which calls `sync_all()` under the hood) per
//! iteration. The existing `wal_bench::bench_wal_append_in_memory`
//! only times the `Vec::extend_from_slice` cost (~31 ns); this
//! bench captures what it actually takes to durably persist
//! one record.
//!
//! Use case: this is the durability floor for any "Recorder
//! flushed N records" or "WAL fsync took X ms" claim. In
//! production WalWriter batches via a 10ms flush cadence,
//! so the per-record amortized cost is much lower; this
//! number is the *unamortized* cost.
//!
//! Assumptions / caveats
//! --------------------
//! - `TempDir` on a Linux box can land on tmpfs (no real
//!   fsync) or on a real disk (ext4/xfs, real fsync). Print
//!   the tempdir mount once with `df -T $(mktemp -d)` to
//!   know which kernel cache lives below. On a real SSD,
//!   expect 20-200µs per fsync; on tmpfs, ~µs.
//! - We use a FillRecord (128 bytes payload + 16-byte header
//!   = 144 bytes per record). Not 64 bytes — this is the
//!   realistic per-record cost.
//! - We do NOT exercise rotation; max_file_size is set high
//!   so the bench stays in a single file.

use core_affinity::CoreId;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
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

fn bench_wal_append_fsync_single(c: &mut Criterion) {
    pin_worker();
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        None,
        1024 * 1024 * 1024, // 1 GiB — never rotate
        600_000_000_000,
    )
    .unwrap();

    // Pre-build the record outside the timed loop. seq gets
    // overwritten by append, so reusing one instance is safe.
    let mut rec = fill_record();
    c.bench_function("wal_append_fsync_single", |b| {
        b.iter(|| {
            writer.append(&mut rec).unwrap();
            writer.flush().unwrap();
        });
    });
}

fn bench_wal_append_fsync_batch_100(c: &mut Criterion) {
    pin_worker();
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        None,
        1024 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();

    // Pre-build the record. Same rationale as the single-append bench.
    let mut rec = fill_record();
    c.bench_function("wal_append_fsync_batch_100", |b| {
        b.iter(|| {
            for _ in 0..100 {
                writer.append(&mut rec).unwrap();
            }
            writer.flush().unwrap();
        });
    });
}

criterion_group!(
    benches,
    bench_wal_append_fsync_single,
    bench_wal_append_fsync_batch_100,
);
criterion_main!(benches);
