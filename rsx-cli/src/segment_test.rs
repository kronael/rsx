use crate::segment::Frame;
use crate::segment::SegmentReader;
use rsx_cast::encode_utils::encode_record;
use std::fs;
use std::path::PathBuf;

fn write_segment(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rsx_cli_segment_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn payload(seq: u64) -> Vec<u8> {
    let mut payload = vec![0u8; 16];
    payload[..8].copy_from_slice(&seq.to_le_bytes());
    payload
}

fn expect_record(frame: Option<Frame>) -> (u16, Vec<u8>, bool) {
    match frame {
        Some(Frame::Record {
            record_type,
            payload,
            is_crc_ok,
        }) => (record_type, payload, is_crc_ok),
        Some(Frame::Truncated) => panic!("expected a record, got Truncated"),
        Some(Frame::BadHeader) => panic!("expected a record, got BadHeader"),
        None => panic!("expected a record, got the end of the file"),
    }
}

#[test]
fn test_empty_file_ends_cleanly() {
    let path = write_segment("empty.wal", &[]);
    let mut reader = SegmentReader::open(&path).unwrap();
    assert!(reader.read_frame().unwrap().is_none());
}

#[test]
fn test_reads_records_and_tracks_the_offset() {
    let mut bytes = encode_record(7, &payload(1));
    bytes.extend(encode_record(8, &payload(2)));
    let path = write_segment("two.wal", &bytes);
    let mut reader = SegmentReader::open(&path).unwrap();

    let (record_type, first, is_crc_ok) = expect_record(reader.read_frame().unwrap());
    assert_eq!((record_type, first, is_crc_ok), (7, payload(1), true));
    assert_eq!(reader.offset, 32);
    let (record_type, second, is_crc_ok) = expect_record(reader.read_frame().unwrap());
    assert_eq!((record_type, second, is_crc_ok), (8, payload(2), true));
    assert_eq!(reader.offset, 64);
    assert!(reader.read_frame().unwrap().is_none());
}

#[test]
fn test_bad_crc_is_flagged_and_the_next_record_still_reads() {
    let mut bytes = encode_record(7, &payload(1));
    bytes.extend(encode_record(7, &payload(2)));
    bytes.extend(encode_record(7, &payload(3)));
    bytes[32 + 16 + 2] ^= 0xFF;
    let path = write_segment("bad_crc.wal", &bytes);
    let mut reader = SegmentReader::open(&path).unwrap();

    assert!(expect_record(reader.read_frame().unwrap()).2);
    assert!(!expect_record(reader.read_frame().unwrap()).2);
    let (_, third, is_crc_ok) = expect_record(reader.read_frame().unwrap());
    assert_eq!((third, is_crc_ok), (payload(3), true));
}

#[test]
fn test_file_ending_inside_a_header_or_payload_is_truncated() {
    let whole = encode_record(7, &payload(1));
    let inside_header = write_segment("cut_header.wal", &whole[..10]);
    let inside_payload = write_segment("cut_payload.wal", &whole[..24]);
    for path in [inside_header, inside_payload] {
        let mut reader = SegmentReader::open(&path).unwrap();
        assert!(matches!(
            reader.read_frame().unwrap(),
            Some(Frame::Truncated)
        ));
    }
}

#[test]
fn test_unknown_header_version_is_a_bad_header() {
    let mut bytes = encode_record(7, &payload(1));
    bytes[0] = 9;
    let path = write_segment("bad_version.wal", &bytes);
    let mut reader = SegmentReader::open(&path).unwrap();
    assert!(matches!(
        reader.read_frame().unwrap(),
        Some(Frame::BadHeader)
    ));
}
