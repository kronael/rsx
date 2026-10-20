use crate::wal::emit_events;
use crate::wal::fan_out;
use crate::wal::flush_if_due;
use crate::wal::publish_events;
use crate::wal::retry_until_ok;
use crate::wal::EventSink;
use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::OrderCancelReason;
use rsx_book::event::OrderDoneReason;
use rsx_cast::cast::CastSender;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::WalReader;
use rsx_cast::wal::WalWriter;
use rsx_cast::WalHeader;
use rsx_messages::FillRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::CANCEL_REASON_EXPIRY;
use rsx_messages::CANCEL_REASON_POST_ONLY_REJECT;
use rsx_messages::CANCEL_REASON_REDUCE_ONLY;
use rsx_messages::CANCEL_REASON_USER_CANCEL;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::SymbolConfig;
use rsx_types::NONE;
use std::io;
use std::net::UdpSocket;
use std::time::Duration;
use std::time::Instant;

#[derive(Default)]
struct CaptureSink {
    fill: Option<FillRecord>,
    done: Option<OrderDoneRecord>,
    cancelled: Option<OrderCancelledRecord>,
}

impl EventSink for CaptureSink {
    fn emit<T: rsx_cast::CastRecord>(&mut self, record: &mut T) -> io::Result<()> {
        let bytes = rsx_cast::as_bytes(record);
        match T::record_type() {
            RECORD_FILL => self.fill = rsx_cast::decode_payload(bytes),
            RECORD_ORDER_DONE => self.done = rsx_cast::decode_payload(bytes),
            RECORD_ORDER_CANCELLED => self.cancelled = rsx_cast::decode_payload(bytes),
            _ => {}
        }
        Ok(())
    }
}

#[test]
fn matching_records_retain_risk_ingress() {
    let config = SymbolConfig {
        symbol_id: 1,
        price_decimals: 0,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    };
    let mut book = Orderbook::new(config, 16, 100);
    book.emit(Event::Fill {
        maker_handle: NONE,
        maker_tif: 0,
        maker_reduce_only: false,
        maker_user_id: 1,
        taker_user_id: 2,
        price: Price(100),
        qty: Qty(1),
        side: 0,
        maker_order_id_hi: 0,
        maker_order_id_lo: 1,
        taker_order_id_hi: 0,
        taker_order_id_lo: 2,
        gw_in_ns: 10,
    });
    book.emit(Event::OrderDone {
        handle: NONE,
        tif: 1,
        reduce_only: false,
        user_id: 2,
        reason: OrderDoneReason::Filled,
        filled_qty: Qty(1),
        remaining_qty: Qty(0),
        order_id_hi: 0,
        order_id_lo: 2,
    });

    let mut sink = CaptureSink::default();
    emit_events(&mut sink, &book, 1, 40, 10, 20, 30, 40).unwrap();

    let fill = sink.fill.unwrap();
    let done = sink.done.unwrap();
    assert_eq!((fill.gw_in_ns, fill.risk_in_ns), (10, 20));
    assert_eq!((done.gw_in_ns, done.risk_in_ns), (10, 20));
    assert_eq!((done.me_in_ns, done.match_done_ns), (30, 40));
}

/// `OrderCancelledRecord.reason` carries the `CANCEL_REASON_*` wire code
/// (specs/2/18-messages.md), not the book enum's discriminant.
#[test]
fn cancelled_record_reason_is_the_wire_code() {
    let config = SymbolConfig {
        symbol_id: 1,
        price_decimals: 0,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    };
    let cases = [
        (OrderCancelReason::User, CANCEL_REASON_USER_CANCEL),
        (OrderCancelReason::ReduceOnly, CANCEL_REASON_REDUCE_ONLY),
        (OrderCancelReason::ImmediateOrCancel, CANCEL_REASON_EXPIRY),
        (OrderCancelReason::PostOnly, CANCEL_REASON_POST_ONLY_REJECT),
    ];
    for (reason, code) in cases {
        let mut book = Orderbook::new(config, 16, 100);
        book.emit(Event::OrderCancelled {
            handle: NONE,
            user_id: 2,
            remaining_qty: Qty(1),
            reason,
            order_id_hi: 0,
            order_id_lo: 2,
        });
        let mut sink = CaptureSink::default();
        emit_events(&mut sink, &book, 1, 40, 0, 0, 0, 0).unwrap();
        let record = sink.cancelled.expect("cancelled record emitted");
        assert_eq!(record.reason, code, "{reason:?}");
    }
}

const STREAM_ID: u32 = 1;

fn book_with_done_and_bbo() -> Orderbook {
    let config = SymbolConfig {
        symbol_id: STREAM_ID,
        price_decimals: 0,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    };
    let mut book = Orderbook::new(config, 16, 100);
    book.emit(Event::OrderDone {
        handle: NONE,
        tif: 0,
        reduce_only: false,
        user_id: 2,
        reason: OrderDoneReason::Cancelled,
        filled_qty: Qty(0),
        remaining_qty: Qty(1),
        order_id_hi: 0,
        order_id_lo: 2,
    });
    book.emit(Event::BBO {
        bid_px: Price(99),
        bid_qty: Qty(1),
        ask_px: Price(101),
        ask_qty: Qty(1),
    });
    book
}

fn receiver() -> (UdpSocket, CastSender) {
    let dir = tempfile::tempdir().unwrap();
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let sender = CastSender::new(socket.local_addr().unwrap(), STREAM_ID, dir.path()).unwrap();
    (socket, sender)
}

fn unreachable_sender() -> CastSender {
    let dir = tempfile::tempdir().unwrap();
    let dest = "[::1]:9".parse().unwrap();
    CastSender::new(dest, STREAM_ID, dir.path()).unwrap()
}

fn assert_fan_out_survives(failing_risk: bool) {
    let wal_dir = tempfile::tempdir().unwrap();
    let mut writer = WalWriter::new(STREAM_ID, wal_dir.path(), 64 * 1024 * 1024).unwrap();
    let (socket, working) = receiver();
    let (mut risk, mut mkt) = if failing_risk {
        (unreachable_sender(), working)
    } else {
        (working, unreachable_sender())
    };
    let book = book_with_done_and_bbo();

    publish_events(
        &mut writer,
        &mut risk,
        &mut mkt,
        &book,
        STREAM_ID,
        1,
        0,
        0,
        0,
        0,
    )
    .unwrap();
    writer.flush().unwrap();

    let mut reader = WalReader::open_from_seq(STREAM_ID, 1, wal_dir.path()).unwrap();
    let mut buf = [0u8; 256];
    for expected_seq in 1..=2u64 {
        let raw = reader.next().unwrap().unwrap();
        assert_eq!(extract_seq(&raw.payload), Some(expected_seq));
        let len = socket.recv(&mut buf).unwrap();
        assert_eq!(&buf[WalHeader::SIZE..len], &raw.payload[..]);
    }
    assert!(reader.next().unwrap().is_none());
}

#[test]
fn failed_risk_send_keeps_wal_and_reaches_marketdata() {
    assert_fan_out_survives(true);
}

#[test]
fn failed_marketdata_send_keeps_wal_and_reaches_risk() {
    assert_fan_out_survives(false);
}

fn fill_record() -> FillRecord {
    FillRecord {
        seq: 0,
        ts_ns: 1,
        symbol_id: STREAM_ID,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 1,
        maker_order_id_hi: 0,
        maker_order_id_lo: 2,
        price: Price(100),
        qty: Qty(1),
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
    }
}

fn active_len(wal_dir: &std::path::Path) -> u64 {
    let active = wal_dir.join("1").join("1_active.wal");
    std::fs::metadata(active).unwrap().len()
}

#[test]
fn full_wal_buffer_stalls_the_producer_until_flushed() {
    // A 1 KiB segment cap puts the buffer limit at its 256 KiB floor, so
    // 5000 records (~144 B each) overrun it more than once.
    const RECORDS: u64 = 5000;
    let wal_dir = tempfile::tempdir().unwrap();
    let mut writer = WalWriter::new(STREAM_ID, wal_dir.path(), 1024).unwrap();
    let (_risk_socket, mut risk) = receiver();
    let (_mkt_socket, mut mkt) = receiver();

    for _ in 0..RECORDS {
        fan_out(&mut writer, &mut risk, &mut mkt, &mut fill_record()).unwrap();
    }
    writer.flush().unwrap();

    let mut reader = WalReader::open_from_seq(STREAM_ID, 1, wal_dir.path()).unwrap();
    for expected_seq in 1..=RECORDS {
        let raw = reader.next().unwrap().unwrap();
        assert_eq!(extract_seq(&raw.payload), Some(expected_seq));
    }
    assert!(reader.next().unwrap().is_none());
}

#[test]
fn record_threshold_flushes_before_the_timer() {
    let wal_dir = tempfile::tempdir().unwrap();
    let mut writer = WalWriter::new(STREAM_ID, wal_dir.path(), 64 * 1024 * 1024).unwrap();
    let now = Instant::now();
    let mut last_flush = now;

    for _ in 0..999 {
        let framed = writer.prepare(&mut fill_record()).unwrap();
        writer.append_framed(&framed).unwrap();
    }
    flush_if_due(&mut writer, &mut last_flush, now);
    assert_eq!(active_len(wal_dir.path()), 0);

    let framed = writer.prepare(&mut fill_record()).unwrap();
    writer.append_framed(&framed).unwrap();
    flush_if_due(&mut writer, &mut last_flush, now);
    assert!(active_len(wal_dir.path()) > 0);
}

#[test]
fn failing_flush_is_retried_until_it_succeeds() {
    let mut calls = 0;
    retry_until_ok("test flush", || {
        calls += 1;
        if calls < 4 {
            return Err(io::Error::other("disk gone"));
        }
        Ok(())
    });
    assert_eq!(calls, 4);
}
