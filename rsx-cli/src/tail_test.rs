use crate::tail::Tail;
use rsx_cast::encode_utils::encode_record;
use rsx_cast::wal::extract_seq;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

const RECORD_TYPE: u16 = 0xFE;
const FRAME_BYTES: usize = 32;

fn wal_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rsx_cli_tail_{}_{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("1")).unwrap();
    dir
}

fn frame(seq: u64) -> Vec<u8> {
    let mut payload = vec![0u8; 16];
    payload[..8].copy_from_slice(&seq.to_le_bytes());
    payload[8..16].copy_from_slice(&(seq * 1000).to_le_bytes());
    encode_record(RECORD_TYPE, &payload)
}

fn frames(seqs: &[u64]) -> Vec<u8> {
    seqs.iter().flat_map(|seq| frame(*seq)).collect()
}

fn segment(dir: &Path, name: &str) -> PathBuf {
    dir.join("1").join(name)
}

fn append(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
}

fn drain(tail: &mut Tail) -> Vec<u64> {
    let mut seqs = Vec::new();
    while let Some(raw) = tail.next_record().unwrap() {
        seqs.push(extract_seq(&raw.payload).unwrap());
    }
    seqs
}

#[test]
fn test_emits_the_stream_then_each_appended_record_once() {
    let dir = wal_dir("append");
    let active = segment(&dir, "1_active.wal");
    append(&active, &frames(&[1, 2, 3]));
    let mut tail = Tail::new(1, &dir, 0);
    assert_eq!(drain(&mut tail), vec![1, 2, 3]);
    assert!(drain(&mut tail).is_empty());

    append(&active, &frames(&[4]));
    assert_eq!(drain(&mut tail), vec![4]);
    assert!(drain(&mut tail).is_empty());
}

#[test]
fn test_an_idle_poll_does_not_read_the_segment_again() {
    let dir = wal_dir("idle");
    let active = segment(&dir, "1_active.wal");
    append(&active, &frames(&[1, 2, 3]));
    let mut tail = Tail::new(1, &dir, 0);
    assert_eq!(drain(&mut tail), vec![1, 2, 3]);

    let mut bytes = fs::read(&active).unwrap();
    bytes[16 + 3] ^= 0xFF;
    fs::write(&active, &bytes).unwrap();
    for _ in 0..3 {
        assert!(tail.next_record().unwrap().is_none());
    }
    append(&active, &frames(&[4]));
    assert_eq!(drain(&mut tail), vec![4]);
}

#[test]
fn test_starts_at_from_seq_and_skips_earlier_records() {
    let dir = wal_dir("from_seq");
    append(&segment(&dir, "1_1_3.wal"), &frames(&[1, 2, 3]));
    append(&segment(&dir, "1_active.wal"), &frames(&[4, 5, 6]));
    let mut tail = Tail::new(1, &dir, 3);
    assert_eq!(drain(&mut tail), vec![3, 4, 5, 6]);
}

#[test]
fn test_a_stream_with_no_segment_yet_is_idle_until_one_appears() {
    let dir = wal_dir("empty");
    let mut tail = Tail::new(1, &dir, 0);
    assert!(drain(&mut tail).is_empty());
    append(&segment(&dir, "1_active.wal"), &frames(&[1]));
    assert_eq!(drain(&mut tail), vec![1]);
}

#[test]
fn test_a_record_cut_short_by_a_concurrent_write_is_waited_out() {
    let dir = wal_dir("partial");
    let active = segment(&dir, "1_active.wal");
    let second = frame(2);
    let mut bytes = frame(1);
    bytes.extend_from_slice(&second[..20]);
    append(&active, &bytes);
    let mut tail = Tail::new(1, &dir, 0);
    assert_eq!(drain(&mut tail), vec![1]);
    assert!(drain(&mut tail).is_empty());

    append(&active, &second[20..]);
    append(&active, &frame(3));
    assert_eq!(drain(&mut tail), vec![2, 3]);
}

#[test]
fn test_rotation_drains_the_old_segment_then_follows_the_new_one() {
    let dir = wal_dir("rotate");
    let active = segment(&dir, "1_active.wal");
    append(&active, &frames(&[1, 2]));
    let mut tail = Tail::new(1, &dir, 0);
    assert_eq!(drain(&mut tail), vec![1, 2]);

    append(&active, &frames(&[3]));
    fs::rename(&active, segment(&dir, "1_1_3.wal")).unwrap();
    append(&active, &frames(&[4, 5]));
    assert_eq!(drain(&mut tail), vec![3, 4, 5]);

    append(&active, &frames(&[6]));
    assert_eq!(drain(&mut tail), vec![6]);
}

#[test]
fn test_rotation_before_the_first_read_starts_in_the_right_segment() {
    let dir = wal_dir("rotate_first");
    append(&segment(&dir, "1_1_3.wal"), &frames(&[1, 2, 3]));
    append(&segment(&dir, "1_active.wal"), &frames(&[4]));
    let mut tail = Tail::new(1, &dir, 4);
    assert_eq!(drain(&mut tail), vec![4]);
}

#[test]
fn test_a_bad_record_with_data_after_it_is_an_error() {
    let dir = wal_dir("bad_middle");
    let mut bytes = frames(&[1, 2, 3]);
    bytes[FRAME_BYTES + 16 + 3] ^= 0xFF;
    append(&segment(&dir, "1_active.wal"), &bytes);
    let mut tail = Tail::new(1, &dir, 0);
    assert_eq!(tail.next_record().unwrap().map(|_| 1), Some(1));
    let failure = tail.next_record().err().unwrap();
    assert!(
        failure
            .message()
            .starts_with("wal read error: crc mismatch"),
        "{}",
        failure.message()
    );
}

#[test]
fn test_a_bad_record_at_the_end_of_the_live_segment_is_waited_out() {
    let dir = wal_dir("bad_tail");
    let active = segment(&dir, "1_active.wal");
    let mut bytes = frames(&[1, 2]);
    bytes[FRAME_BYTES + 16 + 3] ^= 0xFF;
    append(&active, &bytes);
    let mut tail = Tail::new(1, &dir, 0);
    assert_eq!(drain(&mut tail), vec![1]);
    assert!(drain(&mut tail).is_empty());

    fs::write(&active, frames(&[1, 2, 3])).unwrap();
    assert_eq!(drain(&mut tail), vec![2, 3]);
}

#[test]
fn test_a_bad_record_in_a_segment_the_writer_left_is_an_error() {
    let dir = wal_dir("bad_rotated");
    let rotated = segment(&dir, "1_1_2.wal");
    let mut bytes = frames(&[1, 2]);
    bytes[FRAME_BYTES + 16 + 3] ^= 0xFF;
    append(&rotated, &bytes);
    append(&segment(&dir, "1_active.wal"), &frames(&[3]));
    let mut tail = Tail::new(1, &dir, 0);
    assert_eq!(tail.next_record().unwrap().map(|_| 1), Some(1));
    let failure = tail.next_record().err().unwrap();
    assert!(
        failure
            .message()
            .starts_with("wal read error: crc mismatch"),
        "{}",
        failure.message()
    );
}

#[test]
fn test_a_partial_record_in_a_segment_the_writer_left_is_an_error() {
    let dir = wal_dir("partial_rotated");
    let mut bytes = frames(&[1]);
    bytes.extend_from_slice(&frame(2)[..20]);
    append(&segment(&dir, "1_1_2.wal"), &bytes);
    append(&segment(&dir, "1_active.wal"), &frames(&[3]));
    let mut tail = Tail::new(1, &dir, 0);
    assert_eq!(tail.next_record().unwrap().map(|_| 1), Some(1));
    let failure = tail.next_record().err().unwrap();
    assert!(
        failure
            .message()
            .starts_with("wal read error: truncated record"),
        "{}",
        failure.message()
    );
}
