use crate::md::frame_json;
use crate::md::frame_text;
use crate::md::snapshot_json;
use crate::md::snapshot_text;
use rsx_marketdata::types::BboUpdate;
use rsx_marketdata::types::L2Delta;
use rsx_marketdata::types::L2Level;
use rsx_marketdata::types::L2Snapshot;
use rsx_marketdata::types::TradeEvent;
use rsx_marketdata::wire::Frame;
use serde_json::json;

fn level(price: i64, qty: i64, count: u32) -> L2Level {
    L2Level { price, qty, count }
}

fn snapshot(bids: Vec<L2Level>, asks: Vec<L2Level>) -> L2Snapshot {
    L2Snapshot {
        symbol_id: 3,
        bids,
        asks,
        timestamp_ns: 2_000,
        seq: 99,
    }
}

#[test]
fn test_snapshot_json_of_an_empty_book_has_a_null_bbo() {
    let value = snapshot_json(&snapshot(Vec::new(), Vec::new()));
    assert_eq!(
        value,
        json!({"symbol_id": 3, "bbo": null, "bids": [], "asks": []})
    );
}

#[test]
fn test_snapshot_json_takes_the_bbo_from_the_best_levels() {
    let value = snapshot_json(&snapshot(
        vec![level(100, 5, 2), level(99, 3, 1)],
        vec![level(101, 7, 3), level(102, 1, 1)],
    ));
    assert_eq!(
        value,
        json!({
            "symbol_id": 3,
            "bbo": {
                "bid_px": 100, "bid_qty": 5, "ask_px": 101, "ask_qty": 7,
                "seq": 99, "ts_ns": 2000,
            },
            "bids": [[100, 5], [99, 3]],
            "asks": [[101, 7], [102, 1]],
        })
    );
}

#[test]
fn test_snapshot_json_of_a_one_sided_book_zeroes_the_empty_side() {
    let value = snapshot_json(&snapshot(vec![level(100, 5, 2)], Vec::new()));
    assert_eq!(value["bbo"]["bid_px"], 100);
    assert_eq!(value["bbo"]["ask_px"], 0);
    assert_eq!(value["bbo"]["ask_qty"], 0);
    assert_eq!(value["asks"], json!([]));
}

#[test]
fn test_frame_json_names_every_kind() {
    let bbo = frame_json(&Frame::Bbo(BboUpdate {
        symbol_id: 1,
        bid_px: 100,
        bid_qty: 5,
        bid_count: 2,
        ask_px: 101,
        ask_qty: 7,
        ask_count: 3,
        timestamp_ns: 1000,
        seq: 42,
    }));
    assert_eq!(
        bbo,
        json!({
            "kind": "bbo", "symbol_id": 1, "bid_px": 100, "bid_qty": 5, "bid_count": 2,
            "ask_px": 101, "ask_qty": 7, "ask_count": 3, "ts_ns": 1000, "seq": 42,
        })
    );

    let book = frame_json(&Frame::Snapshot(snapshot(
        vec![level(100, 5, 2)],
        vec![level(101, 7, 3)],
    )));
    assert_eq!(
        book,
        json!({
            "kind": "l2_snapshot", "symbol_id": 3, "bids": [[100, 5, 2]],
            "asks": [[101, 7, 3]], "ts_ns": 2000, "seq": 99,
        })
    );

    let delta = frame_json(&Frame::Delta(L2Delta {
        symbol_id: 1,
        side: 1,
        price: 100,
        qty: 0,
        count: 0,
        timestamp_ns: 3000,
        seq: 77,
    }));
    assert_eq!(
        delta,
        json!({
            "kind": "l2_delta", "symbol_id": 1, "side": 1, "px": 100, "qty": 0,
            "count": 0, "ts_ns": 3000, "seq": 77,
        })
    );

    let trade = frame_json(&Frame::Trade(TradeEvent {
        symbol_id: 4,
        price: 300,
        qty: 25,
        taker_side: 0,
        timestamp_ns: 8000,
        seq: 66,
    }));
    assert_eq!(
        trade,
        json!({
            "kind": "trade", "symbol_id": 4, "px": 300, "qty": 25, "taker_side": 0,
            "ts_ns": 8000, "seq": 66,
        })
    );

    let heartbeat = frame_json(&Frame::Heartbeat {
        timestamp_ms: 12345,
    });
    assert_eq!(heartbeat, json!({"kind": "heartbeat", "ts_ms": 12345}));
}

#[test]
fn test_frame_json_keeps_i64_and_u64_extremes_exact() {
    let frame = Frame::Delta(L2Delta {
        symbol_id: u32::MAX,
        side: 0,
        price: i64::MIN,
        qty: i64::MAX,
        count: u32::MAX,
        timestamp_ns: u64::MAX,
        seq: u64::MAX,
    });
    let line = frame_json(&frame).to_string();
    assert!(line.contains("-9223372036854775808"), "{line}");
    assert!(line.contains("9223372036854775807"), "{line}");
    assert!(line.contains("18446744073709551615"), "{line}");
    let back: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(back["px"].as_i64(), Some(i64::MIN));
    assert_eq!(back["qty"].as_i64(), Some(i64::MAX));
    assert_eq!(back["seq"].as_u64(), Some(u64::MAX));
}

#[test]
fn test_frame_text_is_one_line_per_kind() {
    let delta = Frame::Delta(L2Delta {
        symbol_id: 1,
        side: 0,
        price: 100,
        qty: 5,
        count: 1,
        timestamp_ns: 3000,
        seq: 77,
    });
    assert_eq!(frame_text(&delta), "l2_delta sym=1 bid 100x5 seq=77");
    let trade = Frame::Trade(TradeEvent {
        symbol_id: 4,
        price: 300,
        qty: 25,
        taker_side: 1,
        timestamp_ns: 8000,
        seq: 66,
    });
    assert_eq!(
        frame_text(&trade),
        "trade sym=4 px=300 qty=25 taker=sell seq=66"
    );
    assert_eq!(
        frame_text(&Frame::Heartbeat { timestamp_ms: 9 }),
        "heartbeat ts_ms=9"
    );
}

#[test]
fn test_snapshot_text_prints_asks_above_bids() {
    let text = snapshot_text(&snapshot(
        vec![level(100, 5, 2), level(99, 3, 1)],
        vec![level(101, 7, 3), level(102, 1, 1)],
    ));
    assert_eq!(
        text,
        "symbol 3 seq 99\nask 102 x 1\nask 101 x 7\nbid 100 x 5\nbid 99 x 3\n"
    );
}
