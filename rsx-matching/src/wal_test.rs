use crate::wal::emit_events;
use crate::wal::EventSink;
use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::OrderDoneReason;
use rsx_messages::FillRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::SymbolConfig;
use rsx_types::NONE;
use std::io;

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
