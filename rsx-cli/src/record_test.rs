use crate::record::decode_fields;
use crate::record::name_to_record_type;
use crate::record::record_name;
use crate::record::to_json;
use crate::record::to_text;
use crate::record::DisplayScale;
use rsx_cast::header::WalHeader;
use rsx_cast::wal::RawWalRecord;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_messages::RECORD_ORDER_RESPONSE;
use serde_json::json;

const NAMES: [&str; 14] = [
    "FILL",
    "BBO",
    "ORDER_INSERTED",
    "ORDER_CANCELLED",
    "ORDER_DONE",
    "CONFIG_APPLIED",
    "CAUGHT_UP",
    "ORDER_ACCEPTED",
    "MARK_PRICE",
    "ORDER_REQUEST",
    "ORDER_RESPONSE",
    "CANCEL_REQUEST",
    "ORDER_FAILED",
    "LIQUIDATION",
];

fn raw_record(record_type: u16, payload: Vec<u8>) -> RawWalRecord {
    RawWalRecord {
        header: WalHeader::new(record_type, payload.len() as u16, 0xDEADBEEF),
        payload,
    }
}

fn order_request_payload() -> Vec<u8> {
    let mut payload = vec![0u8; 64];
    payload[0..8].copy_from_slice(&7u64.to_le_bytes());
    payload[8..12].copy_from_slice(&10u32.to_le_bytes());
    payload[12..16].copy_from_slice(&2u32.to_le_bytes());
    payload[16..24].copy_from_slice(&50000i64.to_le_bytes());
    payload[24..32].copy_from_slice(&100i64.to_le_bytes());
    payload[32..40].copy_from_slice(&0xAAu64.to_le_bytes());
    payload[40..48].copy_from_slice(&0xBBu64.to_le_bytes());
    payload[56] = 1;
    payload
}

#[test]
fn test_names_round_trip_through_record_types() {
    for name in NAMES {
        let record_type = name_to_record_type(name).unwrap();
        assert_eq!(record_name(record_type), name);
    }
}

#[test]
fn test_unknown_type_and_name() {
    assert_eq!(name_to_record_type("NOPE"), None);
    assert_eq!(record_name(0xFF), "UNKNOWN");
}

#[test]
fn test_short_payload_decodes_to_nothing() {
    let (text, fields) = decode_fields(RECORD_FILL, &[0u8; 16], &DisplayScale::new(None, None));
    assert_eq!(text, "");
    assert_eq!(fields, json!({}));
}

#[test]
fn test_unknown_type_decodes_to_nothing() {
    let (text, fields) = decode_fields(0xFF, &[0u8; 128], &DisplayScale::new(None, None));
    assert_eq!(text, "");
    assert_eq!(fields, json!({}));
}

#[test]
fn test_order_request_fields() {
    let payload = order_request_payload();
    let (text, fields) = decode_fields(
        RECORD_ORDER_REQUEST,
        &payload,
        &DisplayScale::new(None, None),
    );
    assert_eq!(
        fields,
        json!({
            "symbol_id": 2,
            "user_id": 10,
            "price": 50000,
            "qty": 100,
            "side": 1,
            "oid": "00000000000000aa00000000000000bb",
        })
    );
    assert!(text.contains("sym=2 user=10 px=50000 qty=100"), "{text}");
}

#[test]
fn test_order_response_needs_full_record() {
    let scale = DisplayScale::new(None, None);
    let (_, short) = decode_fields(RECORD_ORDER_RESPONSE, &[0u8; 64], &scale);
    assert_eq!(short, json!({}));
    let (_, full) = decode_fields(RECORD_ORDER_RESPONSE, &[0u8; 128], &scale);
    assert_eq!(full["status"], 0);
}

#[test]
fn test_to_json_header_keys_and_raw_fields() {
    let raw = raw_record(RECORD_ORDER_REQUEST, order_request_payload());
    let object = to_json(&raw);
    assert_eq!(object["seq"], 7);
    assert_eq!(object["type"], "ORDER_REQUEST");
    assert_eq!(object["len"], 64);
    assert_eq!(object["crc32"], "0xdeadbeef");
    assert_eq!(object["price"], 50000);
}

#[test]
fn test_to_json_unknown_type_has_header_only() {
    let mut payload = vec![0u8; 16];
    payload[..8].copy_from_slice(&9u64.to_le_bytes());
    payload[8..16].copy_from_slice(&1234u64.to_le_bytes());
    let object = to_json(&raw_record(0xFF, payload));
    assert_eq!(
        object,
        json!({
            "seq": 9,
            "type": "UNKNOWN",
            "ts_ns": 1234,
            "len": 16,
            "crc32": "0xdeadbeef"
        })
    );
}

#[test]
fn test_to_json_ts_ns_follows_each_layout() {
    let mut payload = order_request_payload();
    payload[48..56].copy_from_slice(&777u64.to_le_bytes());
    let request = to_json(&raw_record(RECORD_ORDER_REQUEST, payload));
    assert_eq!(request["ts_ns"], 777);

    let mut payload = vec![0u8; 128];
    payload[8..16].copy_from_slice(&555u64.to_le_bytes());
    let bbo = to_json(&raw_record(RECORD_BBO, payload));
    assert_eq!(bbo["ts_ns"], 555);

    let short = to_json(&raw_record(RECORD_BBO, vec![0u8; 12]));
    assert!(short["ts_ns"].is_null());
}

#[test]
fn test_cancel_reason_text_names_every_wire_code() {
    let names = [
        "user_cancel",
        "reduce_only",
        "expiry",
        "system",
        "post_only_reject",
        "other",
        "unknown_9",
    ];
    for (reason, name) in [0u8, 1, 2, 3, 4, 5, 9].into_iter().zip(names) {
        let mut payload = vec![0u8; 64];
        payload[48] = reason;
        let object = to_json(&raw_record(RECORD_ORDER_CANCELLED, payload));
        assert_eq!(object["reason"], reason, "{name}");
        assert_eq!(object["reason_text"], name);
    }
}

#[test]
fn test_text_scales_price_and_quantity() {
    let mut payload = vec![0u8; 128];
    payload[0..8].copy_from_slice(&3u64.to_le_bytes());
    payload[16..20].copy_from_slice(&4u32.to_le_bytes());
    payload[24..32].copy_from_slice(&5000i64.to_le_bytes());
    payload[32..40].copy_from_slice(&200i64.to_le_bytes());
    let raw = raw_record(RECORD_BBO, payload);
    let line = to_text(&raw, &DisplayScale::new(Some(10.0), Some(100.0)));
    assert!(line.starts_with("seq=3        type=BBO"), "{line}");
    assert!(line.contains("bid=500x2"), "{line}");
}
