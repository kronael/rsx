use crate::output::Failure;
use crate::wal::extract_symbol_id;
use crate::wal::extract_ts_ns;
use crate::wal::extract_user_id;
use crate::wal::Filters;
use crate::wal::Tail;
use rsx_cast::encode_utils::encode_record;
use rsx_cast::wal::extract_seq;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_messages::RECORD_ORDER_REQUEST;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

fn filters(
    names: &[&str],
    symbol: Option<u32>,
    user: Option<u32>,
    from_ts: Option<u64>,
    to_ts: Option<u64>,
) -> Filters {
    let names: Vec<String> = names.iter().map(|name| name.to_string()).collect();
    Filters::new(&names, symbol, user, from_ts, to_ts).unwrap()
}

fn fill_payload(ts_ns: u64, symbol_id: u32, taker_user_id: u32) -> Vec<u8> {
    let mut payload = vec![0u8; 128];
    payload[8..16].copy_from_slice(&ts_ns.to_le_bytes());
    payload[16..20].copy_from_slice(&symbol_id.to_le_bytes());
    payload[20..24].copy_from_slice(&taker_user_id.to_le_bytes());
    payload
}

#[test]
fn test_unknown_record_type_is_misuse() {
    let names = vec!["FILL".to_string(), "NOPE".to_string()];
    let failure = Filters::new(&names, None, None, None, None).err().unwrap();
    assert!(matches!(failure, Failure::Misuse(_)));
    assert_eq!(failure.message(), "unknown record type: NOPE");
}

#[test]
fn test_no_criteria_matches_everything() {
    let all = filters(&[], None, None, None, None);
    assert!(all.matches(0xFF, &[]));
    assert!(all.matches(RECORD_FILL, &fill_payload(1, 2, 3)));
}

#[test]
fn test_record_types_are_or_ed() {
    let fills_and_bbos = filters(&["FILL", "BBO"], None, None, None, None);
    assert!(fills_and_bbos.matches(RECORD_FILL, &[]));
    assert!(fills_and_bbos.matches(RECORD_BBO, &[]));
    assert!(!fills_and_bbos.matches(RECORD_ORDER_FAILED, &[]));
}

#[test]
fn test_symbol_and_user_filters() {
    let payload = fill_payload(1000, 7, 100);
    assert!(filters(&[], Some(7), Some(100), None, None).matches(RECORD_FILL, &payload));
    assert!(!filters(&[], Some(8), None, None, None).matches(RECORD_FILL, &payload));
    assert!(!filters(&[], None, Some(101), None, None).matches(RECORD_FILL, &payload));
}

#[test]
fn test_record_without_the_field_never_matches_its_filter() {
    let payload = vec![0u8; 64];
    assert!(!filters(&[], Some(0), None, None, None).matches(RECORD_ORDER_FAILED, &payload));
    assert!(!filters(&[], None, Some(0), None, None).matches(RECORD_BBO, &payload));
}

#[test]
fn test_ts_window_is_inclusive() {
    let window = filters(&[], None, None, Some(2000), Some(4000));
    assert!(!window.matches(RECORD_FILL, &fill_payload(1999, 1, 1)));
    assert!(window.matches(RECORD_FILL, &fill_payload(2000, 1, 1)));
    assert!(window.matches(RECORD_FILL, &fill_payload(4000, 1, 1)));
    assert!(!window.matches(RECORD_FILL, &fill_payload(4001, 1, 1)));
}

#[test]
fn test_ts_window_ignores_payload_too_short_for_ts() {
    let window = filters(&[], None, None, Some(2000), Some(4000));
    assert!(window.matches(RECORD_FILL, &[0u8; 12]));
}

#[test]
fn test_extractors_read_each_layout() {
    let payload = fill_payload(1234, 7, 100);
    assert_eq!(extract_ts_ns(&payload), Some(1234));
    assert_eq!(extract_symbol_id(RECORD_FILL, &payload), Some(7));
    assert_eq!(extract_user_id(RECORD_FILL, &payload), Some(100));

    let mut request = vec![0u8; 64];
    request[8..12].copy_from_slice(&5u32.to_le_bytes());
    request[12..16].copy_from_slice(&6u32.to_le_bytes());
    assert_eq!(extract_user_id(RECORD_ORDER_REQUEST, &request), Some(5));
    assert_eq!(extract_symbol_id(RECORD_ORDER_REQUEST, &request), Some(6));
}

#[test]
fn test_extractors_return_none_on_short_payload() {
    assert_eq!(extract_ts_ns(&[0u8; 15]), None);
    assert_eq!(extract_symbol_id(RECORD_FILL, &[0u8; 19]), None);
    assert_eq!(extract_user_id(RECORD_FILL, &[0u8; 23]), None);
}

fn tail_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rsx_cli_tail_{}_{}", std::process::id(), name));
    fs::create_dir_all(dir.join("1")).unwrap();
    dir
}

fn frames(seqs: &[u64]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for seq in seqs {
        let mut payload = vec![0u8; 16];
        payload[..8].copy_from_slice(&seq.to_le_bytes());
        payload[8..16].copy_from_slice(&(seq * 1000).to_le_bytes());
        bytes.extend(encode_record(0xFE, &payload));
    }
    bytes
}

fn append(path: &Path, seqs: &[u64]) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    file.write_all(&frames(seqs)).unwrap();
}

fn drain_seqs(tail: &mut Tail) -> Vec<u64> {
    let mut seqs = Vec::new();
    while let Some(raw) = tail.next_record().unwrap() {
        seqs.push(extract_seq(&raw.payload).unwrap());
    }
    seqs
}

#[test]
fn test_idle_polls_do_not_reopen_the_reader() {
    let dir = tail_dir("idle");
    let active = dir.join("1").join("1_active.wal");
    append(&active, &[1, 2, 3]);
    let mut tail = Tail::open(1, &dir, 1).unwrap();
    assert_eq!(drain_seqs(&mut tail), vec![1, 2, 3]);
    for _ in 0..5 {
        assert!(!tail.refresh().unwrap());
        assert!(tail.next_record().unwrap().is_none());
    }
}

#[test]
fn test_appended_bytes_reopen_once_and_emit_the_new_record_once() {
    let dir = tail_dir("append");
    let active = dir.join("1").join("1_active.wal");
    append(&active, &[1, 2, 3]);
    let mut tail = Tail::open(1, &dir, 1).unwrap();
    assert_eq!(drain_seqs(&mut tail), vec![1, 2, 3]);

    append(&active, &[4]);
    assert!(tail.refresh().unwrap());
    assert_eq!(drain_seqs(&mut tail), vec![4]);
    assert!(!tail.refresh().unwrap());
    assert!(drain_seqs(&mut tail).is_empty());
}

#[test]
fn test_rotation_reopens_and_continues_in_the_new_segment() {
    let dir = tail_dir("rotate");
    let stream_dir = dir.join("1");
    let active = stream_dir.join("1_active.wal");
    append(&active, &[1, 2]);
    let mut tail = Tail::open(1, &dir, 1).unwrap();
    assert_eq!(drain_seqs(&mut tail), vec![1, 2]);

    fs::rename(&active, stream_dir.join("1_1_2.wal")).unwrap();
    append(&active, &[3]);
    assert!(tail.refresh().unwrap());
    assert_eq!(drain_seqs(&mut tail), vec![3]);
}
