use rtrb::Producer;

use crate::wire::EventMessage;
use rsx_book::book::Orderbook;

/// Push with bare busy-spin until slot available.
/// Intended for dedicated-core threads only.
#[inline]
fn push_spin(
    prod: &mut Producer<EventMessage>,
    msg: EventMessage,
) {
    loop {
        match prod.push(msg) {
            Ok(()) => return,
            Err(_) => {} // busy-spin
        }
    }
}

/// Drain book event buffer and fan out to downstream
/// SPSC rings per routing table:
///   Fill       -> risk + gateway + mktdata
///   OrderDone  -> risk + gateway
///   OrderInserted -> mktdata
///   OrderCancelled -> gateway + mktdata
///   OrderFailed -> gateway
pub fn drain_and_fanout(
    book: &Orderbook,
    risk_prod: &mut Producer<EventMessage>,
    gw_prod: &mut Producer<EventMessage>,
    mkt_prod: &mut Producer<EventMessage>,
) {
    for event in book.events() {
        let msg = EventMessage::from_book_event(event);
        match msg {
            EventMessage::Fill { .. } => {
                push_spin(risk_prod, msg);
                push_spin(gw_prod, msg);
                push_spin(mkt_prod, msg);
            }
            EventMessage::OrderDone { .. } => {
                push_spin(risk_prod, msg);
                push_spin(gw_prod, msg);
            }
            EventMessage::OrderInserted { .. } => {
                push_spin(mkt_prod, msg);
            }
            EventMessage::OrderCancelled { .. } => {
                push_spin(gw_prod, msg);
                push_spin(mkt_prod, msg);
            }
            EventMessage::OrderFailed { .. } => {
                push_spin(gw_prod, msg);
            }
        }
    }
}
