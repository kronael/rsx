use crate::dup_order::check_outcome;
use crate::dup_order::order_id_hex;
use crate::dup_order::order_message;
use crate::dup_order::read_order_records;
use crate::dup_order::scratch_env;
use crate::dup_order::SCRATCH_SYMBOL_ID;
use crate::fixture_utils::Wal;
use crate::fixture_utils::SELL;
use rsx_book::event::OrderFailureReason;
use rsx_cast::as_bytes;
use rsx_matching::wal::failure_reason_code;
use serde_json::json;
use serde_json::Value;
use std::net::SocketAddr;
use std::path::Path;
use tempfile::tempdir;

const ORDER_LO: u64 = 0x1234;

fn duplicate_code() -> u8 {
    failure_reason_code(OrderFailureReason::Duplicate)
}

fn record(seq: u64, name: &str, reason: Option<u8>) -> Value {
    let mut record = json!({ "seq": seq, "type": name, "oid": order_id_hex(ORDER_LO) });
    if let Some(reason) = reason {
        record["reason"] = json!(reason);
    }
    record
}

fn outcome_of(records: &[Value]) -> Result<(), String> {
    check_outcome(records, &order_id_hex(ORDER_LO)).map_err(|failure| failure.message().to_string())
}

#[test]
fn the_order_is_the_wire_message_risk_casts() {
    let message = order_message(7, ORDER_LO, 99);
    assert_eq!(as_bytes(&message).len(), 128);
    assert_eq!(message.seq, 7);
    assert_eq!(message.order_id_lo, ORDER_LO);
    assert_eq!(message.order_id_hi, 0);
    assert_eq!((message.timestamp_ns, message.risk_in_ns), (99, 99));
    assert_eq!(message.side, SELL);
}

#[test]
fn the_order_id_is_32_hex_characters() {
    assert_eq!(order_id_hex(0xAB), format!("{}ab", "0".repeat(30)));
}

#[test]
fn scratch_env_carries_only_its_own_wiring() {
    let addr = |port: u16| -> SocketAddr { format!("127.0.0.1:{port}").parse().unwrap() };
    let env = scratch_env(
        Path::new("/w/wal"),
        addr(1),
        &[addr(2), addr(3), addr(4)],
        "127.0.0.1:5",
    );
    let get = |key: &str| env.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str());
    assert_eq!(get("RSX_ME_SYMBOL_ID"), Some("999"));
    assert_eq!(get("RSX_ME_WAL_DIR"), Some("/w/wal"));
    assert_eq!(get("RSX_ME_CAST_ADDR"), Some("127.0.0.1:1"));
    assert_eq!(get("RSX_RISK_ME_SEND_ADDR"), Some("127.0.0.1:2"));
    assert_eq!(get("RSX_RISK_ME_RECV_ADDR"), Some("127.0.0.1:3"));
    assert_eq!(get("RSX_MD_CAST_ADDR"), Some("127.0.0.1:4"));
    assert_eq!(get("RSX_ME_HEALTH_ADDR"), Some("127.0.0.1:5"));
    assert_eq!(get("RSX_ME_DATABASE_URL"), None);
    assert_eq!(get("RSX_ME_REPLICATION_BIND_ADDR"), None);
    assert_eq!(get("RSX_ME_CORE_ID"), None);
}

#[test]
fn one_accepted_then_one_duplicate_failure_passes() {
    let records = [
        record(1, "ORDER_ACCEPTED", None),
        record(2, "ORDER_INSERTED", None),
        record(3, "ORDER_FAILED", Some(duplicate_code())),
    ];
    assert_eq!(outcome_of(&records), Ok(()));
}

#[test]
fn every_other_shape_fails_and_names_what_the_wal_has() {
    let accepted = record(1, "ORDER_ACCEPTED", None);
    let duplicate = record(3, "ORDER_FAILED", Some(duplicate_code()));
    let other_reason = record(3, "ORDER_FAILED", Some(duplicate_code() + 1));
    let early_failure = record(0, "ORDER_FAILED", Some(duplicate_code()));
    let cases = [
        vec![],
        vec![accepted.clone()],
        vec![duplicate.clone()],
        vec![accepted.clone(), accepted.clone(), duplicate.clone()],
        vec![accepted.clone(), duplicate.clone(), duplicate.clone()],
        vec![accepted.clone(), other_reason],
        vec![accepted.clone(), early_failure],
    ];
    for records in cases {
        let message = outcome_of(&records).unwrap_err();
        assert!(message.contains(&order_id_hex(ORDER_LO)), "{message}");
    }
    let message = outcome_of(&[accepted]).unwrap_err();
    assert!(message.contains("seq=1 type=ORDER_ACCEPTED"), "{message}");
}

#[test]
fn only_the_records_of_the_order_are_read_from_the_scratch_stream() {
    let dir = tempdir().unwrap();
    let mut wal = Wal::default();
    wal.accepted(1, 0x99, SELL, 100, 1)
        .accepted(1, ORDER_LO, SELL, 100, 1)
        .inserted(1, ORDER_LO, SELL, 100, 1)
        .failed(1, ORDER_LO, duplicate_code())
        .failed(2, 0x77, duplicate_code());
    wal.write_stream(dir.path(), SCRATCH_SYMBOL_ID);

    let records = read_order_records(dir.path(), &order_id_hex(ORDER_LO)).unwrap();

    let seen: Vec<(u64, &str)> = records
        .iter()
        .map(|r| (r["seq"].as_u64().unwrap(), r["type"].as_str().unwrap()))
        .collect();
    assert_eq!(
        seen,
        [
            (2, "ORDER_ACCEPTED"),
            (3, "ORDER_INSERTED"),
            (4, "ORDER_FAILED")
        ]
    );
    assert_eq!(outcome_of(&records), Ok(()));
}

#[test]
fn a_scratch_directory_without_a_wal_reads_as_no_records() {
    let dir = tempdir().unwrap();
    let records = read_order_records(dir.path(), &order_id_hex(ORDER_LO)).unwrap();
    assert!(records.is_empty());
    assert!(outcome_of(&records).is_err());
}
