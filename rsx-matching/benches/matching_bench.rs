//! Two ME micro-op groups that the accept-path bench aggregates but that
//! are worth isolating:
//!
//! - `dedup/*` — the duplicate-order guard (FxHashMap insert / hit /
//!   bulk cleanup). Every accepted order pays the check.
//! - `wal_events/*` — turning a book's emitted events into WAL records
//!   (`write_events_to_wal`) and draining the event buffer. This is the
//!   per-order event-emission cost the accept path incurs after a match.
//!
//! Pure orderbook data-structure micro-benches (slab alloc/free,
//! price->index compression) live in rsx-book's bench set, not here, to
//! keep matching's numbers about the matching engine.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::BatchSize;
use criterion::Criterion;
use rsx_book::matching::process_new_order;
use rsx_cast::wal::WalWriter;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::wal::write_events_to_wal;
use rsx_types::Side;
use rsx_types::TimeInForce;
use tempfile::TempDir;

#[path = "harness.rs"]
mod harness;

// --- Dedup group ------------------------------------------------------

fn bench_dedup(c: &mut Criterion) {
    harness::pin();
    let mut group = c.benchmark_group("dedup");

    group.bench_function("insert_new", |b| {
        let mut dedup = DedupTracker::new();
        let mut id = 0_u64;
        b.iter(|| {
            dedup.check_and_insert(1, 0, id);
            id += 1;
        });
    });

    group.bench_function("hit_duplicate", |b| {
        let mut dedup = DedupTracker::new();
        dedup.check_and_insert(1, 0, 42);
        b.iter(|| dedup.check_and_insert(1, 0, 42));
    });

    group.bench_function("cleanup_10k", |b| {
        b.iter(|| {
            let mut dedup = DedupTracker::new();
            for i in 0..10_000_u64 {
                dedup.check_and_insert(1, 0, i);
            }
            dedup
                .evict(std::time::Instant::now() + std::time::Duration::from_secs(1));
        });
    });

    group.finish();
}

// --- WAL + event-emission group --------------------------------------

/// A crossed book carrying `k` Fill events in its buffer, ready to be
/// serialized. Built by resting `k` asks then sweeping them with one
/// taker; `process_new_order` leaves the events in `book.events()`.
fn crossed_book(k: usize) -> rsx_book::book::Orderbook {
    let mut book = rsx_book::book::Orderbook::new(harness::config(), 65_536, harness::MID);
    for i in 0..k {
        book.insert_resting(
            harness::MID + 1 + i as i64,
            1,
            Side::Sell,
            0,
            200 + i as u32,
            false,
            1,
            0,
            2_000 + i as u64,
        );
    }
    let mut taker = harness::order(
        harness::MID + k as i64,
        k as i64,
        Side::Buy,
        TimeInForce::GTC,
        1,
        9_999,
    );
    process_new_order(&mut book, &mut taker);
    book
}

fn bench_wal_events(c: &mut Criterion) {
    harness::pin();
    let mut group = c.benchmark_group("wal_events");

    // Serialize one match's events (1 fill) to WAL, per iter.
    group.bench_function("append_1_fill", |b| {
        let tmp = TempDir::new().expect("tempdir");
        let mut writer =
            WalWriter::new(harness::SYMBOL_ID, tmp.path(), 64 * 1024 * 1024).expect("wal");
        let book = crossed_book(1);
        let mut n = 0_u64;
        b.iter(|| {
            write_events_to_wal(
                black_box(&mut writer),
                black_box(&book),
                harness::SYMBOL_ID,
                1_700_000_000_000_000_000,
            )
            .expect("write");
            n += 1;
            if n.is_multiple_of(1024) {
                writer.reset_write_buf();
            }
        });
    });

    // Drain the event buffer (what the risk/mkt fan-out iterates).
    for k in [10usize, 100] {
        group.bench_function(format!("drain_{k}_fills"), |b| {
            b.iter_batched(
                || crossed_book(k),
                |book| {
                    for e in book.events() {
                        black_box(e);
                    }
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = harness::criterion();
    targets = bench_dedup, bench_wal_events
}
criterion_main!(benches);
