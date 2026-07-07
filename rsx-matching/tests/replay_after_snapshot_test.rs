//! R-N1 regression test: ME must replay WAL after snapshot.
//!
//! Acceptance: snapshot is taken, then more orders are
//! appended to the WAL, then we simulate a "crash" by tearing
//! down the writer. On restart, `load_snapshot` +
//! `replay_wal_after_snapshot` together must produce a book
//! identical to the pre-crash live state.

use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_cast::wal::WalWriter;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::wal::load_snapshot;
use rsx_matching::wal::load_wal_seq;
use rsx_matching::wal::replay_wal_after_snapshot;
use rsx_matching::wal::save_snapshot;
use rsx_matching::wal::write_events_to_wal;
use rsx_matching::wal::OrderKey;
use rsx_messages::OrderAcceptedRecord;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use rustc_hash::FxHashMap;
use tempfile::TempDir;

const SYM: u32 = 1;

fn cfg() -> SymbolConfig {
    SymbolConfig {
        symbol_id: SYM,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 1,
        lot_size: 1,
    }
}

fn submit(
    book: &mut Orderbook,
    writer: &mut WalWriter,
    uid: u32,
    oid: u64,
    side: Side,
    px: i64,
    qty: i64,
) {
    let mut accepted = OrderAcceptedRecord {
        seq: 0,
        ts_ns: 1,
        user_id: uid,
        symbol_id: SYM,
        order_id_hi: 0,
        order_id_lo: oid,
        price: px,
        qty,
        side: match side {
            Side::Buy => 0,
            Side::Sell => 1,
        },
        tif: 0,
        reduce_only: 0,
        post_only: 0,
        cid: [0; 20],
    };
    {
        let framed = writer.prepare(&mut accepted).unwrap();
        writer.append_framed(&framed).unwrap();
    }
    let mut incoming = IncomingOrder {
        price: px,
        qty,
        remaining_qty: qty,
        side,
        tif: TimeInForce::GTC,
        user_id: uid,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 1,
        order_id_hi: 0,
        order_id_lo: oid,
    };
    process_new_order(book, &mut incoming);
    write_events_to_wal(writer, book, SYM, 1).unwrap();
}

/// Capture observable book state (BBO + resting volumes per
/// price level) for equality testing. Excludes
/// implementation-internal counters that may differ between
/// the live path and a replay path even when the book is
/// functionally identical.
fn book_state(book: &Orderbook) -> Vec<(i64, i64, u8)> {
    let mut out: Vec<(i64, i64, u8)> = Vec::new();
    for i in 0..book.orders.len() {
        let slot = book.orders.get(i);
        if slot.is_active() {
            out.push((slot.price.0, slot.remaining_qty.0, slot.side));
        }
    }
    out.sort();
    out
}

#[test]
fn replay_restores_orders_appended_after_snapshot() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().to_str().unwrap();

    let mut book = Orderbook::new(cfg(), 1024, 50_000);
    let mut writer = WalWriter::new(SYM, tmp.path(), 64 * 1024 * 1024).unwrap();

    // 1. Resting buy, then snapshot.
    submit(&mut book, &mut writer, 10, 1, Side::Buy, 100, 5);
    writer.flush().unwrap();
    save_snapshot(&book, wal_dir, SYM, writer.last_seq()).unwrap();
    let pre_snap = book_state(&book);

    // 2. Two further orders: one rests at a new price, one
    //    crosses the snapshot's resting order. Replay must
    //    reproduce the partial-fill effect on the snapshot's
    //    resting order.
    submit(&mut book, &mut writer, 10, 2, Side::Buy, 99, 3);
    submit(&mut book, &mut writer, 20, 3, Side::Sell, 99, 2);
    writer.flush().unwrap();
    let live = book_state(&book);
    assert_ne!(
        pre_snap, live,
        "post-snap activity should have moved the book"
    );

    // 3. Crash + restart.
    drop(writer);

    let loaded = load_snapshot(wal_dir, SYM).expect("snapshot exists");
    let mut recovered = *loaded;
    assert_eq!(
        book_state(&recovered),
        pre_snap,
        "snapshot alone replays the pre-snap state",
    );

    // 4. Replay the post-snap tail.
    let start = load_wal_seq(wal_dir, SYM).unwrap() + 1;
    let mut order_index: FxHashMap<OrderKey, u32> = FxHashMap::default();
    let mut dedup = DedupTracker::new();
    replay_wal_after_snapshot(
        &mut recovered,
        &mut order_index,
        &mut dedup,
        wal_dir,
        SYM,
        start,
    )
    .unwrap();
    assert_eq!(
        book_state(&recovered),
        live,
        "snapshot + replay must equal live state",
    );
}

#[test]
fn replay_with_no_snapshot_replays_from_seq_1() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().to_str().unwrap();

    let mut book = Orderbook::new(cfg(), 1024, 50_000);
    let mut writer = WalWriter::new(SYM, tmp.path(), 64 * 1024 * 1024).unwrap();

    submit(&mut book, &mut writer, 10, 1, Side::Buy, 100, 5);
    submit(&mut book, &mut writer, 20, 2, Side::Sell, 101, 7);
    writer.flush().unwrap();
    let live = book_state(&book);

    drop(writer);

    // No snapshot present. Cold-start full replay from seq 1.
    let mut recovered = Orderbook::new(cfg(), 1024, 50_000);
    let mut order_index: FxHashMap<OrderKey, u32> = FxHashMap::default();
    let mut dedup = DedupTracker::new();
    replay_wal_after_snapshot(
        &mut recovered,
        &mut order_index,
        &mut dedup,
        wal_dir,
        SYM,
        1,
    )
    .unwrap();
    assert_eq!(book_state(&recovered), live);
}
