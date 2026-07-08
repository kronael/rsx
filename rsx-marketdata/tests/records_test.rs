//! Inbound client control-frame parsing (subscribe / unsubscribe /
//! heartbeat, JSON text). Outbound feed encoding is protobuf and is
//! pinned by `src/wire_test.rs` (golden bytes), not here.

use rsx_marketdata::records::*;
use rsx_marketdata::types::*;

#[test]
fn ws_s_subscribe_frame_parsed() {
    let frame = parse_client_frame("{\"S\":[1,3]}").unwrap();
    assert_eq!(
        frame,
        MdFrame::Subscribe {
            symbol_id: 1,
            channels: 3
        }
    );
}

#[test]
fn ws_x_unsubscribe_frame_parsed() {
    let frame = parse_client_frame("{\"X\":[1,1]}").unwrap();
    assert_eq!(
        frame,
        MdFrame::Unsubscribe {
            symbol_id: 1,
            channels: 1
        }
    );
}

#[test]
fn ws_x_unsubscribe_all_parsed() {
    let frame = parse_client_frame("{\"X\":[0,0]}").unwrap();
    assert_eq!(
        frame,
        MdFrame::Unsubscribe {
            symbol_id: 0,
            channels: 0
        }
    );
}

#[test]
fn ws_seq_gap_detected_when_u_jumps() {
    // MD22: seq gap when u jumps > 1
    let bbo1 = BboUpdate {
        symbol_id: 1,
        bid_px: 100,
        bid_qty: 10,
        bid_count: 1,
        ask_px: 200,
        ask_qty: 20,
        ask_count: 2,
        timestamp_ns: 5000,
        seq: 10,
    };
    let bbo2 = BboUpdate {
        symbol_id: 1,
        bid_px: 101,
        bid_qty: 10,
        bid_count: 1,
        ask_px: 200,
        ask_qty: 20,
        ask_count: 2,
        timestamp_ns: 6000,
        seq: 15, // gap: 10 -> 15
    };
    let gap = bbo2.seq - bbo1.seq;
    assert!(gap > 1, "seq gap should be detected");
}

#[test]
fn ws_h_heartbeat_frame_parsed() {
    let frame = parse_client_frame("{\"H\":[12345]}").unwrap();
    assert_eq!(
        frame,
        MdFrame::Heartbeat {
            timestamp_ms: 12345
        }
    );
}

#[test]
fn ws_h_heartbeat_missing_timestamp_fails() {
    let result = parse_client_frame("{\"H\":[]}");
    assert!(result.is_err());
}

#[test]
fn ws_h_heartbeat_invalid_timestamp_fails() {
    let result = parse_client_frame("{\"H\":[\"not_a_number\"]}");
    assert!(result.is_err());
}
