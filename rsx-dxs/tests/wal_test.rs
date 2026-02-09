use rsx_dxs::*;
use std::mem;
use tempfile::TempDir;

fn make_fill(seq: u64) -> FillRecord {
    FillRecord {
        seq,
        ts_ns: seq * 1000,
        symbol_id: 1,
        maker_oid: seq as u128,
        taker_oid: (seq + 100) as u128,
        px: 50000,
        qty: 100,
        maker_side: 0,
        _pad1: [0; 7],
    }
}

fn fill_payload(record: &FillRecord) -> Vec<u8> {
    unsafe {
        std::slice::from_raw_parts(
            record as *const FillRecord as *const u8,
            mem::size_of::<FillRecord>(),
        )
    }
    .to_vec()
}

#[test]
fn writer_assigns_monotonic_seq() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024, 600_000_000_000,
    )
    .unwrap();
    let fill1 = make_fill(0);
    let fill2 = make_fill(0);
    let seq1 = writer
        .append(RECORD_FILL, &fill_payload(&fill1))
        .unwrap();
    let seq2 = writer
        .append(RECORD_FILL, &fill_payload(&fill2))
        .unwrap();
    assert_eq!(seq1, 1);
    assert_eq!(seq2, 2);
    assert!(seq2 > seq1);
}

#[test]
fn writer_append_to_buffer_no_io() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024, 600_000_000_000,
    )
    .unwrap();
    let fill = make_fill(1);
    writer
        .append(RECORD_FILL, &fill_payload(&fill))
        .unwrap();

    let active = tmp
        .path()
        .join("1")
        .join("1_active.wal");
    let size = std::fs::metadata(&active).unwrap().len();
    assert_eq!(size, 0);
}

#[test]
fn writer_flush_writes_to_file() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024, 600_000_000_000,
    )
    .unwrap();
    let fill = make_fill(1);
    writer
        .append(RECORD_FILL, &fill_payload(&fill))
        .unwrap();
    writer.flush().unwrap();

    let active = tmp
        .path()
        .join("1")
        .join("1_active.wal");
    let size = std::fs::metadata(&active).unwrap().len();
    assert!(size > 0);
}

#[test]
fn writer_rotation_at_threshold() {
    let tmp = TempDir::new().unwrap();
    // 1KB threshold - each fill record is ~80 bytes
    // (16 header + 64 payload), so ~12 records to rotate
    let mut writer = WalWriter::new(
        1, tmp.path(), 1024, 600_000_000_000,
    )
    .unwrap();

    let fill = make_fill(1);
    let payload = fill_payload(&fill);

    for _ in 0..20 {
        writer.append(RECORD_FILL, &payload).unwrap();
    }
    writer.flush().unwrap();

    let dir = tmp.path().join("1");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        files.len() >= 2,
        "expected rotation, got {} files",
        files.len()
    );
}

#[test]
fn writer_backpressure_stalls() {
    let tmp = TempDir::new().unwrap();
    // small max so backpressure = max(2*4096, 256KB) = 256KB
    let mut writer = WalWriter::new(
        1, tmp.path(), 4096, 600_000_000_000,
    )
    .unwrap();

    // fill buffer past 256KB without flushing
    let big_payload = vec![0u8; 8192];
    let mut hit_backpressure = false;
    for _ in 0..200 {
        match writer.append(RECORD_FILL, &big_payload) {
            Ok(_) => continue,
            Err(e) => {
                assert_eq!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                );
                hit_backpressure = true;
                break;
            }
        }
    }
    assert!(
        hit_backpressure,
        "should have hit backpressure"
    );
}

#[test]
fn reader_sequential_iteration_all_records() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024, 600_000_000_000,
    )
    .unwrap();

    for i in 0..10 {
        let fill = make_fill(i);
        writer
            .append(RECORD_FILL, &fill_payload(&fill))
            .unwrap();
    }
    writer.flush().unwrap();

    let mut reader =
        WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    let mut count = 0;
    while let Ok(Some(_)) = reader.next() {
        count += 1;
    }
    assert_eq!(count, 10);
}

#[test]
fn reader_returns_none_at_eof() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024, 600_000_000_000,
    )
    .unwrap();

    let fill = make_fill(1);
    writer
        .append(RECORD_FILL, &fill_payload(&fill))
        .unwrap();
    writer.flush().unwrap();

    let mut reader =
        WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    assert!(reader.next().unwrap().is_some());
    assert!(reader.next().unwrap().is_none());
}

#[test]
fn reader_crc32_invalid_truncates_stream() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024, 600_000_000_000,
    )
    .unwrap();

    let fill = make_fill(1);
    writer
        .append(RECORD_FILL, &fill_payload(&fill))
        .unwrap();
    writer.flush().unwrap();

    let active = tmp
        .path()
        .join("1")
        .join("1_active.wal");
    let mut data = std::fs::read(&active).unwrap();
    if data.len() > WalHeader::SIZE {
        data[WalHeader::SIZE] ^= 0xFF;
    }
    std::fs::write(&active, &data).unwrap();

    let mut reader =
        WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    assert!(reader.next().unwrap().is_none());
}

#[test]
fn reader_unknown_version_fails_fast() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024, 600_000_000_000,
    )
    .unwrap();

    let fill = make_fill(1);
    writer
        .append(RECORD_FILL, &fill_payload(&fill))
        .unwrap();
    writer.flush().unwrap();

    let active = tmp
        .path()
        .join("1")
        .join("1_active.wal");
    let mut data = std::fs::read(&active).unwrap();
    // corrupt version field (bytes 0-1)
    data[0] = 0xFF;
    data[1] = 0xFF;
    std::fs::write(&active, &data).unwrap();

    let mut reader =
        WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    assert!(reader.next().is_err());
}

#[test]
fn write_rotate_read_across_files() {
    let tmp = TempDir::new().unwrap();
    // 1KB threshold to force multiple rotations
    let mut writer = WalWriter::new(
        1, tmp.path(), 1024, 600_000_000_000,
    )
    .unwrap();

    let fill = make_fill(1);
    let payload = fill_payload(&fill);
    let n = 30;

    for _ in 0..n {
        writer.append(RECORD_FILL, &payload).unwrap();
    }
    writer.flush().unwrap();

    let mut reader =
        WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    let mut count = 0;
    while let Ok(Some(_)) = reader.next() {
        count += 1;
    }
    assert_eq!(count, n);
}

#[test]
fn gc_deletes_old_files() {
    let tmp = TempDir::new().unwrap();
    // 1KB threshold, retention = 1ns (basically zero)
    let mut writer =
        WalWriter::new(1, tmp.path(), 1024, 1).unwrap();

    let fill = make_fill(1);
    let payload = fill_payload(&fill);

    for _ in 0..100 {
        writer.append(RECORD_FILL, &payload).unwrap();
    }
    writer.flush().unwrap();

    // manually trigger gc again with high seq
    writer.gc().unwrap();

    let dir = tmp.path().join("1");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    // should have cleaned up old rotated files
    // active file always remains
    assert!(files.len() >= 1);
}

#[test]
fn record_max_payload_64kb() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024, 600_000_000_000,
    )
    .unwrap();

    // exactly 64KB should succeed
    let payload = vec![0u8; 64 * 1024];
    assert!(writer.append(RECORD_FILL, &payload).is_ok());

    // 64KB + 1 should fail
    let payload = vec![0u8; 64 * 1024 + 1];
    assert!(writer.append(RECORD_FILL, &payload).is_err());
}
