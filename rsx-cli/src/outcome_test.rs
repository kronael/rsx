use crate::outcome::cancel_name;
use crate::outcome::failure_name;
use crate::outcome::CancelTracker;
use crate::outcome::PlaceTracker;
use crate::outcome::Status;
use crate::outcome::Verdict;
use rsx_messages::parse;
use rsx_types::FailureReason;
use serde_json::json;
use serde_json::Value;

const OID: &str = "0190000000007000800000000000000a";
const OTHER: &str = "0190000000007000800000000000000b";

fn feed(tracker: &mut PlaceTracker, text: &str) -> Option<Verdict> {
    let frame = parse(text).unwrap();
    let raw: Value = serde_json::from_str(text).unwrap();
    tracker.accept(&frame, raw)
}

fn feed_cancel(tracker: &mut CancelTracker, text: &str) -> Option<Verdict> {
    let frame = parse(text).unwrap();
    let raw: Value = serde_json::from_str(text).unwrap();
    tracker.accept(&frame, raw)
}

fn update(
    order_id: &str,
    status: u8,
    filled: i64,
    remaining: i64,
    reason: u8,
    stamp: u64,
) -> String {
    format!("{{\"U\":[\"{order_id}\",{status},{filled},{remaining},{reason},{stamp},0,0,0,0]}}")
}

fn fill(taker: &str, maker: &str, price: i64, qty: i64) -> String {
    format!("{{\"F\":[\"{taker}\",\"{maker}\",{price},{qty},5000,0,0,0,0,0,0]}}")
}

#[test]
fn test_failure_names_match_the_failure_reason_values() {
    let expected = [
        (FailureReason::InvalidTickSize, "INVALID_TICK_SIZE"),
        (FailureReason::InvalidLotSize, "INVALID_LOT_SIZE"),
        (FailureReason::SymbolNotFound, "SYMBOL_NOT_FOUND"),
        (FailureReason::DuplicateOrderId, "DUPLICATE_ORDER_ID"),
        (FailureReason::InsufficientMargin, "INSUFFICIENT_MARGIN"),
        (FailureReason::Overloaded, "OVERLOADED"),
        (FailureReason::InternalError, "INTERNAL_ERROR"),
        (FailureReason::ReduceOnlyViolation, "REDUCE_ONLY_VIOLATION"),
        (FailureReason::NetworkError, "NETWORK_ERROR"),
        (FailureReason::RateLimit, "RATE_LIMIT"),
        (FailureReason::Timeout, "TIMEOUT"),
        (FailureReason::UserInLiquidation, "USER_IN_LIQUIDATION"),
        (FailureReason::WrongShard, "WRONG_SHARD"),
        (FailureReason::FillOrKill, "FILL_OR_KILL"),
        (FailureReason::OrderTooSmall, "ORDER_TOO_SMALL"),
    ];
    for (reason, name) in expected {
        assert_eq!(failure_name(reason as u8), name);
    }
    assert_eq!(failure_name(15), "UNKNOWN_15");
}

#[test]
fn test_cancel_names_match_the_cancel_reason_values() {
    let names = [
        "USER_CANCEL",
        "REDUCE_ONLY",
        "EXPIRY",
        "SYSTEM",
        "POST_ONLY_REJECT",
        "OTHER",
    ];
    for (code, name) in names.iter().enumerate() {
        assert_eq!(cancel_name(code as u8), *name);
    }
    assert_eq!(cancel_name(6), "UNKNOWN_6");
}

#[test]
fn test_resting_update_ends_a_new_order_as_resting() {
    let mut tracker = PlaceTracker::new();
    let verdict = feed(&mut tracker, &update(OID, 1, 0, 5, 0, 0)).unwrap();
    assert_eq!(verdict.status, Status::Resting);
    assert_eq!(verdict.order_id.as_deref(), Some(OID));
    assert_eq!(verdict.reply, Some('U'));
    assert!(verdict.fills.is_empty());
    assert_eq!(verdict.frames.len(), 1);
}

#[test]
fn test_fills_then_done_is_filled_with_the_fills_listed() {
    let mut tracker = PlaceTracker::new();
    assert!(feed(&mut tracker, &fill(OID, OTHER, 100, 3)).is_none());
    assert!(feed(&mut tracker, &fill(OID, OTHER, 101, 2)).is_none());
    let verdict = feed(&mut tracker, &update(OID, 0, 5, 0, 0, 77)).unwrap();
    assert_eq!(verdict.status, Status::Filled);
    let fills: Vec<(i64, i64, u64)> = verdict
        .fills
        .iter()
        .map(|fill| (fill.price, fill.qty, fill.timestamp_ns))
        .collect();
    assert_eq!(fills, [(100, 3, 5000), (101, 2, 5000)]);
    assert_eq!(verdict.frames.len(), 3);
}

#[test]
fn test_fills_then_resting_is_partial() {
    let mut tracker = PlaceTracker::new();
    feed(&mut tracker, &fill(OID, OTHER, 100, 3));
    let verdict = feed(&mut tracker, &update(OID, 1, 0, 2, 0, 0)).unwrap();
    assert_eq!(verdict.status, Status::Partial);
    assert_eq!(verdict.fills.len(), 1);
}

#[test]
fn test_cancelled_after_fills_is_partial_and_names_the_reason() {
    let mut tracker = PlaceTracker::new();
    feed(&mut tracker, &fill(OID, OTHER, 100, 3));
    let verdict = feed(&mut tracker, &update(OID, 2, 3, 2, 2, 99)).unwrap();
    assert_eq!(verdict.status, Status::Partial);
    assert_eq!(verdict.reason_code, Some(2));
    assert_eq!(verdict.reason.as_deref(), Some("EXPIRY"));
}

#[test]
fn test_cancelled_without_fills_names_the_cancel_reason() {
    let mut tracker = PlaceTracker::new();
    let verdict = feed(&mut tracker, &update(OID, 2, 0, 5, 4, 123)).unwrap();
    assert_eq!(verdict.status, Status::Cancelled);
    assert_eq!(verdict.reason_code, Some(4));
    assert_eq!(verdict.reason.as_deref(), Some("POST_ONLY_REJECT"));
}

#[test]
fn test_failed_update_is_rejected_with_the_failure_name() {
    let mut tracker = PlaceTracker::new();
    let verdict = feed(&mut tracker, &update(OID, 3, 0, 0, 4, 0)).unwrap();
    assert_eq!(verdict.status, Status::Rejected);
    assert_eq!(verdict.reason_code, Some(4));
    assert_eq!(verdict.reason.as_deref(), Some("INSUFFICIENT_MARGIN"));
    assert_eq!(verdict.reply, Some('U'));
}

#[test]
fn test_duplicate_order_id_reason_is_a_rejection_carrying_the_first_order_id() {
    let mut tracker = PlaceTracker::new();
    let verdict = feed(&mut tracker, &update(OID, 3, 0, 0, 3, 0)).unwrap();
    assert_eq!(verdict.status, Status::Rejected);
    assert_eq!(verdict.reason.as_deref(), Some("DUPLICATE_ORDER_ID"));
    assert_eq!(verdict.order_id.as_deref(), Some(OID));
}

#[test]
fn test_gateway_timeout_reason_is_a_timeout() {
    let mut tracker = PlaceTracker::new();
    let verdict = feed(&mut tracker, &update(OID, 3, 0, 0, 10, 0)).unwrap();
    assert_eq!(verdict.status, Status::Timeout);
    assert_eq!(verdict.reason.as_deref(), Some("TIMEOUT"));
    assert!(verdict.status.is_failure());
}

#[test]
fn test_error_frame_is_a_rejection_with_its_code_and_message() {
    let mut tracker = PlaceTracker::new();
    let verdict = feed(&mut tracker, r#"{"E":[1009,"qty not lot aligned"]}"#).unwrap();
    assert_eq!(verdict.status, Status::Rejected);
    assert_eq!(verdict.reason_code, Some(1009));
    assert_eq!(verdict.reason.as_deref(), Some("qty not lot aligned"));
    assert_eq!(verdict.reply, Some('E'));
    assert_eq!(verdict.order_id, None);
}

#[test]
fn test_every_gateway_error_code_is_reported_with_its_code() {
    let codes = [
        (5, "overloaded"),
        (8, "network error"),
        (1001, "invalid utf8"),
        (1002, "invalid json"),
        (1004, "unsupported"),
        (1005, "order not found"),
        (1006, "rate limited"),
        (1007, "unknown symbol"),
        (1008, "price not tick aligned"),
        (1009, "qty not lot aligned"),
    ];
    for (code, message) in codes {
        let mut tracker = PlaceTracker::new();
        let text = format!("{{\"E\":[{code},\"{message}\"]}}");
        let verdict = feed(&mut tracker, &text).unwrap();
        assert_eq!(verdict.status, Status::Rejected);
        assert_eq!(verdict.reason_code, Some(code));
        assert_eq!(verdict.reason.as_deref(), Some(message));
    }
}

#[test]
fn test_frames_of_other_orders_are_skipped_after_the_order_is_named() {
    let mut tracker = PlaceTracker::new();
    assert!(feed(&mut tracker, &fill(OID, OTHER, 100, 3)).is_none());
    assert!(feed(&mut tracker, &update(OTHER, 0, 9, 0, 0, 1)).is_none());
    assert!(feed(&mut tracker, &fill(OTHER, OID, 100, 9)).is_none());
    let verdict = feed(&mut tracker, &update(OID, 1, 0, 2, 0, 0)).unwrap();
    assert_eq!(verdict.status, Status::Partial);
    assert_eq!(verdict.fills.len(), 1);
    assert_eq!(verdict.frames.len(), 2);
}

#[test]
fn test_liquidation_frames_do_not_end_an_order() {
    let mut tracker = PlaceTracker::new();
    assert!(feed(&mut tracker, r#"{"Q":[1,0,1,0,5,100,10]}"#).is_none());
}

#[test]
fn test_expire_is_a_local_timeout_with_no_reason() {
    let mut tracker = PlaceTracker::new();
    let verdict = tracker.expire();
    assert_eq!(verdict.status, Status::Timeout);
    assert_eq!(verdict.reply, None);
    assert_eq!(verdict.reason_code, None);
}

#[test]
fn test_place_json_has_the_contract_keys() {
    let mut tracker = PlaceTracker::new();
    feed(&mut tracker, &fill(OID, OTHER, 100, 3));
    let verdict = feed(&mut tracker, &update(OID, 0, 3, 0, 0, 7)).unwrap();
    let value = verdict.place_json("abc", 812);
    assert_eq!(
        value,
        json!({
            "cid": "abc",
            "status": "filled",
            "reason_code": null,
            "reason": null,
            "reply": "U",
            "order_id": OID,
            "fills": [{"px": 100, "qty": 3, "ts_ns": 5000}],
            "frames": [
                {"F": [OID, OTHER, 100, 3, 5000, 0, 0, 0, 0, 0, 0]},
                {"U": [OID, 0, 3, 0, 0, 7, 0, 0, 0, 0]},
            ],
            "rtt_us": 812,
        })
    );
}

#[test]
fn test_cancel_by_order_id_ends_on_that_orders_cancelled_update() {
    let mut tracker = CancelTracker::new(Some(OID.to_string()));
    assert!(feed_cancel(&mut tracker, &update(OTHER, 2, 0, 5, 0, 0)).is_none());
    let verdict = feed_cancel(&mut tracker, &update(OID, 2, 0, 5, 0, 0)).unwrap();
    assert_eq!(verdict.status, Status::Cancelled);
    assert_eq!(verdict.reason.as_deref(), Some("USER_CANCEL"));
    assert_eq!(verdict.order_id.as_deref(), Some(OID));
    assert_eq!(verdict.reply, Some('U'));
}

#[test]
fn test_cancel_by_cid_takes_the_first_cancelled_update() {
    let mut tracker = CancelTracker::new(None);
    assert!(feed_cancel(&mut tracker, &update(OID, 1, 0, 5, 0, 0)).is_none());
    let verdict = feed_cancel(&mut tracker, &update(OID, 2, 0, 5, 0, 0)).unwrap();
    assert_eq!(verdict.status, Status::Cancelled);
    assert_eq!(verdict.order_id.as_deref(), Some(OID));
}

#[test]
fn test_cancel_of_an_unknown_order_is_rejected_with_the_error_code() {
    let mut tracker = CancelTracker::new(None);
    let verdict = feed_cancel(&mut tracker, r#"{"E":[1005,"order not found"]}"#).unwrap();
    assert_eq!(verdict.status, Status::Rejected);
    assert_eq!(verdict.reason_code, Some(1005));
    assert_eq!(verdict.reason.as_deref(), Some("order not found"));
}

#[test]
fn test_cancel_expire_keeps_the_known_order_id() {
    let mut tracker = CancelTracker::new(Some(OID.to_string()));
    let verdict = tracker.expire();
    assert_eq!(verdict.status, Status::Timeout);
    assert_eq!(verdict.order_id.as_deref(), Some(OID));
}

#[test]
fn test_cancel_json_has_no_fills_and_a_null_cid_for_an_order_id() {
    let mut tracker = CancelTracker::new(Some(OID.to_string()));
    let verdict = feed_cancel(&mut tracker, &update(OID, 2, 0, 5, 0, 0)).unwrap();
    let value = verdict.cancel_json(None, 40);
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "cid",
            "frames",
            "order_id",
            "reason",
            "reason_code",
            "reply",
            "rtt_us",
            "status"
        ]
    );
    assert_eq!(value["cid"], Value::Null);
}

#[test]
fn test_text_line_names_status_key_fills_and_reason() {
    let mut tracker = PlaceTracker::new();
    feed(&mut tracker, &fill(OID, OTHER, 100, 3));
    let verdict = feed(&mut tracker, &update(OID, 2, 3, 2, 4, 0)).unwrap();
    assert_eq!(
        verdict.to_text("abc", 9),
        format!(
            "partial abc order_id={OID} fills=1 filled_qty=3 reason=POST_ONLY_REJECT(4) rtt_us=9"
        )
    );
    let mut tracker = PlaceTracker::new();
    let verdict = feed(&mut tracker, r#"{"E":[1009,"qty not lot aligned"]}"#).unwrap();
    assert_eq!(
        verdict.to_text("abc", 9),
        "rejected abc error=1009 (qty not lot aligned) rtt_us=9"
    );
}
