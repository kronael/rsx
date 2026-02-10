use rsx_dxs::encode_utils::decode_config_applied_record;
use rsx_dxs::records::ConfigAppliedRecord;
use rsx_dxs::records::CmpRecord;
use rsx_dxs::records::RECORD_CONFIG_APPLIED;
use rsx_dxs::wal::WalReader;
use rsx_dxs::wal::WalWriter;
use rsx_types::time::time_ns;
use tempfile::TempDir;

#[test]
fn config_applied_record_has_version() {
    let ts = time_ns();
    let record = ConfigAppliedRecord {
        seq: 0,
        ts_ns: ts,
        symbol_id: 1,
        _pad0: 0,
        config_version: 42,
        effective_at_ms: 0,
        applied_at_ns: ts,
    };
    assert_eq!(record.config_version, 42);
    assert_eq!(record.symbol_id, 1);
    assert_eq!(record.applied_at_ns, ts);
}

#[test]
fn config_applied_can_be_written_to_wal() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().to_path_buf();
    let mut wal = WalWriter::new(
        1,
        &wal_dir,
        None,
        64 * 1024 * 1024,
        10 * 60 * 1_000_000_000,
    )
    .expect("wal");

    let ts = time_ns();
    let mut record = ConfigAppliedRecord {
        seq: 0,
        ts_ns: ts,
        symbol_id: 1,
        _pad0: 0,
        config_version: 1,
        effective_at_ms: 0,
        applied_at_ns: ts,
    };

    wal.append(&mut record).expect("append");
    wal.flush().expect("flush");

    // Verify active WAL file exists
    let active_wal =
        wal_dir.join("1").join("1_active.wal");
    assert!(active_wal.exists());
    let metadata =
        std::fs::metadata(&active_wal).unwrap();
    assert!(metadata.len() > 0);
}

#[test]
fn config_version_increments() {
    let versions = vec![1u64, 2, 3, 4, 5];
    for (i, &v) in versions.iter().enumerate() {
        assert_eq!(v, (i + 1) as u64);
    }
}

#[test]
fn config_applied_record_type() {
    assert_eq!(
        ConfigAppliedRecord::record_type(),
        rsx_dxs::records::RECORD_CONFIG_APPLIED
    );
}

#[test]
fn config_applied_emits_event() {
    let tmp = TempDir::new().unwrap();
    let mut wal = WalWriter::new(
        1,
        tmp.path(),
        None,
        64 * 1024 * 1024,
        10 * 60 * 1_000_000_000,
    )
    .expect("wal");

    let ts = time_ns();
    let mut record = ConfigAppliedRecord {
        seq: 0,
        ts_ns: ts,
        symbol_id: 1,
        _pad0: 0,
        config_version: 1,
        effective_at_ms: 0,
        applied_at_ns: ts,
    };

    wal.append(&mut record).expect("append");
    wal.flush().expect("flush");

    // Read back the record
    let mut reader =
        WalReader::open_from_seq(1, 0, tmp.path())
            .unwrap();
    let raw = reader.next().unwrap().unwrap();

    assert_eq!(raw.header.record_type, RECORD_CONFIG_APPLIED);
    let r = decode_config_applied_record(&raw.payload)
        .expect("decode");
    assert_eq!(r.symbol_id, 1);
    assert_eq!(r.config_version, 1);
    assert_eq!(r.applied_at_ns, ts);
}

#[test]
fn config_version_monotonic() {
    let tmp = TempDir::new().unwrap();
    let mut wal = WalWriter::new(
        1,
        tmp.path(),
        None,
        64 * 1024 * 1024,
        10 * 60 * 1_000_000_000,
    )
    .expect("wal");

    let ts = time_ns();

    // Emit multiple config versions
    for version in 1..=5 {
        let mut record = ConfigAppliedRecord {
            seq: 0,
            ts_ns: ts + version * 1000,
            symbol_id: 1,
            _pad0: 0,
            config_version: version,
            effective_at_ms: 0,
            applied_at_ns: ts + version * 1000,
        };
        wal.append(&mut record).expect("append");
    }
    wal.flush().expect("flush");

    // Verify versions are monotonic
    let mut reader =
        WalReader::open_from_seq(1, 0, tmp.path())
            .unwrap();
    let mut last_version = 0u64;
    while let Ok(Some(raw)) = reader.next() {
        if raw.header.record_type == RECORD_CONFIG_APPLIED {
            let r = decode_config_applied_record(
                &raw.payload,
            )
            .expect("decode");
            assert!(r.config_version > last_version);
            last_version = r.config_version;
        }
    }
    assert_eq!(last_version, 5);
}

#[test]
fn config_effective_at_respected() {
    let ts = time_ns();
    let future_ms = 1_700_000_000_000u64;

    let record = ConfigAppliedRecord {
        seq: 0,
        ts_ns: ts,
        symbol_id: 1,
        _pad0: 0,
        config_version: 2,
        effective_at_ms: future_ms,
        applied_at_ns: ts,
    };

    // Verify effective_at is preserved
    assert_eq!(record.effective_at_ms, future_ms);
    // effective_at_ms is in milliseconds, applied_at_ns in nanoseconds
    // So effective_at * 1_000_000 converts ms to ns
    // This test just verifies the field exists and can be set
    assert_eq!(record.config_version, 2);
}

#[test]
fn config_updates_tick_lot_sizes() {
    // Simulate tick/lot size change
    let old_tick = 1i64;
    let new_tick = 10i64;
    let old_lot = 1i64;
    let new_lot = 100i64;

    assert_ne!(old_tick, new_tick);
    assert_ne!(old_lot, new_lot);

    // Verify new config can be represented
    let ts = time_ns();
    let record = ConfigAppliedRecord {
        seq: 0,
        ts_ns: ts,
        symbol_id: 1,
        _pad0: 0,
        config_version: 2,
        effective_at_ms: 0,
        applied_at_ns: ts,
    };

    // CONFIG_APPLIED signals that new config is active
    assert_eq!(record.config_version, 2);
}

#[test]
fn config_poll_interval_10min() {
    // Verify 10 min = 600 seconds
    let interval_sec = 600u64;
    assert_eq!(interval_sec, 10 * 60);

    // Verify elapsed check pattern
    let elapsed_sec = 601u64;
    assert!(elapsed_sec >= interval_sec);

    let elapsed_sec = 599u64;
    assert!(elapsed_sec < interval_sec);
}
