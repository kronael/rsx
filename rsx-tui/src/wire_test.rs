//! Golden-bytes tests for the hand-derived prost schema.
//!
//! The schema has no `.proto` and no `prost-build`, so nothing but these
//! tests catches an accidental field-number/tag change. Each golden pins
//! the exact encoded bytes of one sample message; a changed tag or field
//! layout flips a byte and fails here. Plus a strict-enum check that a
//! garbled `side`/`tif` errors the decode rather than coercing to a
//! default (see `side_from_i32` / `tif_from_i32`).

use crate::conn::GwEvent;
use crate::conn::OrderReq;
use crate::conn::Side;
use crate::wire::to_wire_event;
use crate::wire::WireHello;
use crate::wire::WireOrder;
use prost::Message;

/// Encode one event to its wire bytes (client would decode these).
fn ev_bytes(ev: GwEvent) -> Vec<u8> {
    to_wire_event(&ev)
        .expect("event maps to wire")
        .encode_to_vec()
}

#[test]
fn hello_golden_bytes() {
    let bytes = WireHello {
        jwt: "tok".to_owned(),
        user: 7,
    }
    .encode_to_vec();
    assert_eq!(bytes, [10, 3, 116, 111, 107, 16, 7]);
}

#[test]
fn order_golden_bytes() {
    let bytes = WireOrder {
        cid: 3,
        symbol: 42,
        side: 1,
        price: 10_001,
        qty: 5,
        tif: 1,
    }
    .encode_to_vec();
    assert_eq!(bytes, [8, 3, 16, 42, 24, 1, 32, 145, 78, 40, 5, 48, 1]);
}

#[test]
fn event_golden_bytes() {
    assert_eq!(
        ev_bytes(GwEvent::Book {
            bids: vec![(100, 1), (99, 2)],
            asks: vec![(101, 3)],
        }),
        [10, 18, 10, 4, 8, 100, 16, 1, 10, 4, 8, 99, 16, 2, 18, 4, 8, 101, 16, 3],
    );
    assert_eq!(
        ev_bytes(GwEvent::Trade {
            side: Side::Buy,
            px: 100,
            qty: 4,
        }),
        [18, 4, 16, 100, 24, 4],
    );
    assert_eq!(ev_bytes(GwEvent::Accepted { oid: 9 }), [26, 2, 8, 9]);
    assert_eq!(
        ev_bytes(GwEvent::Fill {
            oid: 9,
            px: 100,
            qty: 4,
            side: Side::Sell,
        }),
        [34, 8, 8, 9, 16, 100, 24, 4, 32, 1],
    );
    assert_eq!(ev_bytes(GwEvent::Done { oid: 9 }), [42, 2, 8, 9]);
    assert_eq!(
        ev_bytes(GwEvent::Rejected {
            reason: "no".to_owned(),
        }),
        [50, 4, 10, 2, 110, 111],
    );
    assert_eq!(
        ev_bytes(GwEvent::Position {
            symbol: "X".to_owned(),
            net_qty: 5,
            entry_px: 100,
            upnl: -3,
        }),
        [58, 18, 10, 1, 88, 16, 5, 24, 100, 32, 253, 255, 255, 255, 255, 255, 255, 255, 255, 1],
    );
    assert_eq!(
        ev_bytes(GwEvent::Latency {
            cid: 3,
            net_ns: None,
            internal_ns: 500,
            engine_ns: 100,
        }),
        [66, 7, 8, 3, 24, 244, 3, 32, 100],
    );
}

#[test]
fn unknown_side_errors_the_decode() {
    let bytes = WireOrder {
        cid: 1,
        symbol: 0,
        side: 7,
        price: 1,
        qty: 1,
        tif: 0,
    }
    .encode_to_vec();
    let decoded = WireOrder::decode(&bytes[..]).expect("decode WireOrder");
    assert!(
        OrderReq::try_from(decoded).is_err(),
        "unknown side must error, not coerce to Buy",
    );
}

#[test]
fn unknown_tif_errors_the_decode() {
    let bytes = WireOrder {
        cid: 1,
        symbol: 0,
        side: 0,
        price: 1,
        qty: 1,
        tif: 9,
    }
    .encode_to_vec();
    let decoded = WireOrder::decode(&bytes[..]).expect("decode WireOrder");
    assert!(
        OrderReq::try_from(decoded).is_err(),
        "unknown tif must error, not coerce to Gtc",
    );
}
