//! WAL random-access read (NAK cold-tier fallback path).
//!
//! What this measures
//! -----------------
//! `wal::read_record_at_seq(stream_id, target_seq, wal_dir, None)`
//! against a pre-populated WAL file of N records, for random
//! target seqs uniformly distributed across [1, N].
//!
//! This is the cold-tier hop in the two-tier NAK retransmit
//! horizon (CMP §10.6): if the requested seq has aged out of
//! the in-memory send-ring (4 096 slots), the sender falls
//! through to this path, opens the matching WAL file, and
//! linear-scans from the file start looking for the target
//! seq.
//!
//! The current implementation is O(records_in_file) per
//! lookup — there is no per-file index. This bench surfaces
//! that cost.
//!
//! Assumptions / caveats
//! --------------------
//! - We populate one 100k-record WAL file. Real production
//!   files are 64 MB ≈ 500k records of 128-byte payload; the
//!   cost scales linearly with file size.
//! - TempDir mount affects the page-cache hit/miss profile.
//!   After a `flush()`, the file is in the page cache; the
//!   bench will measure warm reads. Cold reads (after a
//!   `posix_fadvise(POSIX_FADV_DONTNEED)` or process restart)
//!   would be slower.
//! - The scan reads from byte 0 every call (no resume cursor).
//!   Mean seek depth = N/2.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_dxs::read_record_at_seq;
use rsx_dxs::WalWriter;
use rsx_messages::FillRecord;
use rsx_types::Price;
use rsx_types::Qty;
use tempfile::TempDir;

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

/// Deterministic xorshift64 — pseudo-random target seqs without
/// pulling in rand.
struct XorShift64(u64);
impl XorShift64 {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

fn bench_wal_random_read_10k(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        None,
        1024 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();
    for _ in 0..10_000u64 {
        let mut rec = fill_record();
        writer.append(&mut rec).unwrap();
    }
    writer.flush().unwrap();

    let mut rng = XorShift64(0xdeadbeef);

    c.bench_function("wal_random_read_10k", |b| {
        b.iter(|| {
            let seq = (rng.next() % 10_000) + 1;
            let rec = read_record_at_seq(
                1,
                black_box(seq),
                tmp.path(),
                None,
            )
            .unwrap();
            black_box(rec);
        });
    });
}

fn bench_wal_random_read_100k(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        None,
        1024 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();
    for _ in 0..100_000u64 {
        let mut rec = fill_record();
        writer.append(&mut rec).unwrap();
    }
    writer.flush().unwrap();

    let mut rng = XorShift64(0xdeadbeef);

    c.bench_function("wal_random_read_100k", |b| {
        b.iter(|| {
            let seq = (rng.next() % 100_000) + 1;
            let rec = read_record_at_seq(
                1,
                black_box(seq),
                tmp.path(),
                None,
            )
            .unwrap();
            black_box(rec);
        });
    });
}

criterion_group!(
    benches,
    bench_wal_random_read_10k,
    bench_wal_random_read_100k,
);
criterion_main!(benches);
