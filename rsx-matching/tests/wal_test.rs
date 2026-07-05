use rsx_types::Price;
use rsx_types::Qty;
use rsx_book::book::Orderbook;
use rsx_book::matching::IncomingOrder;
use rsx_book::matching::process_new_order;
use rsx_messages::FillRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_cast::decode_payload;
use rsx_cast::wal::WalReader;
use rsx_cast::wal::WalWriter;
use rsx_matching::wal::flush_if_due;
use rsx_matching::wal::write_events_to_wal;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;

fn test_config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 1,
        lot_size: 1,
    }
}

fn test_book() -> Orderbook {
    Orderbook::new(test_config(), 1024, 50_000)
}

#[test]
fn wal_records_written_for_all_event_types() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    let mut book = test_book();

    // Resting sell -> buy crosses -> fill + done events
    book.insert_resting(
        50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0,
    );
    let mut order = IncomingOrder {
        price: 50_100,
        qty: 50,
        remaining_qty: 50,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 2,
        reduce_only: false,
        post_only: false,
        order_id_hi: 0,
        order_id_lo: 0,
        timestamp_ns: 1000,
    };
    process_new_order(&mut book, &mut order);

    write_events_to_wal(
        &mut writer, &book, 1, 1000,
    )
    .unwrap();
    writer.flush().unwrap();

    // Now insert a resting order (no cross)
    let mut order2 = IncomingOrder {
        price: 49_900,
        qty: 100,
        remaining_qty: 100,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 3,
        reduce_only: false,
        post_only: false,
        order_id_hi: 0,
        order_id_lo: 0,
        timestamp_ns: 2000,
    };
    process_new_order(&mut book, &mut order2);
    write_events_to_wal(
        &mut writer, &book, 1, 2000,
    )
    .unwrap();
    writer.flush().unwrap();

    // Read back all records
    let mut reader =
        WalReader::open_from_seq(1, 0, tmp.path())
            .unwrap();
    let mut count = 0;
    while let Ok(Some(_)) = reader.next() {
        count += 1;
    }
    // Fill + OrderDone(taker) + OrderInserted = 3
    assert_eq!(count, 3);
}

#[test]
fn ioc_cancel_final_status_is_webproto_cancelled() {
    // An IOC that can't cross must cancel. The OrderDoneRecord's
    // final_status is a webproto U-frame status (2 = CANCELLED per
    // specs/2/49-webproto.md), NOT the raw matching reason
    // (REASON_CANCELLED = 1, which the gateway forwards as RESTING).
    // Regression for the "IOC surfaces to the client as resting" bug.
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();
    let mut book = test_book();

    // Empty book -> IOC buy can't match -> OrderDone CANCELLED.
    let mut order = IncomingOrder {
        price: 50_000,
        qty: 10,
        remaining_qty: 10,
        side: Side::Buy,
        tif: TimeInForce::IOC,
        user_id: 1,
        reduce_only: false,
        post_only: false,
        order_id_hi: 0,
        order_id_lo: 42,
        timestamp_ns: 1000,
    };
    process_new_order(&mut book, &mut order);
    write_events_to_wal(&mut writer, &book, 1, 1000).unwrap();
    writer.flush().unwrap();

    let mut reader =
        WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    let mut done: Option<OrderDoneRecord> = None;
    while let Ok(Some(rec)) = reader.next() {
        if rec.header.record_type == RECORD_ORDER_DONE {
            done = decode_payload::<OrderDoneRecord>(&rec.payload);
        }
    }
    let done = done
        .expect("expected an OrderDone record for the cancelled IOC");
    assert_eq!(
        done.final_status, 2,
        "cancelled IOC must report webproto CANCELLED(2), not RESTING(1)",
    );
}

#[test]
fn flush_timer_fires_at_10ms() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    // last_flush = now, so flush_if_due should NOT flush
    let mut last_flush = Instant::now();
    let mut dummy_record = FillRecord {
        seq: 1,
        ts_ns: 1000,
        symbol_id: 1,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 0,
        maker_order_id_hi: 0,
        maker_order_id_lo: 0,
        price: Price(50000),
        qty: Qty(100),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
taker_ts_ns: 0,
    };
    {
        let framed = writer.prepare(&mut dummy_record).unwrap();
        writer.append_framed(&framed).unwrap();
    }
    flush_if_due(&mut writer, &mut last_flush).unwrap();

    let active = tmp
        .path()
        .join("1")
        .join("1_active.wal");
    let size1 =
        std::fs::metadata(&active).unwrap().len();
    // Should not have flushed (0ms elapsed)
    assert_eq!(size1, 0);

    // Simulate 10ms passing
    std::thread::sleep(
        Duration::from_millis(15),
    );
    flush_if_due(&mut writer, &mut last_flush).unwrap();

    let size2 =
        std::fs::metadata(&active).unwrap().len();
    assert!(size2 > 0);
}
