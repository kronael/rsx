//! Checkpoint restore plus WAL replay must reproduce live book state.

use apache_avro::types::Value;
use apache_avro::Reader;
use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_book::snapshot;
use rsx_cast::wal::WalWriter;
use rsx_health::Gauge;
use rsx_matching::checkpoint::load_checkpoint;
use rsx_matching::checkpoint::save_checkpoint;
use rsx_matching::checkpoint::CheckpointWriter;
use rsx_matching::checkpoint::MatchingCheckpointV1;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::wal::rebuild_dedup_window;
use rsx_matching::wal::replay_wal_after_snapshot;
use rsx_matching::wal::write_events_to_wal;
use rsx_matching::wal::OrderKey;
use rsx_messages::OrderAcceptedRecord;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use rustc_hash::FxHashMap;
use std::sync::Arc;
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
        _pad: [0; 48],
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
    save_checkpoint(&book, wal_dir, SYM, writer.last_seq()).unwrap();
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

    let (loaded, wal_seq) = load_checkpoint(wal_dir, SYM, 1024)
        .unwrap()
        .expect("checkpoint exists");
    let mut recovered = *loaded;
    assert_eq!(
        book_state(&recovered),
        pre_snap,
        "snapshot alone replays the pre-snap state",
    );

    // 4. Replay the post-snap tail.
    let start = wal_seq + 1;
    let mut order_index: FxHashMap<OrderKey, u32> = FxHashMap::default();
    replay_wal_after_snapshot(&mut recovered, &mut order_index, wal_dir, SYM, start).unwrap();
    assert_eq!(
        book_state(&recovered),
        live,
        "snapshot + replay must equal live state",
    );
}

#[test]
fn incomplete_next_checkpoint_does_not_replace_last_good_checkpoint() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().to_str().unwrap();
    let mut book = Orderbook::new(cfg(), 1024, 50_000);
    let handle = book.insert_resting(100, 5, Side::Buy, 0, 10, false, 1, 0, 1);
    save_checkpoint(&book, wal_dir, SYM, 17).unwrap();

    let next_tmp = tmp.path().join(SYM.to_string()).join("snapshot.bin.tmp");
    std::fs::write(next_tmp, b"partial next checkpoint").unwrap();

    let (loaded, wal_seq) = load_checkpoint(wal_dir, SYM, 1024)
        .unwrap()
        .expect("last checkpoint remains");
    assert_eq!(wal_seq, 17);
    assert!(loaded.orders.get(handle).is_active());
}

#[test]
fn checkpoint_is_self_describing_avro_v1() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().to_str().unwrap();
    let mut config = cfg();
    config.symbol_id = u32::MAX;
    let mut book = Orderbook::new(config, 1024, 50_000);
    let handle = book.insert_resting(
        100,
        5,
        Side::Buy,
        0,
        u32::MAX,
        false,
        u64::MAX,
        u64::MAX,
        u64::MAX - 1,
    );
    book.orders.get_mut(handle).sequence = u32::MAX;
    save_checkpoint(&book, wal_dir, SYM, u64::MAX).unwrap();

    let path = tmp.path().join(SYM.to_string()).join("snapshot.bin");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], b"Obj\x01");

    let file = std::fs::File::open(path).unwrap();
    let mut reader = Reader::new(file).unwrap();
    assert!(reader
        .writer_schema()
        .canonical_form()
        .contains("MatchingCheckpointV1"));
    let value = reader.next().unwrap().unwrap();
    let Value::Record(fields) = value else {
        panic!("checkpoint record expected");
    };
    assert!(fields
        .iter()
        .any(|(name, value)| name == "schema_version" && *value == Value::Int(1)));
    assert!(reader.next().is_none());

    let (loaded, wal_seq) = load_checkpoint(wal_dir, SYM, 1024).unwrap().unwrap();
    assert_eq!(wal_seq, u64::MAX);
    assert_eq!(loaded.orders.get(0).timestamp_ns, u64::MAX);
    assert_eq!(loaded.orders.get(0).order_id_hi, u64::MAX);
    assert_eq!(loaded.orders.get(0).order_id_lo, u64::MAX - 1);
    assert_eq!(loaded.orders.get(0).user_id, u32::MAX);
    assert_eq!(loaded.orders.get(0).sequence, u32::MAX);
    assert_eq!(loaded.config.symbol_id, u32::MAX);
}

#[test]
fn checkpoint_worker_persists_final_capture_before_join() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().to_str().unwrap();
    let mut book = Orderbook::new(cfg(), 1024, 50_000);
    book.insert_resting(100, 5, Side::Buy, 0, 10, false, 1, 0, 1);

    let persisted = Arc::new(Gauge::default());
    let worker = CheckpointWriter::start(wal_dir, SYM, persisted.clone()).unwrap();
    let first = MatchingCheckpointV1::new(7, snapshot::capture(&book).unwrap());
    worker.submit(first).unwrap();
    book.insert_resting(99, 3, Side::Buy, 0, 11, false, 2, 0, 2);
    let final_checkpoint = MatchingCheckpointV1::new(11, snapshot::capture(&book).unwrap());
    worker.shutdown(Some(final_checkpoint)).unwrap();

    let (loaded, wal_seq) = load_checkpoint(wal_dir, SYM, 1024).unwrap().unwrap();
    assert_eq!(wal_seq, 11);
    assert_eq!(persisted.get(), 11);
    assert_eq!(book_state(&loaded), book_state(&book));
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
    replay_wal_after_snapshot(&mut recovered, &mut order_index, wal_dir, SYM, 1).unwrap();
    assert_eq!(book_state(&recovered), live);
}

/// An accepted order inside the dedup window remains protected after recovery,
/// even when its record precedes the checkpoint boundary.
#[test]
fn dedup_window_rebuilt_from_wal_for_pre_snapshot_order() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().to_str().unwrap();

    let mut book = Orderbook::new(cfg(), 1024, 50_000);
    let mut writer = WalWriter::new(SYM, tmp.path(), 64 * 1024 * 1024).unwrap();

    // Accept order A (writes RECORD_ORDER_ACCEPTED to the WAL), snapshot,
    // then no further activity for A. Crash + restart.
    submit(&mut book, &mut writer, 10, 1, Side::Buy, 100, 5);
    writer.flush().unwrap();
    save_checkpoint(&book, wal_dir, SYM, writer.last_seq()).unwrap();
    drop(writer);

    // Recover the dedup window from the WAL. The harness stamps every
    // accepted record with ts_ns=1, so a restart clock of 1 keeps A's age
    // at 0 (well inside DEDUP_WINDOW).
    let mut dedup = DedupTracker::new();
    let seeded = rebuild_dedup_window(&mut dedup, wal_dir, SYM, 1).unwrap();
    assert_eq!(seeded, 1, "one accepted order in the window");

    // A's (user, oid) is now known → a client resend is deduped.
    assert!(
        dedup.check_and_insert(10, 0, 1),
        "resend of pre-snapshot order A must be deduped after recovery",
    );
    // An unrelated order is unaffected.
    assert!(!dedup.check_and_insert(10, 0, 2));
}
