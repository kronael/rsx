//! FM9 regression: WAL replay must preserve intra-level FIFO
//! (time-priority) order, not just the sorted (price, qty,
//! side) projection that `book_state()` compares.
//!
//! `replay_after_snapshot_test::book_state` sorts and drops
//! order identity, so a book that replayed a same-price level
//! in the WRONG order would still compare equal. This test
//! rests multiple orders at one price, replays into a fresh
//! book, then fires a taker that PARTIALLY consumes the level
//! and asserts the fills land on the resting orders in their
//! original arrival (time-priority) order.

use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::matching::IncomingOrder;
use rsx_book::matching::process_new_order;
use rsx_cast::wal::WalWriter;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::wal_integration::replay_wal_after_snapshot;
use rsx_matching::wal_integration::write_events_to_wal;
use rsx_matching::wal_integration::OrderKey;
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
    ts_ns: u64,
) {
    let mut accepted = OrderAcceptedRecord {
        seq: 0,
        ts_ns,
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
        timestamp_ns: ts_ns,
        order_id_hi: 0,
        order_id_lo: oid,
    };
    process_new_order(book, &mut incoming);
    write_events_to_wal(writer, book, SYM, 1).unwrap();
}

fn taker_buy(qty: i64) -> IncomingOrder {
    IncomingOrder {
        price: 100,
        qty,
        remaining_qty: qty,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 1,
        reduce_only: false,
        post_only: false,
        timestamp_ns: 9_000,
        order_id_hi: 0,
        order_id_lo: 999,
    }
}

#[test]
fn replay_preserves_intra_level_fifo() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().to_str().unwrap();

    let mut book = Orderbook::new(cfg(), 1024, 50_000);
    let mut writer = WalWriter::new(
        SYM, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    // Five sells at the SAME price, distinct users, strictly
    // increasing timestamps → arrival order is the FIFO order.
    let makers: Vec<u32> = vec![10, 11, 12, 13, 14];
    for (i, &uid) in makers.iter().enumerate() {
        submit(
            &mut book,
            &mut writer,
            uid,
            uid as u64,
            Side::Sell,
            100,
            10,
            1_000 + i as u64,
        );
    }
    writer.flush().unwrap();
    drop(writer);

    // Replay into a FRESH book from seq 1 (no snapshot).
    let mut recovered = Orderbook::new(cfg(), 1024, 50_000);
    let mut order_index: FxHashMap<OrderKey, u32> =
        FxHashMap::default();
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

    // Taker partially consumes the level: 25 lots against five
    // 10-lot rests → fills makers 10, 11, then half of 12.
    let mut taker = taker_buy(25);
    process_new_order(&mut recovered, &mut taker);

    let fill_makers: Vec<u32> = recovered
        .events()
        .iter()
        .filter_map(|e| match e {
            Event::Fill { maker_user_id, .. } => {
                Some(*maker_user_id)
            }
            _ => None,
        })
        .collect();

    // FIFO: fills must hit makers in arrival order. A replay
    // that scrambled same-price queue order would fill a
    // different subset and fail here, even though the lossy
    // sorted book_state() would still compare equal.
    assert_eq!(
        fill_makers,
        vec![10, 11, 12],
        "replay must preserve time-priority within the level",
    );
}
