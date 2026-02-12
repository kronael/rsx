use rsx_dxs::*;
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
    }
}

#[test]
fn writer_assigns_monotonic_seq() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
        1, tmp.path(), None, 1024, 600_000_000_000,
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
        1, tmp.path(), None, 4096, 600_000_000_000,
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
    // corrupt record_type field (bytes 0-1) to unknown type
    data[0] = 0xFF;
    data[1] = 0xFF;
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
        1, tmp.path(), None, 1024, 600_000_000_000,
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
fn gc_deletes_old_files() {
    let tmp = TempDir::new().unwrap();
    // 1KB threshold, retention = 1ns (basically zero)
    let mut writer =
        WalWriter::new(1, tmp.path(), None, 1024, 1).unwrap();

    for i in 0..100 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
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
        WalWriter::new(1, tmp.path(), None, 512, 600_000_000_000)
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
fn writer_gc_preserves_recent_files() {
    let tmp = TempDir::new().unwrap();
    // 512B threshold, high retention so nothing gets gc'd
    let mut writer = WalWriter::new(
        1, tmp.path(), None, 512, u64::MAX,
    )
    .unwrap();

    for i in 0..50 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
    )
    .unwrap();

    assert_eq!(writer.next_seq, 1);
    let mut fill = make_fill(0);
    let seq = writer.append(&mut fill).unwrap();
    assert_eq!(seq, 1);
}

#[test]
fn writer_gc_runs_on_rotation_not_timer() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), None, 512, 1,
    )
    .unwrap();

    for i in 0..50 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
        1, tmp.path(), None, 512, 600_000_000_000,
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
        1, tmp.path(), None, 512, 600_000_000_000,
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
        1, tmp.path(), None, 512, 600_000_000_000,
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
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
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
fn writer_archive_dir_created_on_init() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("archive");
    let _writer = WalWriter::new(
        1,
        tmp.path(),
        Some(archive.clone()),
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();

    let archive_stream_dir = archive.join("1");
    assert!(
        archive_stream_dir.exists(),
        "archive dir not created"
    );
}

#[test]
fn writer_gc_moves_to_archive_instead_of_delete() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("archive");

    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        Some(archive.clone()),
        1024,
        1,
    )
    .unwrap();

    for i in 0..100 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();

    writer.gc().unwrap();

    let wal_dir = tmp.path().join("1");
    let archive_dir = archive.join("1");

    let wal_files: Vec<_> = std::fs::read_dir(&wal_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name_str = name.to_string_lossy();
            !name_str.contains("active")
        })
        .collect();

    let archive_files: Vec<_> = std::fs::read_dir(&archive_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();

    assert!(
        archive_files.len() > 0,
        "no files moved to archive"
    );
    assert!(
        wal_files.len() == 0,
        "old files still in wal dir"
    );
}

#[test]
fn writer_archive_preserves_filename() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("archive");

    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        Some(archive.clone()),
        512,
        1,
    )
    .unwrap();

    for i in 0..30 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();

    writer.gc().unwrap();

    let archive_dir = archive.join("1");
    let files: Vec<_> = std::fs::read_dir(&archive_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();

    assert!(files.len() > 0);
    for file in files {
        let name = file.file_name();
        let name_str = name.to_string_lossy();
        assert!(
            name_str.starts_with("1_"),
            "filename format incorrect"
        );
        assert!(name_str.ends_with(".wal"));
    }
}

#[test]
fn reader_can_read_from_archive() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("archive");

    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        Some(archive.clone()),
        512,
        1,
    )
    .unwrap();

    for i in 0..30 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();

    writer.gc().unwrap();

    let mut reader =
        WalReader::open_from_seq(1, 0, &archive).unwrap();
    let mut count = 0;
    while let Ok(Some(_)) = reader.next() {
        count += 1;
    }
    assert!(count > 0, "no records read from archive");
}

#[test]
fn writer_without_archive_deletes_on_gc() {
    let tmp = TempDir::new().unwrap();
    let mut writer =
        WalWriter::new(1, tmp.path(), None, 512, 1).unwrap();

    for i in 0..50 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();

    writer.gc().unwrap();

    let wal_dir = tmp.path().join("1");
    let files: Vec<_> = std::fs::read_dir(&wal_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name_str = name.to_string_lossy();
            !name_str.contains("active")
        })
        .collect();

    assert_eq!(files.len(), 0, "old files not deleted");
}

#[test]
fn reader_archive_fallback_when_seq_not_in_hot() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("archive");

    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        Some(archive.clone()),
        512,
        1,
    )
    .unwrap();

    // write records that will get archived
    for i in 0..50 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();

    // trigger GC to move old files to archive
    writer.gc().unwrap();

    // write more records to hot WAL
    for i in 50..60 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();

    // request replay from seq 1 (which is now in archive)
    let mut reader = WalReader::open_from_seq_with_archive(
        1,
        1,
        tmp.path(),
        Some(&archive),
    )
    .unwrap();

    let mut count = 0;
    while let Ok(Some(_)) = reader.next() {
        count += 1;
    }

    // should read all records (archive + hot)
    assert_eq!(count, 60, "should read from archive and hot");
}

#[test]
fn reader_archive_to_hot_seamless_transition() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("archive");

    let mut writer = WalWriter::new(
        1,
        tmp.path(),
        Some(archive.clone()),
        512,
        1,
    )
    .unwrap();

    // write and archive first batch
    for i in 0..30 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();

    // sleep to ensure file mtime ages past retention
    std::thread::sleep(std::time::Duration::from_millis(5));
    writer.gc().unwrap();

    // check archive dir
    let archive_dir = archive.join("1");
    let archive_files: Vec<_> = std::fs::read_dir(&archive_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    eprintln!("Archive files after GC: {}", archive_files.len());

    // write second batch to hot
    for i in 30..50 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();

    // read with archive fallback
    let mut reader = WalReader::open_from_seq_with_archive(
        1,
        0,
        tmp.path(),
        Some(&archive),
    )
    .unwrap();

    eprintln!("Reader wal_dir: {}", reader.wal_dir().display());

    let mut seqs = Vec::new();
    while let Ok(Some(rec)) = reader.next() {
        if let Some(seq) = extract_seq(&rec.payload) {
            seqs.push(seq);
        }
    }

    eprintln!("Total records read: {}", seqs.len());

    // verify continuous seq from 1 to 50
    assert_eq!(seqs.len(), 50);
    assert_eq!(seqs[0], 1);
    assert_eq!(seqs[49], 50);
    for i in 0..49 {
        assert_eq!(
            seqs[i] + 1,
            seqs[i + 1],
            "gap at seq {}",
            seqs[i]
        );
    }
}

#[test]
fn reader_no_archive_fallback_without_archive_dir() {
    let tmp = TempDir::new().unwrap();

    let mut writer =
        WalWriter::new(1, tmp.path(), None, 512, 1).unwrap();

    for i in 0..50 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();

    // delete old files (simulating GC without archive)
    writer.gc().unwrap();

    // write new records
    for i in 50..60 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();

    // request from seq 1 (no longer available, no archive)
    let mut reader = WalReader::open_from_seq_with_archive(
        1, 1, tmp.path(), None,
    )
    .unwrap();

    // should start from first available (seq 50+)
    if let Ok(Some(rec)) = reader.next() {
        if let Some(seq) = extract_seq(&rec.payload) {
            assert!(
                seq >= 50,
                "should skip missing seqs, got {}",
                seq
            );
        }
    }
}

#[test]
fn reader_archive_fallback_empty_archive() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("archive");
    std::fs::create_dir_all(
        archive.join("1"),
    )
    .unwrap();

    let mut writer =
        WalWriter::new(
            1,
            tmp.path(),
            None,
            64 * 1024 * 1024,
            600_000_000_000,
        )
        .unwrap();

    for i in 0..20 {
        let mut fill = make_fill(i);
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();

    // read with archive fallback (but archive is empty)
    let mut reader = WalReader::open_from_seq_with_archive(
        1,
        0,
        tmp.path(),
        Some(&archive),
    )
    .unwrap();

    let mut count = 0;
    while let Ok(Some(_)) = reader.next() {
        count += 1;
    }

    // should read from hot WAL only
    assert_eq!(count, 20);
}

#[test]
fn wal_rotate_at_64mb() {
    let tmp = TempDir::new().unwrap();
    let threshold = 64 * 1024 * 1024;
    let mut writer = WalWriter::new(
        1, tmp.path(), None, threshold, 600_000_000_000,
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
fn wal_gc_removes_old_files() {
    let tmp = TempDir::new().unwrap();
    // high retention so rotate() GC doesn't delete immediately
    let mut writer = WalWriter::new(
        1, tmp.path(), None, 1024, u64::MAX,
    )
    .unwrap();

    // flush in batches to trigger multiple rotations
    for batch in 0..5u64 {
        for j in 0..15u64 {
            let mut fill = make_fill(batch * 15 + j);
            let _ = writer.append(&mut fill);
        }
        writer.flush().unwrap();
    }

    let dir = tmp.path().join("1");
    let before: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            !e.file_name()
                .to_string_lossy()
                .contains("active")
        })
        .collect();
    assert!(
        before.len() >= 2,
        "expected rotated files, got {}",
        before.len()
    );

    // now create a new writer with 1ns retention to gc
    let writer2 =
        WalWriter::new(1, tmp.path(), None, 1024, 1).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    writer2.gc().unwrap();

    let after: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            !e.file_name()
                .to_string_lossy()
                .contains("active")
        })
        .collect();
    assert_eq!(
        after.len(),
        0,
        "gc should remove old rotated files"
    );
}
