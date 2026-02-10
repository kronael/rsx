use rsx_book::book::Orderbook;
use rsx_book::matching::IncomingOrder;
use rsx_book::matching::process_new_order;
use rsx_dxs::FillRecord;
use rsx_dxs::wal::WalReader;
use rsx_dxs::wal::WalWriter;
use rsx_matching::wal_integration::flush_if_due;
use rsx_matching::wal_integration::write_events_to_wal;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
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
        1,
        tmp.path(),
        None,
        64 * 1024 * 1024,
        600_000_000_000,
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
fn flush_timer_fires_at_10ms() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        None,
        64 * 1024 * 1024,
        600_000_000_000,
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
        price: 50000,
        qty: 100,
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    writer.append(&mut dummy_record).unwrap();
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
        std::time::Duration::from_millis(15),
    );
    flush_if_due(&mut writer, &mut last_flush).unwrap();

    let size2 =
        std::fs::metadata(&active).unwrap().len();
    assert!(size2 > 0);
}
