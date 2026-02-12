use rsx_dxs::header::WalHeader;
use rsx_dxs::records::RECORD_BBO;
use rsx_dxs::records::RECORD_FILL;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn make_test_dir() -> PathBuf {
    let dir = PathBuf::from("./tmp/cli_test");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_test_wal(path: &PathBuf, records: usize) {
    let mut file = File::create(path).unwrap();
    for i in 0..records {
        let header = WalHeader::new(
            RECORD_BBO,
            8,
            i as u32,
        );
        let payload = vec![i as u8; 8];
        file.write_all(&header.to_bytes()).unwrap();
        file.write_all(&payload).unwrap();
    }
    file.sync_all().unwrap();
}

#[test]
fn test_dump_file_parsing() {
    let dir = make_test_dir();
    let file_path = dir.join("test.wal");
    write_test_wal(&file_path, 5);

    assert!(file_path.exists());
    let metadata = fs::metadata(&file_path).unwrap();
    assert_eq!(metadata.len(), 5 * (16 + 8));
}

#[test]
fn test_wal_header_format() {
    let header = WalHeader::new(
        RECORD_FILL,
        100,
        0xDEADBEEF,
    );

    let bytes = header.to_bytes();
    assert_eq!(bytes.len(), 16);

    let record_type =
        u16::from_le_bytes([bytes[0], bytes[1]]);
    let len =
        u16::from_le_bytes([bytes[2], bytes[3]]);
    let crc32 = u32::from_le_bytes([
        bytes[4], bytes[5], bytes[6], bytes[7],
    ]);

    assert_eq!(record_type, RECORD_FILL);
    assert_eq!(len, 100);
    assert_eq!(crc32, 0xDEADBEEF);
}

#[test]
fn test_multiple_records_in_file() {
    let dir = make_test_dir();
    let file_path = dir.join("multi.wal");
    let mut file = File::create(&file_path).unwrap();

    for i in 0..3 {
        let header = WalHeader::new(
            RECORD_BBO,
            4,
            i,
        );
        let payload = vec![i as u8; 4];
        file.write_all(&header.to_bytes()).unwrap();
        file.write_all(&payload).unwrap();
    }
    file.sync_all().unwrap();
    drop(file);

    let contents = fs::read(&file_path).unwrap();
    assert_eq!(contents.len(), 3 * (16 + 4));
}

#[test]
fn test_json_output_format() {
    let json_str = r#"{"seq":123,"type":"BBO","len":32,"crc32":"0x12345678"}"#;
    let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap();

    assert_eq!(parsed["seq"], 123);
    assert_eq!(parsed["type"], "BBO");
    assert_eq!(parsed["len"], 32);
    assert_eq!(parsed["crc32"], "0x12345678");
}

#[test]
fn test_file_empty_handling() {
    let dir = make_test_dir();
    let empty_file = dir.join("empty.wal");
    File::create(&empty_file).unwrap();

    let contents = fs::read(&empty_file).unwrap();
    assert_eq!(contents.len(), 0);
}
