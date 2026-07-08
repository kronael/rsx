use rsx_book::matching::IncomingOrder;
use rsx_messages::OrderMessage;
use rsx_types::Side;
use rsx_types::TimeInForce;

/// Convert a wire `OrderMessage` (risk→ME, `RECORD_ORDER_REQUEST`) into the
/// book's `IncomingOrder`. Lives here rather than on `OrderMessage` because
/// the conversion depends on `rsx-book`, which the wire crate must not.
pub fn to_incoming(msg: &OrderMessage) -> IncomingOrder {
    IncomingOrder {
        price: msg.price,
        qty: msg.qty,
        remaining_qty: msg.qty,
        side: if msg.side == 0 { Side::Buy } else { Side::Sell },
        tif: match msg.tif {
            1 => TimeInForce::IOC,
            2 => TimeInForce::FOK,
            _ => TimeInForce::GTC,
        },
        user_id: msg.user_id,
        reduce_only: msg.reduce_only != 0,
        post_only: msg.post_only != 0,
        timestamp_ns: msg.timestamp_ns,
        order_id_hi: msg.order_id_hi,
        order_id_lo: msg.order_id_lo,
    }
}
