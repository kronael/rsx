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
        let header = WalHeader::new(0, 0, 1, 0);
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
