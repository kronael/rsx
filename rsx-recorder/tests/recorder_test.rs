use rsx_dxs::header::WalHeader;
use rsx_dxs::records::RECORD_BBO;
use rsx_dxs::wal::WalReader;
use rsx_dxs::RawWalRecord;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn make_test_dir() -> PathBuf {
    let dir = PathBuf::from("./tmp/recorder_test");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_wal_record_serialization() {
    let header = WalHeader::new(
        RECORD_BBO,
        32,
        0x12345678,
    );
    let payload = vec![0u8; 32];
    let record = RawWalRecord { header, payload };

    let bytes = header.to_bytes();
    assert_eq!(bytes.len(), 16);
    assert_eq!(bytes[0..2], RECORD_BBO.to_le_bytes());
    assert_eq!(bytes[2..4], 32u16.to_le_bytes());
}

#[test]
fn test_archive_file_creation() {
    let dir = make_test_dir();
    let stream_dir = dir.join("42");
    fs::create_dir_all(&stream_dir).unwrap();

    let file_path = stream_dir.join("42_2026-02-12.wal");
    let mut file = File::create(&file_path).unwrap();
    file.write_all(&[1, 2, 3, 4]).unwrap();
    file.sync_all().unwrap();

    assert!(file_path.exists());
    let contents = fs::read(&file_path).unwrap();
    assert_eq!(contents, vec![1, 2, 3, 4]);
}

#[test]
fn test_daily_rotation_naming() {
    let dir = make_test_dir();
    let stream_id = 5u32;
    let stream_dir = dir.join(stream_id.to_string());
    fs::create_dir_all(&stream_dir).unwrap();

    let day1 = stream_dir.join("5_2026-02-12.wal");
    let day2 = stream_dir.join("5_2026-02-13.wal");

    File::create(&day1).unwrap();
    File::create(&day2).unwrap();

    assert!(day1.exists());
    assert!(day2.exists());
}

#[test]
fn test_buffered_writes() {
    let dir = make_test_dir();
    let file_path = dir.join("test.wal");
    let mut file = File::create(&file_path).unwrap();

    let mut buf = Vec::with_capacity(1024);
    for i in 0..10u8 {
        buf.push(i);
    }

    file.write_all(&buf).unwrap();
    file.sync_all().unwrap();
    drop(file);

    let contents = fs::read(&file_path).unwrap();
    assert_eq!(contents.len(), 10);
    assert_eq!(contents[0], 0);
    assert_eq!(contents[9], 9);
}

#[test]
fn test_wal_roundtrip() {
    let dir = make_test_dir();
    let stream_dir = dir.join("99");
    fs::create_dir_all(&stream_dir).unwrap();

    let file_path = stream_dir.join("test.wal");
    let mut file = File::create(&file_path).unwrap();

    let header = WalHeader::new(
        RECORD_BBO,
        8,
        0xAABBCCDD,
    );
    let payload = vec![1u8, 2, 3, 4, 5, 6, 7, 8];

    file.write_all(&header.to_bytes()).unwrap();
    file.write_all(&payload).unwrap();
    file.sync_all().unwrap();
    drop(file);

    let mut reader =
        WalReader::open_from_seq(99, 0, &dir).unwrap();
    let record = reader.next().unwrap().unwrap();

    assert_eq!(record.header.record_type, RECORD_BBO);
    assert_eq!(record.header.len, 8);
    assert_eq!(record.payload.len(), 8);
    assert_eq!(record.payload[0], 1);
    assert_eq!(record.payload[7], 8);
}
