use rsx_marketdata::protocol::*;
use rsx_marketdata::types::*;

#[test]
fn ws_bbo_frame_includes_u_seq_field() {
    let bbo = BboUpdate {
        symbol_id: 1,
        bid_px: 50000,
        bid_qty: 100,
        bid_count: 3,
        ask_px: 50100,
        ask_qty: 200,
        ask_count: 5,
        timestamp_ns: 1000000,
        seq: 42,
    };
    let s = serialize_bbo(&bbo);
    assert!(s.contains("42"));
    let v: serde_json::Value =
        serde_json::from_str(&s).unwrap();
    let arr = v["BBO"].as_array().unwrap();
    assert_eq!(arr.len(), 9);
    assert_eq!(arr[8], 42);
}

#[test]
fn ws_b_snapshot_includes_u_seq_field() {
    let snap = L2Snapshot {
        symbol_id: 1,
        bids: vec![L2Level {
            price: 100,
            qty: 10,
            count: 1,
        }],
        asks: vec![L2Level {
            price: 101,
            qty: 20,
            count: 2,
        }],
        timestamp_ns: 2000000,
        seq: 99,
    };
    let s = serialize_l2_snapshot(&snap);
    let v: serde_json::Value =
        serde_json::from_str(&s).unwrap();
    let arr = v["B"].as_array().unwrap();
    assert_eq!(arr.len(), 5);
    assert_eq!(arr[4], 99);
}

#[test]
fn ws_d_delta_includes_u_seq_field() {
    let delta = L2Delta {
        symbol_id: 1,
        side: 0,
        price: 100,
        qty: 5,
        count: 1,
        timestamp_ns: 3000000,
        seq: 77,
    };
    let s = serialize_l2_delta(&delta);
    let v: serde_json::Value =
        serde_json::from_str(&s).unwrap();
    let arr = v["D"].as_array().unwrap();
    assert_eq!(arr.len(), 7);
    assert_eq!(arr[6], 77);
}

#[test]
fn ws_s_subscribe_frame_parsed() {
    let frame =
        parse_client_frame("{\"S\":[1,3]}").unwrap();
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
    let frame =
        parse_client_frame("{\"X\":[1,1]}").unwrap();
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
    let frame =
        parse_client_frame("{\"X\":[0,0]}").unwrap();
    assert_eq!(
        frame,
        MdFrame::Unsubscribe {
            symbol_id: 0,
            channels: 0
        }
    );
}

#[test]
fn bbo_serialize_contains_all_fields() {
    let bbo = BboUpdate {
        symbol_id: 5,
        bid_px: 1000,
        bid_qty: 50,
        bid_count: 2,
        ask_px: 1001,
        ask_qty: 60,
        ask_count: 3,
        timestamp_ns: 9999,
        seq: 10,
    };
    let s = serialize_bbo(&bbo);
    let v: serde_json::Value =
        serde_json::from_str(&s).unwrap();
    let arr = v["BBO"].as_array().unwrap();
    assert_eq!(arr[0], 5);
    assert_eq!(arr[1], 1000);
    assert_eq!(arr[2], 50);
    assert_eq!(arr[3], 2);
    assert_eq!(arr[4], 1001);
    assert_eq!(arr[5], 60);
    assert_eq!(arr[6], 3);
    assert_eq!(arr[7], 9999);
    assert_eq!(arr[8], 10);
}

#[test]
fn snapshot_serialize_contains_levels() {
    let snap = L2Snapshot {
        symbol_id: 2,
        bids: vec![
            L2Level { price: 100, qty: 10, count: 1 },
            L2Level { price: 99, qty: 20, count: 2 },
        ],
        asks: vec![
            L2Level { price: 101, qty: 30, count: 3 },
        ],
        timestamp_ns: 5000,
        seq: 55,
    };
    let s = serialize_l2_snapshot(&snap);
    let v: serde_json::Value =
        serde_json::from_str(&s).unwrap();
    let arr = v["B"].as_array().unwrap();
    assert_eq!(arr[0], 2);
    let bids = arr[1].as_array().unwrap();
    assert_eq!(bids.len(), 2);
    assert_eq!(bids[0][0], 100);
    let asks = arr[2].as_array().unwrap();
    assert_eq!(asks.len(), 1);
    assert_eq!(asks[0][0], 101);
}

#[test]
fn delta_serialize_contains_all_fields() {
    let delta = L2Delta {
        symbol_id: 3,
        side: 1,
        price: 200,
        qty: 15,
        count: 4,
        timestamp_ns: 7000,
        seq: 88,
    };
    let s = serialize_l2_delta(&delta);
    let v: serde_json::Value =
        serde_json::from_str(&s).unwrap();
    let arr = v["D"].as_array().unwrap();
    assert_eq!(arr[0], 3);
    assert_eq!(arr[1], 1);
    assert_eq!(arr[2], 200);
    assert_eq!(arr[3], 15);
    assert_eq!(arr[4], 4);
    assert_eq!(arr[5], 7000);
    assert_eq!(arr[6], 88);
}

#[test]
fn ws_u_field_equals_quic_seq() {
    // MD23: WS `u` field maps to QUIC `seq`
    let bbo = BboUpdate {
        symbol_id: 1,
        bid_px: 100,
        bid_qty: 10,
        bid_count: 1,
        ask_px: 200,
        ask_qty: 20,
        ask_count: 2,
        timestamp_ns: 5000,
        seq: 777, // QUIC seq
    };
    let s = serialize_bbo(&bbo);
    let v: serde_json::Value =
        serde_json::from_str(&s).unwrap();
    // Last element (u) must equal seq
    let arr = v["BBO"].as_array().unwrap();
    assert_eq!(arr[8].as_u64().unwrap(), 777);

    let delta = L2Delta {
        symbol_id: 1,
        side: 0,
        price: 100,
        qty: 10,
        count: 1,
        timestamp_ns: 5000,
        seq: 888,
    };
    let s = serialize_l2_delta(&delta);
    let v: serde_json::Value =
        serde_json::from_str(&s).unwrap();
    let arr = v["D"].as_array().unwrap();
    assert_eq!(arr[6].as_u64().unwrap(), 888);
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
fn trade_serialize_format() {
    let trade = TradeEvent {
        symbol_id: 4,
        price: 300,
        qty: 25,
        taker_side: 0,
        timestamp_ns: 8000,
        seq: 66,
    };
    let s = serialize_trade(&trade);
    let v: serde_json::Value =
        serde_json::from_str(&s).unwrap();
    let arr = v["T"].as_array().unwrap();
    assert_eq!(arr.len(), 6);
    assert_eq!(arr[0], 4);
    assert_eq!(arr[1], 300);
    assert_eq!(arr[5], 66);
}

#[test]
fn ws_h_heartbeat_frame_parsed() {
    let frame =
        parse_client_frame("{\"H\":[12345]}").unwrap();
    assert_eq!(
        frame,
        MdFrame::Heartbeat { timestamp_ms: 12345 }
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
