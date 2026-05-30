//! Full ME accept path in one bench: dedup check +
//! `OrderAcceptedRecord` WAL append + `process_new_order` +
//! `write_events_to_wal` + `update_order_index`.
//!
//! This is everything the matching engine main loop runs
//! between `me_in` and `me_out` (sans cast send / per-stage
//! latency probes). All of it on real production code:
//! - real `Orderbook` (with pre-seeded resting liquidity)
//! - real `WalWriter` (tempdir-backed)
//! - real `DedupTracker`
//! - real FxHashMap order index
//!
//! Each iter timestamps one fresh inbound order with a unique
//! `(user_id, order_id_lo)` so dedup is always a miss, the
//! WAL append always grows the buffer, and the order index
//! actually does work.

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_cast::wal::WalWriter;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::wal_integration::write_events_to_wal;
use rsx_messages::OrderAcceptedRecord;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use rustc_hash::FxHashMap;
use std::path::PathBuf;

const SYMBOL_ID: u32 = 99;

type OrderKey = (u32, u64, u64);

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: SYMBOL_ID,
        price_decimals: 2,
        qty_decimals: 4,
        tick_size: 1,
        lot_size: 1,
    }
}

/// Mirror of the production `update_order_index` helper in
/// rsx-matching/src/main.rs.
fn update_index(
    events: &[Event],
    index: &mut FxHashMap<OrderKey, u32>,
) {
    for event in events {
        match *event {
            Event::OrderInserted {
                handle,
                user_id,
                order_id_hi,
                order_id_lo,
                ..
            } => {
                index.insert(
                    (user_id, order_id_hi, order_id_lo),
                    handle,
                );
            }
            Event::OrderDone {
                user_id,
                order_id_hi,
                order_id_lo,
                ..
            } => {
                index.remove(&(
                    user_id, order_id_hi, order_id_lo,
                ));
            }
            _ => {}
        }
    }
}

fn make_book_with_liquidity() -> Orderbook {
    let mut book = Orderbook::new(config(), 65_536, 100_000);
    // Seed 50 resting bids + 50 resting asks well off the
    // mid so per-iter inserts don't cross.
    for i in 0..50 {
        let mut bid = IncomingOrder {
            price: 99_500 - i as i64,
            qty: 100,
            remaining_qty: 100,
            side: Side::Buy,
            tif: TimeInForce::GTC,
            user_id: 100 + i as u32,
            reduce_only: false,
            post_only: false,
            timestamp_ns: 1_000_000,
            order_id_hi: 0,
            order_id_lo: 1_000 + i as u64,
        };
        process_new_order(&mut book, &mut bid);
        let mut ask = IncomingOrder {
            price: 100_500 + i as i64,
            // Big enough that 10M iter taker buys of qty=1
            // don't fully drain this ask level.
            qty: 1_000_000_000,
            remaining_qty: 1_000_000_000,
            side: Side::Sell,
            tif: TimeInForce::GTC,
            user_id: 200 + i as u32,
            reduce_only: false,
            post_only: false,
            timestamp_ns: 1_000_000,
            order_id_hi: 0,
            order_id_lo: 2_000 + i as u64,
        };
        process_new_order(&mut book, &mut ask);
    }
    book
}

/// Full GW-validated-order arrival -> ME emitted-events
/// pipeline. Measures the ME critical-section cost.
fn bench_me_accept_path(c: &mut Criterion) {
    let tmp = PathBuf::from("./tmp/bench_me_process");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    c.bench_function("me_process_order_full_path", |b| {
        let mut book = make_book_with_liquidity();
        let mut wal = WalWriter::new(
        SYMBOL_ID, &tmp, 64 * 1024 * 1024,
    )
        .expect("wal");
        let mut dedup = DedupTracker::new();
        let mut index: FxHashMap<OrderKey, u32> =
            FxHashMap::default();
        let mut counter: u64 = 1;

        b.iter(|| {
            counter += 1;
            // IOC buy at best ask: one fill, no resting.
            // Slab stays bounded across the criterion run.
            let user_id = 1_000 + (counter % 1024) as u32;
            let oid_lo = counter;

            let is_dup = dedup.check_and_insert(
                user_id,
                0,
                oid_lo,
            );
            black_box(is_dup);

            let mut accepted = OrderAcceptedRecord {
                seq: 0,
                ts_ns: 1_700_000_000_000_000_000,
                user_id,
                symbol_id: SYMBOL_ID,
                order_id_hi: 0,
                order_id_lo: oid_lo,
                price: 100_500,
                qty: 1,
                side: 0,
                tif: 1, // IOC
                reduce_only: 0,
                post_only: 0,
                cid: [0; 20],
            };
            {
                let framed = wal.prepare(&mut accepted).unwrap();
                wal.append_framed(&framed).unwrap();
            }

            let mut incoming = IncomingOrder {
                price: 100_500,
                qty: 1,
                remaining_qty: 1,
                side: Side::Buy,
                tif: TimeInForce::IOC,
                user_id,
                reduce_only: false,
                post_only: false,
                timestamp_ns: 1_700_000_000_000_000_000,
                order_id_hi: 0,
                order_id_lo: oid_lo,
            };
            process_new_order(
                black_box(&mut book),
                black_box(&mut incoming),
            );

            // Write emitted events to WAL.
            write_events_to_wal(
                &mut wal,
                &book,
                SYMBOL_ID,
                1_700_000_000_000_000_000,
            )
            .unwrap();

            // Drain the WAL buffer periodically WITHOUT fsync. The
            // per-order hot path is a memcpy into the buffer; fsync
            // is batched every 10ms off-path, so it must not be in
            // the timed loop. reset_write_buf clears the buffer with
            // no disk I/O, keeping it bounded across the criterion
            // run while preserving the true in-memory append cost.
            if counter % 1024 == 0 {
                wal.reset_write_buf();
            }

            // Maintain order index (O(1) cancels later).
            update_index(book.events(), &mut index);
        });
    });

    let _ = std::fs::remove_dir_all(&tmp);
}

criterion_group!(benches, bench_me_accept_path);
criterion_main!(benches);
