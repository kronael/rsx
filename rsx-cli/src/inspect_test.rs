use crate::inspect::find_streams;
use crate::inspect::list_stream;
use crate::inspect::verify_stream;
use rsx_cast::encode_utils::encode_record;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

const RECORD_TYPE: u16 = 0xFE;

fn wal_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rsx_cli_inspect_{}_{}", std::process::id(), name));
    fs::create_dir_all(dir.join("1")).unwrap();
    dir
}

fn frames(seqs: &[u64]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for seq in seqs {
        let mut payload = vec![0u8; 16];
        payload[..8].copy_from_slice(&seq.to_le_bytes());
        payload[8..16].copy_from_slice(&(seq * 1000).to_le_bytes());
        bytes.extend(encode_record(RECORD_TYPE, &payload));
    }
    bytes
}

fn write_segment(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join("1").join(name);
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn test_clean_stream_across_segments_is_ok() {
    let dir = wal_dir("clean");
    write_segment(&dir, "1_1_3.wal", &frames(&[1, 2, 3]));
    write_segment(&dir, "1_active.wal", &frames(&[4, 5]));
    let verdict = verify_stream(1, &dir).unwrap();
    assert_eq!(verdict.records, 5);
    assert_eq!((verdict.first_seq, verdict.last_seq), (Some(1), Some(5)));
    assert_eq!(verdict.crc_errors, 0);
    assert!(verdict.seq_gaps.is_empty());
    assert!(!verdict.torn_tail);
    assert!(!verdict.is_corrupt());
}

#[test]
fn test_flipped_byte_is_one_crc_error_without_a_phantom_gap() {
    let dir = wal_dir("flipped");
    let mut bytes = frames(&[1, 2, 3, 4]);
    bytes[32 + 16 + 3] ^= 0xFF;
    write_segment(&dir, "1_active.wal", &bytes);
    let verdict = verify_stream(1, &dir).unwrap();
    assert_eq!(verdict.crc_errors, 1);
    assert_eq!(verdict.records, 3);
    assert!(verdict.seq_gaps.is_empty());
    assert!(verdict.is_corrupt());
}

#[test]
fn test_flipped_byte_in_a_rotated_segment_is_found() {
    let dir = wal_dir("flipped_rotated");
    let mut bytes = frames(&[1, 2, 3]);
    bytes[16 + 3] ^= 0xFF;
    write_segment(&dir, "1_1_3.wal", &bytes);
    write_segment(&dir, "1_active.wal", &frames(&[4]));
    let verdict = verify_stream(1, &dir).unwrap();
    assert_eq!(verdict.crc_errors, 1);
    assert!(verdict.is_corrupt());
}

#[test]
fn test_missing_records_are_a_seq_gap() {
    let dir = wal_dir("gap");
    write_segment(&dir, "1_1_2.wal", &frames(&[1, 2]));
    write_segment(&dir, "1_active.wal", &frames(&[6, 7]));
    let verdict = verify_stream(1, &dir).unwrap();
    assert_eq!(verdict.seq_gaps, vec![(3, 5)]);
    assert!(verdict.is_corrupt());
}

#[test]
fn test_first_segment_may_start_above_seq_one() {
    let dir = wal_dir("pruned_head");
    write_segment(&dir, "1_active.wal", &frames(&[40, 41]));
    let verdict = verify_stream(1, &dir).unwrap();
    assert!(verdict.seq_gaps.is_empty());
    assert_eq!(verdict.first_seq, Some(40));
}

#[test]
fn test_seq_that_goes_backwards_is_an_error() {
    let dir = wal_dir("backwards");
    write_segment(&dir, "1_active.wal", &frames(&[1, 2, 2]));
    let verdict = verify_stream(1, &dir).unwrap();
    assert!(verdict.error.is_some());
    assert!(verdict.is_corrupt());
}

#[test]
fn test_partial_record_at_the_end_of_the_last_segment_is_a_torn_tail() {
    let dir = wal_dir("torn");
    let mut bytes = frames(&[1, 2]);
    bytes.extend(&frames(&[3])[..20]);
    write_segment(&dir, "1_active.wal", &bytes);
    let verdict = verify_stream(1, &dir).unwrap();
    assert!(verdict.torn_tail);
    assert_eq!(verdict.records, 2);
    assert!(!verdict.is_corrupt());
}

#[test]
fn test_partial_record_in_a_rotated_segment_is_an_error() {
    let dir = wal_dir("truncated_rotated");
    let mut bytes = frames(&[1, 2]);
    bytes.extend(&frames(&[3])[..20]);
    write_segment(&dir, "1_1_3.wal", &bytes);
    write_segment(&dir, "1_active.wal", &frames(&[4]));
    let verdict = verify_stream(1, &dir).unwrap();
    assert!(!verdict.torn_tail);
    assert!(verdict.error.is_some());
    assert!(verdict.is_corrupt());
}

#[test]
fn test_unknown_header_version_is_an_error() {
    let dir = wal_dir("bad_header");
    let mut bytes = frames(&[1, 2]);
    bytes[32] = 9;
    write_segment(&dir, "1_active.wal", &bytes);
    let verdict = verify_stream(1, &dir).unwrap();
    assert_eq!(verdict.records, 1);
    assert!(verdict.error.is_some());
}

#[test]
fn test_listing_reports_files_bytes_and_the_last_record() {
    let dir = wal_dir("listing");
    write_segment(&dir, "1_1_3.wal", &frames(&[1, 2, 3]));
    write_segment(&dir, "1_active.wal", &frames(&[4, 5]));
    let listing = list_stream(1, &dir).unwrap();
    assert_eq!(listing.files, 2);
    assert_eq!(listing.bytes, 5 * 32);
    assert_eq!((listing.first_seq, listing.last_seq), (Some(1), Some(5)));
    assert_eq!(listing.last_ts_ns, Some(5000));
}

#[test]
fn test_listing_skips_an_empty_last_segment() {
    let dir = wal_dir("listing_empty_active");
    write_segment(&dir, "1_1_3.wal", &frames(&[1, 2, 3]));
    write_segment(&dir, "1_active.wal", &[]);
    let listing = list_stream(1, &dir).unwrap();
    assert_eq!(listing.files, 2);
    assert_eq!(listing.last_seq, Some(3));
}

#[test]
fn test_listing_an_empty_stream_has_no_seq() {
    let dir = wal_dir("listing_none");
    let listing = list_stream(1, &dir).unwrap();
    assert_eq!(listing.files, 0);
    assert_eq!(listing.first_seq, None);
    assert_eq!(listing.last_seq, None);
    assert_eq!(listing.last_ts_ns, None);
}

#[test]
fn test_find_streams_takes_numeric_directories_only() {
    let dir = wal_dir("find");
    fs::create_dir_all(dir.join("12")).unwrap();
    fs::create_dir_all(dir.join("archive")).unwrap();
    fs::write(dir.join("3"), b"a file, not a stream").unwrap();
    assert_eq!(find_streams(&dir, &[]).unwrap(), vec![1, 12]);
    assert_eq!(find_streams(&dir, &[12, 12]).unwrap(), vec![12]);
}

#[test]
fn test_find_streams_rejects_a_stream_without_a_directory() {
    let dir = wal_dir("find_missing");
    let failure = find_streams(&dir, &[1, 99]).unwrap_err();
    assert!(failure
        .message()
        .starts_with("no wal directory for stream 99"));
    assert!(find_streams(&dir.join("absent"), &[]).is_err());
}
