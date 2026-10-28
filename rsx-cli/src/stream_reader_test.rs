use crate::stream_reader::StreamReader;
use crate::verify::scan_stream;
use rsx_cast::encode_utils::encode_record;
use rsx_cast::wal::extract_seq;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

const RECORD_TYPE: u16 = 0xFE;

fn wal_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rsx_cli_stream_reader_{}_{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&dir);
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

fn write_segment(dir: &Path, name: &str, bytes: &[u8]) {
    fs::write(dir.join("1").join(name), bytes).unwrap();
}

fn read_seqs(dir: &Path, from_seq: u64) -> Result<Vec<u64>, String> {
    let mut reader =
        StreamReader::open(1, from_seq, dir).map_err(|err| err.message().to_string())?;
    let mut seqs = Vec::new();
    while let Some(raw) = reader
        .next_record()
        .map_err(|err| err.message().to_string())?
    {
        seqs.push(extract_seq(&raw.payload).unwrap());
    }
    Ok(seqs)
}

#[test]
fn dated_archive_files_are_read_in_date_order() {
    let dir = wal_dir("dated_order");
    write_segment(&dir, "1_2026-10-27.wal", &frames(&[4, 5]));
    write_segment(&dir, "1_2026-10-26.wal", &frames(&[1, 2, 3]));
    assert_eq!(read_seqs(&dir, 0), Ok(vec![1, 2, 3, 4, 5]));
}

#[test]
fn writer_files_come_before_the_archive() {
    let dir = wal_dir("writer_then_archive");
    write_segment(&dir, "1_1_2.wal", &frames(&[1, 2]));
    write_segment(&dir, "1_active.wal", &frames(&[3]));
    write_segment(&dir, "1_2026-10-26.wal", &frames(&[4, 5]));
    assert_eq!(read_seqs(&dir, 0), Ok(vec![1, 2, 3, 4, 5]));
}

#[test]
fn segments_that_end_before_from_seq_are_not_read() {
    let dir = wal_dir("from_seq");
    write_segment(&dir, "1_1_3.wal", &frames(&[1, 2, 3]));
    write_segment(&dir, "1_4_6.wal", &frames(&[4, 5, 6]));
    assert_eq!(read_seqs(&dir, 5), Ok(vec![4, 5, 6]));
}

#[test]
fn a_torn_tail_in_the_last_segment_ends_the_stream() {
    let dir = wal_dir("torn_tail");
    let mut bytes = frames(&[1, 2]);
    bytes.extend(&frames(&[3])[..10]);
    write_segment(&dir, "1_2026-10-26.wal", &bytes);
    assert_eq!(read_seqs(&dir, 0), Ok(vec![1, 2]));
}

#[test]
fn a_bad_record_with_data_after_it_is_an_error() {
    let dir = wal_dir("bad_crc");
    let mut bytes = frames(&[1, 2, 3]);
    let first_payload = 16;
    bytes[first_payload] ^= 0xFF;
    write_segment(&dir, "1_2026-10-26.wal", &bytes);
    let error = read_seqs(&dir, 0).unwrap_err();
    assert!(error.contains("crc mismatch at byte 0"), "{error}");
}

#[test]
fn a_truncated_record_in_a_segment_before_the_last_is_an_error() {
    let dir = wal_dir("truncated_middle");
    let mut bytes = frames(&[1, 2]);
    bytes.truncate(bytes.len() - 4);
    write_segment(&dir, "1_2026-10-26.wal", &bytes);
    write_segment(&dir, "1_2026-10-27.wal", &frames(&[3]));
    let error = read_seqs(&dir, 0).unwrap_err();
    assert!(error.contains("truncated record"), "{error}");
}

#[test]
fn the_ledger_scan_reads_the_archive() {
    let dir = wal_dir("ledger_scan");
    write_segment(&dir, "1_2026-10-26.wal", &frames(&[1, 2, 3]));
    write_segment(&dir, "1_2026-10-27.wal", &frames(&[4, 5]));
    let ledger = scan_stream(&dir, 1, None, None).unwrap();
    assert_eq!(ledger.last_seq, 5);
}
