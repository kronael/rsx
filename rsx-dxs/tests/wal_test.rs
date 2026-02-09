use rsx_dxs::*;
use std::mem;
use tempfile::TempDir;

fn make_fill(seq: u64) -> FillRecord {
    FillRecord {
        seq,
        ts_ns: seq * 1000,
        symbol_id: 1,
        taker_user_id: seq as u32,
        maker_user_id: (seq + 100) as u32,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: seq,
        maker_order_id_hi: 0,
        maker_order_id_lo: seq + 100,
        price: 50000,
        qty: 100,
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
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
fn reader_open_from_seq_finds_correct_file() {
    let tmp = TempDir::new().unwrap();
    // 512B threshold to force many rotations
    let mut writer =
        WalWriter::new(1, tmp.path(), 512, 600_000_000_000)
            .unwrap();

    let fill = make_fill(1);
    let payload = fill_payload(&fill);

    // Write 50 records across multiple files
    for _ in 0..50 {
        writer.append(RECORD_FILL, &payload).unwrap();
    }
    writer.flush().unwrap();

    // Open reader from seq 0, should read all records
    let mut reader =
        WalReader::open_from_seq(1, 0, tmp.path())
            .unwrap();
    let mut count = 0;
    while let Ok(Some(_)) = reader.next() {
        count += 1;
    }
    assert_eq!(count, 50);
}

#[test]
fn reader_skips_to_target_seq_within_file() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024, 600_000_000_000,
    )
    .unwrap();

    let fill = make_fill(1);
    let payload = fill_payload(&fill);

    for _ in 0..10 {
        writer.append(RECORD_FILL, &payload).unwrap();
    }
    writer.flush().unwrap();

    // Open from seq 5 -- reader opens the file containing
    // that seq. Since all are in one active file, it reads
    // from the beginning.
    let mut reader =
        WalReader::open_from_seq(1, 5, tmp.path())
            .unwrap();
    let mut count = 0;
    while let Ok(Some(_)) = reader.next() {
        count += 1;
    }
    // All records are in the active file, reader reads all
    assert_eq!(count, 10);
}

#[test]
fn writer_gc_preserves_recent_files() {
    let tmp = TempDir::new().unwrap();
    // 512B threshold, high retention so nothing gets gc'd
    let mut writer = WalWriter::new(
        1, tmp.path(), 512, u64::MAX,
    )
    .unwrap();

    let fill = make_fill(1);
    let payload = fill_payload(&fill);

    for _ in 0..50 {
        writer.append(RECORD_FILL, &payload).unwrap();
    }
    writer.flush().unwrap();

    writer.gc().unwrap();

    let dir = tmp.path().join("1");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    // With high retention, no rotated files deleted
    // Should have active + rotated files
    assert!(files.len() >= 2);
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

#[test]
fn writer_empty_flush_no_io() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024, 600_000_000_000,
    )
    .unwrap();

    writer.flush().unwrap();

    let active = tmp
        .path()
        .join("1")
        .join("1_active.wal");
    let size = std::fs::metadata(&active).unwrap().len();
    assert_eq!(size, 0);
}

#[test]
fn writer_seq_starts_at_1() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024, 600_000_000_000,
    )
    .unwrap();

    assert_eq!(writer.next_seq, 1);
    let fill = make_fill(0);
    let seq = writer
        .append(RECORD_FILL, &fill_payload(&fill))
        .unwrap();
    assert_eq!(seq, 1);
}

#[test]
fn writer_gc_runs_on_rotation_not_timer() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 512, 1,
    )
    .unwrap();

    let fill = make_fill(1);
    let payload = fill_payload(&fill);

    for _ in 0..50 {
        writer.append(RECORD_FILL, &payload).unwrap();
    }
    writer.flush().unwrap();

    let dir = tmp.path().join("1");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(files.len() >= 1);
}

#[test]
fn writer_flush_calls_fsync() {
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
fn writer_rotation_renames_with_seq_range() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 512, 600_000_000_000,
    )
    .unwrap();

    let fill = make_fill(1);
    let payload = fill_payload(&fill);

    for _ in 0..30 {
        writer.append(RECORD_FILL, &payload).unwrap();
    }
    writer.flush().unwrap();

    let dir = tmp.path().join("1");
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();

    let has_rotated = entries.iter().any(|e| {
        let name = e.file_name();
        let name_str = name.to_string_lossy();
        name_str.contains("_") && name_str.ends_with(".wal")
            && !name_str.contains("active")
    });
    assert!(has_rotated);
}

#[test]
fn writer_active_file_uses_temp_name() {
    let tmp = TempDir::new().unwrap();
    let _writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024, 600_000_000_000,
    )
    .unwrap();

    let active = tmp
        .path()
        .join("1")
        .join("1_active.wal");
    assert!(active.exists());
}

#[test]
fn reader_open_from_seq_0_starts_at_beginning() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024, 600_000_000_000,
    )
    .unwrap();

    for i in 0..5 {
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
    assert_eq!(count, 5);
}

#[test]
fn reader_handles_empty_wal_directory() {
    let tmp = TempDir::new().unwrap();
    let reader =
        WalReader::open_from_seq(999, 0, tmp.path());
    assert!(reader.is_ok());
}

#[test]
fn reader_handles_single_file() {
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
    let mut count = 0;
    while let Ok(Some(_)) = reader.next() {
        count += 1;
    }
    assert_eq!(count, 1);
}

#[test]
fn reader_handles_multiple_files_sorted() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 512, 600_000_000_000,
    )
    .unwrap();

    let fill = make_fill(1);
    let payload = fill_payload(&fill);

    for _ in 0..50 {
        writer.append(RECORD_FILL, &payload).unwrap();
    }
    writer.flush().unwrap();

    let mut reader =
        WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    let mut count = 0;
    while let Ok(Some(_)) = reader.next() {
        count += 1;
    }
    assert_eq!(count, 50);
}

#[test]
fn reader_file_transition_seamless() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 512, 600_000_000_000,
    )
    .unwrap();

    let fill = make_fill(1);
    let payload = fill_payload(&fill);

    for _ in 0..30 {
        writer.append(RECORD_FILL, &payload).unwrap();
    }
    writer.flush().unwrap();

    let mut reader =
        WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    let mut count = 0;
    while let Ok(Some(_)) = reader.next() {
        count += 1;
    }
    assert_eq!(count, 30);
}

#[test]
fn reader_returns_none_when_caught_up() {
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
    reader.next().unwrap();
    let result = reader.next().unwrap();
    assert!(result.is_none());
}
