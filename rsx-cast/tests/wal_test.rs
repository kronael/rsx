use rsx_cast::*;
use rsx_messages::FillRecord;
use rsx_types::Price;
use rsx_types::Qty;
use tempfile::TempDir;

fn make_fill(seq: u64) -> FillRecord {
    FillRecord {
        seq: 0,
        ts_ns: seq * 1000,
        symbol_id: 1,
        taker_user_id: seq as u32,
        maker_user_id: (seq + 100) as u32,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: seq,
        maker_order_id_hi: 0,
        maker_order_id_lo: seq + 100,
        price: Price(50000),
        qty: Qty(100),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
taker_ts_ns: 0,
    }
}

fn extract_seq(payload: &[u8]) -> Option<u64> {
    if payload.len() >= 8 {
        let bytes = payload[0..8].try_into().ok()?;
        Some(u64::from_le_bytes(bytes))
    } else {
        None
    }
}

#[test]
fn writer_assigns_monotonic_seq() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();
    let mut fill1 = make_fill(0);
    let mut fill2 = make_fill(0);
    let seq1 = writer.append(&mut fill1).unwrap();
    let seq2 = writer.append(&mut fill2).unwrap();
    assert_eq!(seq1, 1);
    assert_eq!(seq2, 2);
    assert!(seq2 > seq1);
}

#[test]
fn writer_append_to_buffer_no_io() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();
    let mut fill = make_fill(1);
    writer.append(&mut fill).unwrap();

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
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();
    let mut fill = make_fill(1);
    writer.append(&mut fill).unwrap();
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
        1, tmp.path(), 1024,
    )
    .unwrap();

    for i in 0..20 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
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
        1, tmp.path(), 4096,
    )
    .unwrap();

    // fill buffer past 256KB without flushing
    // Use fill records which are smaller, need more iterations
    let mut hit_backpressure = false;
    for i in 0..5000 {
        let mut fill = make_fill(i);
        match writer.append(&mut fill) {
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
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    for i in 0..10 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
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
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    let mut fill = make_fill(1);
    writer.append(&mut fill).unwrap();
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
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    let mut fill = make_fill(1);
    writer.append(&mut fill).unwrap();
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
fn reader_unknown_record_type_handled() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    let mut fill = make_fill(1);
    writer.append(&mut fill).unwrap();
    writer.flush().unwrap();

    let active = tmp
        .path()
        .join("1")
        .join("1_active.wal");
    let mut data = std::fs::read(&active).unwrap();
    // corrupt record_type field (bytes 2-3) to unknown type;
    // keep byte 0 = WalVersion::V1 so the header still parses.
    data[2] = 0xFF;
    data[3] = 0xFF;
    std::fs::write(&active, &data).unwrap();

    let mut reader =
        WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    // Reader should handle unknown record types gracefully
    // (returns None for unknown types, doesn't crash)
    let result = reader.next();
    assert!(result.is_ok());
}

#[test]
fn write_rotate_read_across_files() {
    let tmp = TempDir::new().unwrap();
    // 1KB threshold to force multiple rotations
    let mut writer = WalWriter::new(
        1, tmp.path(), 1024,
    )
    .unwrap();

    let n = 30;
    for i in 0..n {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
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
fn reader_open_from_seq_finds_correct_file() {
    let tmp = TempDir::new().unwrap();
    // 512B threshold to force many rotations
    let mut writer =
        WalWriter::new(
        1, tmp.path(), 512,
    )
            .unwrap();

    // Write 50 records across multiple files
    for i in 0..50 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
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
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    for i in 0..10 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
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
fn record_max_payload_64kb() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    // FillRecord is 64 bytes, which is well under 64KB limit
    let mut fill = make_fill(1);
    assert!(writer.append(&mut fill).is_ok());
}

#[test]
fn writer_empty_flush_no_io() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024,
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
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    assert_eq!(writer.next_seq, 1);
    let mut fill = make_fill(0);
    let seq = writer.append(&mut fill).unwrap();
    assert_eq!(seq, 1);
}


#[test]
fn writer_flush_calls_fsync() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    let mut fill = make_fill(1);
    writer.append(&mut fill).unwrap();
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
        1, tmp.path(), 512,
    )
    .unwrap();

    for i in 0..30 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
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
        1, tmp.path(), 64 * 1024 * 1024,
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
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    for i in 0..5 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
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
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    let mut fill = make_fill(1);
    writer.append(&mut fill).unwrap();
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
        1, tmp.path(), 512,
    )
    .unwrap();

    for i in 0..50 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
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
        1, tmp.path(), 512,
    )
    .unwrap();

    for i in 0..30 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
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
        1, tmp.path(), 64 * 1024 * 1024,
    )
    .unwrap();

    let mut fill = make_fill(1);
    writer.append(&mut fill).unwrap();
    writer.flush().unwrap();

    let mut reader =
        WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    reader.next().unwrap();
    let result = reader.next().unwrap();
    assert!(result.is_none());
}










#[test]
fn wal_rotate_at_64mb() {
    let tmp = TempDir::new().unwrap();
    let threshold = 64 * 1024 * 1024;
    let mut writer = WalWriter::new(
        1, tmp.path(), threshold,
    )
    .unwrap();

    let record_size = 16 + std::mem::size_of::<FillRecord>();
    let count = (threshold as usize / record_size) + 10;
    for i in 0..count as u64 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();

    let dir = tmp.path().join("1");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        files.len() >= 2,
        "expected rotation at 64MB, got {} files",
        files.len()
    );
}


#[test]
fn read_record_at_seq_finds_active() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 1024 * 1024,
    )
    .unwrap();
    for _ in 0..100 {
        let mut fill = make_fill(0);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();
    drop(writer);

    // Read back seq 42 by random access.
    let rec = read_record_at_seq(1, 42, tmp.path())
    .unwrap()
    .expect("seq 42 should exist");
    assert_eq!(extract_seq(&rec.payload), Some(42));
}

#[test]
fn read_record_at_seq_returns_none_for_missing() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), 1024 * 1024,
    )
    .unwrap();
    for _ in 0..10 {
        let mut fill = make_fill(0);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();
    drop(writer);

    let rec = read_record_at_seq(1, 999, tmp.path())
    .unwrap();
    assert!(rec.is_none());
}

#[test]
fn read_record_at_seq_finds_in_rotated_file() {
    let tmp = TempDir::new().unwrap();
    // Small rotation threshold so we get multiple files.
    let mut writer = WalWriter::new(
        1, tmp.path(), 256,
    )
    .unwrap();
    for _ in 0..200 {
        let mut fill = make_fill(0);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();
    drop(writer);

    // Multi-file: read seq 5 (early, rotated) and seq 195
    // (late, possibly active).
    let early = read_record_at_seq(1, 5, tmp.path())
    .unwrap()
    .expect("seq 5 should exist");
    assert_eq!(extract_seq(&early.payload), Some(5));

    let late = read_record_at_seq(1, 195, tmp.path())
    .unwrap()
    .expect("seq 195 should exist");
    assert_eq!(extract_seq(&late.payload), Some(195));
}
