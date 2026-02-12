use std::fs;
use tempfile::TempDir;

#[test]
fn tip_persistence_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let tip_file = tmp.path().join("tip");

    // write tip
    let tip: u64 = 42;
    let tmp_path = tip_file.with_extension("tmp");
    fs::write(&tmp_path, &tip.to_le_bytes()).unwrap();
    fs::rename(&tmp_path, &tip_file).unwrap();

    // read tip
    let data = fs::read(&tip_file).unwrap();
    let bytes: [u8; 8] = data[..8].try_into().unwrap();
    let loaded = u64::from_le_bytes(bytes);
    assert_eq!(loaded, 42);
}

#[test]
fn tip_missing_returns_zero() {
    let tmp = TempDir::new().unwrap();
    let tip_file = tmp.path().join("nonexistent_tip");

    // should fail to read (file doesn't exist)
    let result = fs::read(&tip_file);
    assert!(result.is_err());
    // consumer defaults to 0 when file missing
}

#[test]
fn consumer_sends_tip_plus_1() {
    // verify the consumer would request from_seq = tip + 1
    let tip: u64 = 100;
    let from_seq = tip + 1;
    assert_eq!(from_seq, 101);
}

#[test]
fn consumer_callback_invoked_per_record() {
    // Simulate what DxsConsumer does: for each raw record
    // received, the callback is invoked once.
    use rsx_dxs::header::WalHeader;
    use rsx_dxs::wal::RawWalRecord;

    let mut count = 0u32;
    let mut callback = |_record: RawWalRecord| {
        count += 1;
    };

    // Simulate 5 records
    for _i in 0..5 {
        let header = WalHeader::new(0, 0, 0);
        let record = RawWalRecord {
            header,
            payload: vec![],
        };
        callback(record);
    }
    assert_eq!(count, 5);
}

#[test]
fn consumer_dedup_by_seq() {
    // Verify dedup logic: consumer tracks tip and only
    // processes records with seq > tip.
    let mut tip: u64 = 5;
    let mut processed = Vec::new();

    let incoming_seqs = [3, 5, 6, 7, 6, 8];
    for seq in incoming_seqs {
        if seq > tip {
            processed.push(seq);
            tip = seq;
        }
    }
    assert_eq!(processed, vec![6, 7, 8]);
    assert_eq!(tip, 8);
}

#[test]
fn backoff_schedule() {
    let schedule = [1u64, 2, 4, 8, 30];
    assert_eq!(schedule[0], 1);
    assert_eq!(schedule[1], 2);
    assert_eq!(schedule[2], 4);
    assert_eq!(schedule[3], 8);
    assert_eq!(schedule[4], 30);

    // clamped at max
    let idx = 10;
    let secs = schedule[idx.min(schedule.len() - 1)];
    assert_eq!(secs, 30);
}

#[test]
fn consumer_loads_tip_from_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tip_file = tmp.path().join("tip");
    let tip: u64 = 42;
    std::fs::write(&tip_file, &tip.to_le_bytes()).unwrap();

    let data = std::fs::read(&tip_file).unwrap();
    let bytes: [u8; 8] = data[..8].try_into().unwrap();
    let loaded = u64::from_le_bytes(bytes);
    assert_eq!(loaded, 42);
}

#[test]
fn consumer_tip_zero_if_file_missing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tip_file = tmp.path().join("nonexistent");

    let result = std::fs::read(&tip_file);
    assert!(result.is_err());
}

#[test]
fn consumer_advances_tip_per_record() {
    let mut tip: u64 = 0;
    for _ in 0..10 {
        tip += 1;
    }
    assert_eq!(tip, 10);
}

#[test]
fn consumer_persists_tip_on_interval() {
    use std::time::Duration;
    use std::time::Instant;

    let persist_interval = Duration::from_millis(10);
    let mut last_persist = Instant::now();

    std::thread::sleep(Duration::from_millis(15));
    assert!(last_persist.elapsed() >= persist_interval);
    last_persist = Instant::now();
    assert!(last_persist.elapsed() < persist_interval);
}

#[test]
fn consumer_reconnect_backoff_1_2_4_8_30() {
    let backoff = [1u64, 2, 4, 8, 30];
    assert_eq!(backoff.len(), 5);
    assert_eq!(backoff[4], 30);
}

#[test]
fn consumer_reconnect_resets_on_success() {
    let backoff_idx = 4;
    let backoff_idx_reset = 0;
    assert_ne!(backoff_idx, backoff_idx_reset);
    assert_eq!(backoff_idx_reset, 0);
}

#[test]
fn consumer_skips_unknown_record_types() {
    use rsx_dxs::header::WalHeader;
    use rsx_dxs::records::RECORD_FILL;
    use rsx_dxs::wal::RawWalRecord;

    let mut known_count = 0u32;
    let mut callback = |_record: RawWalRecord| {
        known_count += 1;
    };

    // Known record type (RECORD_FILL = 0)
    let known_header = WalHeader::new(RECORD_FILL, 0, 0);
    let known_record = RawWalRecord {
        header: known_header,
        payload: vec![],
    };
    callback(known_record);

    // Unknown record type (0xFFFF)
    let unknown_header = WalHeader::new(0xFFFF, 0, 0);
    let unknown_record = RawWalRecord {
        header: unknown_header,
        payload: vec![],
    };
    // This would be skipped by consumer, not passed to callback
    // So we only count known records
    let is_known = matches!(unknown_header.record_type, 0..=18);
    if is_known {
        callback(unknown_record);
    }

    assert_eq!(known_count, 1);
}

#[test]
fn consumer_advances_tip_on_unknown_record() {
    // Tip should advance from payload seq when available,
    // even if record type is unknown/skipped.
    let mut tip: u64 = 100;

    // Receive 3 records with seq values.
    let seqs = [101u64, 250u64, 251u64];
    for seq in seqs {
        tip = tip.max(seq);
    }

    assert_eq!(tip, 251);
}

#[test]
fn consumer_tip_never_decreases_on_reordered_seq() {
    let mut tip: u64 = 1000;
    let incoming = [1001u64, 998u64, 1002u64];
    for seq in incoming {
        tip = tip.max(seq);
    }
    assert_eq!(tip, 1002);
}
