// Future: intra-process tile decomposition (TILES.md).
// Currently unused -- between processes CMP replaces
// this routing. Kept for tile-based single-process mode.
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
        // busy-spin until push succeeds
        if let Ok(()) = prod.push(msg) {
            return;
        }
    }
}

/// Drain book event buffer and fan out to downstream
/// SPSC rings per routing table (CONSISTENCY.md section 1):
///   Fill       -> risk + gateway + mktdata
///   BBO        -> risk
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
            EventMessage::BBO { .. } => {
                push_spin(risk_prod, msg);
            }
        }
    }
}
