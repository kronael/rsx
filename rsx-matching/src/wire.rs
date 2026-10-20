use rsx_book::matching::IncomingOrder;
use rsx_messages::OrderMessage;
use rsx_types::Side;
use rsx_types::TimeInForce;

/// Decode a wire `side` byte: 0 is Buy, any other value Sell. Total by design;
/// the Gateway rejects `side > 1` at parse (specs/2/47-validation-edge-cases.md
/// § 1.8), so the ME does not re-validate. Shared by the live path and replay.
pub fn decode_side(side: u8) -> Side {
    if side == 0 {
        Side::Buy
    } else {
        Side::Sell
    }
}

/// Decode a wire `tif` byte: 1 is IOC, 2 is FOK, any other value GTC. Total
/// by design, like `decode_side` (Gateway rejects `tif > 2`).
pub fn decode_tif(tif: u8) -> TimeInForce {
    match tif {
        1 => TimeInForce::IOC,
        2 => TimeInForce::FOK,
        _ => TimeInForce::GTC,
    }
}

/// Convert a wire `OrderMessage` (risk→ME, `RECORD_ORDER_REQUEST`) into the
/// book's `IncomingOrder`. Lives here rather than on `OrderMessage` because
/// the conversion depends on `rsx-book`, which the wire crate must not.
pub fn to_incoming(msg: &OrderMessage) -> IncomingOrder {
    IncomingOrder {
        price: msg.price,
        qty: msg.qty,
        remaining_qty: msg.qty,
        side: decode_side(msg.side),
        tif: decode_tif(msg.tif),
        user_id: msg.user_id,
        reduce_only: msg.reduce_only != 0,
        post_only: msg.post_only != 0,
        timestamp_ns: msg.timestamp_ns,
        order_id_hi: msg.order_id_hi,
        order_id_lo: msg.order_id_lo,
    }
}
