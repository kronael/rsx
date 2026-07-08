//! Golden-bytes tests for the hand-derived market-data wire schema.
//!
//! The schema has no `.proto` codegen (see `marketdata.proto` for the
//! documented shape), so nothing but these tests catches an accidental
//! field-number/tag change. Each golden pins the exact encoded bytes of
//! one frame — a 4-byte big-endian length prefix plus the `MdFrame`
//! prost body. A changed tag or field layout flips a byte and fails
//! here. The Python decoder (`rsx-playground/md_wire.py`) is validated
//! against these same byte vectors in `tests/api_md_wire_test.py`.

use crate::types::BboUpdate;
use crate::types::L2Delta;
use crate::types::L2Level;
use crate::types::L2Snapshot;
use crate::types::TradeEvent;
use crate::wire::encode_bbo;
use crate::wire::encode_heartbeat;
use crate::wire::encode_l2_delta;
use crate::wire::encode_l2_snapshot;
use crate::wire::encode_trade;

/// Every frame is `[len:u32 BE][body]`; the prefix must equal the body
/// length, and the whole frame is 4 bytes longer than the body.
fn assert_framed(frame: &[u8]) {
    assert!(frame.len() >= 4, "frame carries its 4-byte length prefix");
    let len = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    assert_eq!(len, frame.len() - 4, "length prefix matches body length");
}

#[test]
fn bbo_golden_bytes() {
    let bbo = BboUpdate {
        symbol_id: 1,
        bid_px: 100,
        bid_qty: 5,
        bid_count: 2,
        ask_px: 101,
        ask_qty: 7,
        ask_count: 3,
        timestamp_ns: 1000,
        seq: 42,
    };
    let frame = encode_bbo(&bbo);
    assert_framed(&frame);
    assert_eq!(
        frame,
        [
            0, 0, 0, 21, 10, 19, 8, 1, 16, 100, 24, 5, 32, 2, 40, 101, 48, 7, 56, 3, 64, 232, 7,
            72, 42
        ],
    );
}

#[test]
fn snapshot_golden_bytes() {
    let snap = L2Snapshot {
        symbol_id: 1,
        bids: vec![
            L2Level {
                price: 100,
                qty: 5,
                count: 2,
            },
            L2Level {
                price: 99,
                qty: 3,
                count: 1,
            },
        ],
        asks: vec![L2Level {
            price: 101,
            qty: 7,
            count: 3,
        }],
        timestamp_ns: 2000,
        seq: 99,
    };
    let frame = encode_l2_snapshot(&snap);
    assert_framed(&frame);
    assert_eq!(
        frame,
        [
            0, 0, 0, 33, 18, 31, 8, 1, 18, 6, 8, 100, 16, 5, 24, 2, 18, 6, 8, 99, 16, 3, 24, 1, 26,
            6, 8, 101, 16, 7, 24, 3, 32, 208, 15, 40, 99,
        ],
    );
}

#[test]
fn delta_golden_bytes() {
    let delta = L2Delta {
        symbol_id: 1,
        side: 1,
        price: 100,
        qty: 5,
        count: 1,
        timestamp_ns: 3000,
        seq: 77,
    };
    let frame = encode_l2_delta(&delta);
    assert_framed(&frame);
    assert_eq!(
        frame,
        [0, 0, 0, 17, 26, 15, 8, 1, 16, 1, 24, 100, 32, 5, 40, 1, 48, 184, 23, 56, 77],
    );
}

#[test]
fn trade_golden_bytes() {
    // taker_side = 0 (buy): proto3 omits the zero-valued scalar, so the
    // Trade body carries no field-4 tag. The Python decoder must read an
    // absent scalar as 0.
    let trade = TradeEvent {
        symbol_id: 4,
        price: 300,
        qty: 25,
        taker_side: 0,
        timestamp_ns: 8000,
        seq: 66,
    };
    let frame = encode_trade(&trade);
    assert_framed(&frame);
    assert_eq!(
        frame,
        [0, 0, 0, 14, 34, 12, 8, 4, 16, 172, 2, 24, 25, 40, 192, 62, 48, 66],
    );
}

#[test]
fn heartbeat_golden_bytes() {
    let frame = encode_heartbeat(12345);
    assert_framed(&frame);
    assert_eq!(frame, [0, 0, 0, 5, 42, 3, 8, 185, 96]);
}

#[test]
fn heartbeat_zero_omits_scalar() {
    // timestamp_ms = 0 is a proto3 default: the Heartbeat body is empty,
    // so the frame is just the oneof wrapper (tag 5, length 0).
    assert_eq!(encode_heartbeat(0), [0, 0, 0, 2, 42, 0]);
}
