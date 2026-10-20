use crate::wal::emit_events;
use crate::wal::publish_events;
use crate::wal::EventSink;
use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::OrderDoneReason;
use rsx_cast::cast::CastSender;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::WalReader;
use rsx_cast::wal::WalWriter;
use rsx_cast::WalHeader;
use rsx_messages::FillRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::SymbolConfig;
use rsx_types::NONE;
use std::io;
use std::net::UdpSocket;
use std::time::Duration;

#[derive(Default)]
struct CaptureSink {
    fill: Option<FillRecord>,
    done: Option<OrderDoneRecord>,
}

impl EventSink for CaptureSink {
    fn emit<T: rsx_cast::CastRecord>(&mut self, record: &mut T) -> io::Result<()> {
        let bytes = rsx_cast::as_bytes(record);
        match T::record_type() {
            RECORD_FILL => self.fill = rsx_cast::decode_payload(bytes),
            RECORD_ORDER_DONE => self.done = rsx_cast::decode_payload(bytes),
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
