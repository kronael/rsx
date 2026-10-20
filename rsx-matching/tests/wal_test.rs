use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_cast::decode_payload;
use rsx_cast::wal::WalReader;
use rsx_cast::wal::WalWriter;
use rsx_matching::wal::flush_if_due;
use rsx_matching::wal::write_events_to_wal;
use rsx_messages::FillRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_types::Price;
use rsx_types::Qty;
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
    let mut writer = WalWriter::new(1, tmp.path(), 64 * 1024 * 1024).unwrap();

    let mut book = test_book();

    // Resting sell -> buy crosses -> fill + done events
    book.insert_resting(50_100, 100, Side::Sell, 0, 1, false, 0, 0, 0);
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

    write_events_to_wal(&mut writer, &book, 1, 1000).unwrap();
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
    write_events_to_wal(&mut writer, &book, 1, 2000).unwrap();
    writer.flush().unwrap();

    // Read back all records
    let mut reader = WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    let mut count = 0;
    while let Ok(Some(_)) = reader.next() {
        count += 1;
    }
    // First order (buy crosses resting sell, best ask px/tick unchanged):
    //   Fill + OrderDone(taker) = 2, no BBO (only qty at best changed).
    // Second order (buy rests below, best bid none -> 49_900):
    //   OrderInserted + BBO = 2 (bid appears -> best-bid tick changed).
    // BBO is WAL-persisted (same records as the production fan-out).
    assert_eq!(count, 4);
}

#[test]
fn ioc_cancel_final_status_is_webproto_cancelled() {
    // An unfilled IOC produces web-protocol CANCELLED, never RESTING.
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(1, tmp.path(), 64 * 1024 * 1024).unwrap();
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

    let mut reader = WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    let mut done: Option<OrderDoneRecord> = None;
    while let Ok(Some(rec)) = reader.next() {
        if rec.header.record_type == RECORD_ORDER_DONE {
            done = decode_payload::<OrderDoneRecord>(&rec.payload);
        }
    }
    let done = done.expect("expected an OrderDone record for the cancelled IOC");
    assert_eq!(
        done.final_status, 2,
        "cancelled IOC must report webproto CANCELLED(2), not RESTING(1)",
    );
}

#[test]
fn maker_wire_metadata_survives_freed_slot_reuse() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(1, tmp.path(), 64 * 1024 * 1024).unwrap();
    let mut book = test_book();

    let maker_handle = book.insert_resting(
        50_100,
        10,
        Side::Sell,
        TimeInForce::IOC as u8,
        1,
        true,
        0,
        11,
        12,
    );
    let mut taker = IncomingOrder {
        price: 50_100,
        qty: 20,
        remaining_qty: 20,
        side: Side::Buy,
        tif: TimeInForce::GTC,
        user_id: 2,
        reduce_only: false,
        post_only: false,
        order_id_hi: 21,
        order_id_lo: 22,
        timestamp_ns: 1000,
    };
    process_new_order(&mut book, &mut taker);

    assert!(book.orders.get(maker_handle).is_active());
    assert_eq!(book.orders.get(maker_handle).user_id, taker.user_id);

    write_events_to_wal(&mut writer, &book, 1, 1000).unwrap();
    writer.flush().unwrap();

    let mut reader = WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    let mut maker_fill = None;
    let mut maker_done = None;
    while let Some(record) = reader.next().unwrap() {
        match record.header.record_type {
            RECORD_FILL => {
                let fill = decode_payload::<FillRecord>(&record.payload).unwrap();
                if fill.maker_order_id_hi == 11 && fill.maker_order_id_lo == 12 {
                    maker_fill = Some(fill);
                }
            }
            RECORD_ORDER_DONE => {
                let done = decode_payload::<OrderDoneRecord>(&record.payload).unwrap();
                if done.order_id_hi == 11 && done.order_id_lo == 12 {
                    maker_done = Some(done);
                }
            }
            _ => {}
        }
    }

    let fill = maker_fill.expect("maker FillRecord");
    assert_eq!(fill.tif, TimeInForce::IOC as u8);
    assert_eq!(fill.reduce_only, 1);
    let done = maker_done.expect("maker OrderDoneRecord");
    assert_eq!(done.tif, TimeInForce::IOC as u8);
    assert_eq!(done.reduce_only, 1);
}

#[test]
fn flush_timer_fires_at_10ms() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(1, tmp.path(), 64 * 1024 * 1024).unwrap();

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
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    };
    {
        let framed = writer.prepare(&mut dummy_record).unwrap();
        writer.append_framed(&framed).unwrap();
    }
    flush_if_due(&mut writer, &mut last_flush, Instant::now());

    let active = tmp.path().join("1").join("1_active.wal");
    let size1 = std::fs::metadata(&active).unwrap().len();
    // Should not have flushed (0ms elapsed)
    assert_eq!(size1, 0);

    // Simulate 10ms passing
    std::thread::sleep(Duration::from_millis(15));
    flush_if_due(&mut writer, &mut last_flush, Instant::now());

    let size2 = std::fs::metadata(&active).unwrap().len();
    assert!(size2 > 0);
}
