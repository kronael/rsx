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
use crate::wire::decode;
use crate::wire::encode_bbo;
use crate::wire::encode_heartbeat;
use crate::wire::encode_l2_delta;
use crate::wire::encode_l2_snapshot;
use crate::wire::encode_trade;
use crate::wire::DecodeError;
use crate::wire::Frame;
use prost::encoding::encode_varint;

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

fn bbo_with(bid_px: i64, ask_px: i64, qty: i64, seq: u64) -> BboUpdate {
    BboUpdate {
        symbol_id: u32::MAX,
        bid_px,
        bid_qty: qty,
        bid_count: u32::MAX,
        ask_px,
        ask_qty: qty,
        ask_count: 0,
        timestamp_ns: u64::MAX,
        seq,
    }
}

fn levels(count: i64) -> Vec<L2Level> {
    (0..count)
        .map(|i| L2Level {
            price: 1_000 - i,
            qty: i + 1,
            count: i as u32 + 1,
        })
        .collect()
}

fn snapshot_with(bids: Vec<L2Level>, asks: Vec<L2Level>) -> L2Snapshot {
    L2Snapshot {
        symbol_id: 7,
        bids,
        asks,
        timestamp_ns: 5,
        seq: 6,
    }
}

/// `[len][tag, len, inner]` plus `outer` bytes after the message, for frames
/// the encoder never produces.
fn envelope(tag: u8, inner: &[u8], outer: &[u8]) -> Vec<u8> {
    let mut body = vec![tag];
    encode_varint(inner.len() as u64, &mut body);
    body.extend_from_slice(inner);
    body.extend_from_slice(outer);
    let mut frame = (body.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(&body);
    frame
}

#[test]
fn decode_inverts_every_golden_frame() {
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
    let golden = [
        0, 0, 0, 21, 10, 19, 8, 1, 16, 100, 24, 5, 32, 2, 40, 101, 48, 7, 56, 3, 64, 232, 7, 72, 42,
    ];
    assert_eq!(decode(&golden), Ok(Frame::Bbo(bbo)));

    let snap = snapshot_with(Vec::new(), Vec::new());
    assert_eq!(
        decode(&encode_l2_snapshot(&snap)),
        Ok(Frame::Snapshot(snap))
    );

    let golden = [0, 0, 0, 5, 42, 3, 8, 185, 96];
    assert_eq!(
        decode(&golden),
        Ok(Frame::Heartbeat {
            timestamp_ms: 12345
        })
    );
    let golden = [
        0, 0, 0, 14, 34, 12, 8, 4, 16, 172, 2, 24, 25, 40, 192, 62, 48, 66,
    ];
    let trade = TradeEvent {
        symbol_id: 4,
        price: 300,
        qty: 25,
        taker_side: 0,
        timestamp_ns: 8000,
        seq: 66,
    };
    assert_eq!(decode(&golden), Ok(Frame::Trade(trade)));
    let golden = [
        0, 0, 0, 17, 26, 15, 8, 1, 16, 1, 24, 100, 32, 5, 40, 1, 48, 184, 23, 56, 77,
    ];
    let delta = L2Delta {
        symbol_id: 1,
        side: 1,
        price: 100,
        qty: 5,
        count: 1,
        timestamp_ns: 3000,
        seq: 77,
    };
    assert_eq!(decode(&golden), Ok(Frame::Delta(delta)));
}

#[test]
fn decode_inverts_snapshot_golden_bytes() {
    let golden = [
        0, 0, 0, 33, 18, 31, 8, 1, 18, 6, 8, 100, 16, 5, 24, 2, 18, 6, 8, 99, 16, 3, 24, 1, 26, 6,
        8, 101, 16, 7, 24, 3, 32, 208, 15, 40, 99,
    ];
    let Ok(Frame::Snapshot(snap)) = decode(&golden) else {
        panic!("golden snapshot must decode");
    };
    assert_eq!(snap.symbol_id, 1);
    assert_eq!(snap.bids.len(), 2);
    assert_eq!(
        (snap.bids[0].price, snap.bids[0].qty, snap.bids[0].count),
        (100, 5, 2)
    );
    assert_eq!(
        (snap.bids[1].price, snap.bids[1].qty, snap.bids[1].count),
        (99, 3, 1)
    );
    assert_eq!(snap.asks.len(), 1);
    assert_eq!(
        (snap.asks[0].price, snap.asks[0].qty, snap.asks[0].count),
        (101, 7, 3)
    );
    assert_eq!((snap.timestamp_ns, snap.seq), (2000, 99));
}

#[test]
fn bbo_round_trips_edge_values() {
    for px in [i64::MIN, -1, 0, 1, i64::MAX] {
        for qty in [i64::MIN, 0, i64::MAX] {
            for seq in [0, 1, u64::MAX] {
                let bbo = bbo_with(px, px.wrapping_add(1), qty, seq);
                assert_eq!(decode(&encode_bbo(&bbo)), Ok(Frame::Bbo(bbo)));
            }
        }
    }
}

#[test]
fn bbo_with_every_field_zero_round_trips() {
    let bbo = BboUpdate {
        symbol_id: 0,
        bid_px: 0,
        bid_qty: 0,
        bid_count: 0,
        ask_px: 0,
        ask_qty: 0,
        ask_count: 0,
        timestamp_ns: 0,
        seq: 0,
    };
    let frame = encode_bbo(&bbo);
    assert_eq!(frame, [0, 0, 0, 2, 10, 0]);
    assert_eq!(decode(&frame), Ok(Frame::Bbo(bbo)));
}

#[test]
fn snapshot_round_trips_empty_and_one_sided_books() {
    for (bids, asks) in [
        (Vec::new(), Vec::new()),
        (levels(3), Vec::new()),
        (Vec::new(), levels(3)),
        (levels(200), levels(200)),
    ] {
        let snap = snapshot_with(bids, asks);
        assert_eq!(
            decode(&encode_l2_snapshot(&snap)),
            Ok(Frame::Snapshot(snap))
        );
    }
}

#[test]
fn snapshot_round_trips_extreme_levels_in_order() {
    let extremes = vec![
        L2Level {
            price: i64::MAX,
            qty: i64::MAX,
            count: u32::MAX,
        },
        L2Level {
            price: 0,
            qty: 0,
            count: 0,
        },
        L2Level {
            price: i64::MIN,
            qty: i64::MIN,
            count: 1,
        },
    ];
    let snap = L2Snapshot {
        symbol_id: u32::MAX,
        bids: extremes.clone(),
        asks: extremes,
        timestamp_ns: u64::MAX,
        seq: u64::MAX,
    };
    assert_eq!(
        decode(&encode_l2_snapshot(&snap)),
        Ok(Frame::Snapshot(snap))
    );
}

#[test]
fn delta_round_trips_both_sides_and_removal() {
    for side in [0u8, 1] {
        for (price, qty) in [(i64::MIN, 0), (i64::MAX, i64::MAX), (0, 0), (-5, i64::MIN)] {
            let delta = L2Delta {
                symbol_id: u32::MAX,
                side,
                price,
                qty,
                count: u32::MAX,
                timestamp_ns: u64::MAX,
                seq: u64::MAX,
            };
            assert_eq!(decode(&encode_l2_delta(&delta)), Ok(Frame::Delta(delta)));
        }
    }
}

#[test]
fn trade_round_trips_both_takers_and_extremes() {
    for taker_side in [0u8, 1] {
        for (price, qty) in [(i64::MIN, i64::MIN), (i64::MAX, i64::MAX), (0, 0)] {
            let trade = TradeEvent {
                symbol_id: u32::MAX,
                price,
                qty,
                taker_side,
                timestamp_ns: u64::MAX,
                seq: u64::MAX,
            };
            assert_eq!(decode(&encode_trade(&trade)), Ok(Frame::Trade(trade)));
        }
    }
}

#[test]
fn heartbeat_round_trips_zero_and_max() {
    for timestamp_ms in [0, 1, u64::MAX] {
        assert_eq!(
            decode(&encode_heartbeat(timestamp_ms)),
            Ok(Frame::Heartbeat { timestamp_ms })
        );
    }
}

#[test]
fn unknown_fields_are_skipped_at_every_level() {
    // Field 15 varint, field 14 length-delimited, field 13 fixed32 and
    // field 12 fixed64 inside the message and after it in the envelope.
    let mut unknown = vec![15 << 3, 7, (14 << 3) | 2, 3, 1, 2, 3];
    unknown.extend_from_slice(&[(13 << 3) | 5, 1, 2, 3, 4]);
    unknown.extend_from_slice(&[(12 << 3) | 1, 1, 2, 3, 4, 5, 6, 7, 8]);

    let mut inner = vec![8, 9, 16, 100];
    inner.extend_from_slice(&unknown);
    let frame = envelope((1 << 3) | 2, &inner, &unknown);
    let Ok(Frame::Bbo(bbo)) = decode(&frame) else {
        panic!("a bbo with unknown fields must decode");
    };
    assert_eq!((bbo.symbol_id, bbo.bid_px, bbo.ask_px), (9, 100, 0));

    let mut level = vec![8, 100, 16, 5];
    level.extend_from_slice(&unknown);
    let mut inner = vec![8, 3, (2 << 3) | 2, level.len() as u8];
    inner.extend_from_slice(&level);
    inner.extend_from_slice(&unknown);
    let frame = envelope((2 << 3) | 2, &inner, &[]);
    let Ok(Frame::Snapshot(snap)) = decode(&frame) else {
        panic!("a snapshot with unknown fields must decode");
    };
    assert_eq!(snap.bids.len(), 1);
    assert_eq!((snap.bids[0].price, snap.bids[0].qty), (100, 5));
}

#[test]
fn an_envelope_field_the_schema_lacks_sets_no_body() {
    let frame = envelope((6 << 3) | 2, &[8, 1], &[]);
    assert_eq!(decode(&frame), Err(DecodeError::EmptyFrame));
}

#[test]
fn a_repeated_oneof_keeps_the_last_message() {
    let mut frame = envelope((5 << 3) | 2, &[8, 1], &[]);
    let second = envelope((5 << 3) | 2, &[8, 2], &[]);
    frame.extend_from_slice(&second[4..]);
    let body_len = (frame.len() - 4) as u32;
    frame[..4].copy_from_slice(&body_len.to_be_bytes());
    assert_eq!(decode(&frame), Ok(Frame::Heartbeat { timestamp_ms: 2 }));
}

#[test]
fn a_repeated_scalar_keeps_the_last_value() {
    let frame = envelope((5 << 3) | 2, &[8, 1, 8, 9], &[]);
    assert_eq!(decode(&frame), Ok(Frame::Heartbeat { timestamp_ms: 9 }));
}

#[test]
fn an_empty_envelope_is_rejected() {
    assert_eq!(decode(&[0, 0, 0, 0]), Err(DecodeError::EmptyFrame));
}

#[test]
fn every_proper_prefix_of_a_frame_is_rejected() {
    let frame = encode_l2_snapshot(&snapshot_with(levels(2), levels(1)));
    for len in 0..4 {
        assert_eq!(decode(&frame[..len]), Err(DecodeError::Truncated { len }));
    }
    for len in 4..frame.len() {
        assert_eq!(
            decode(&frame[..len]),
            Err(DecodeError::LengthMismatch {
                prefix: frame.len() - 4,
                body: len - 4,
            })
        );
    }
}

#[test]
fn trailing_bytes_after_the_frame_are_rejected() {
    let mut frame = encode_heartbeat(5);
    frame.push(0);
    assert!(matches!(
        decode(&frame),
        Err(DecodeError::LengthMismatch { .. })
    ));
}

#[test]
fn a_malformed_body_is_a_protobuf_error() {
    // Tag claims 5 length-delimited bytes, none follow.
    assert!(matches!(
        decode(&[0, 0, 0, 2, 10, 5]),
        Err(DecodeError::Protobuf(_))
    ));
    // Wire type 7 does not exist.
    assert!(matches!(
        decode(&[0, 0, 0, 1, 7]),
        Err(DecodeError::Protobuf(_))
    ));
}

#[test]
fn a_side_wider_than_u8_is_rejected() {
    let delta = envelope((3 << 3) | 2, &[16, 0x80, 0x02], &[]);
    assert_eq!(decode(&delta), Err(DecodeError::SideOutOfRange(256)));
    let trade = envelope((4 << 3) | 2, &[32, 0x80, 0x02], &[]);
    assert_eq!(decode(&trade), Err(DecodeError::SideOutOfRange(256)));
}
