use rsx_cast::wal::WalReader;
use rsx_cast::wal::WalWriter;
use rsx_cast::CastRecord;
use rsx_messages::decode_config_applied_record;
use rsx_messages::ConfigAppliedRecord;
use rsx_messages::RECORD_CONFIG_APPLIED;
use rsx_types::time_utils::time_ns;
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
    let mut wal = WalWriter::new(1, &wal_dir, 64 * 1024 * 1024).expect("wal");

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

    {
        let framed = wal.prepare(&mut record).expect("prepare");
        wal.append_framed(&framed).expect("append");
    }
    wal.flush().expect("flush");

    // Verify active WAL file exists
    let active_wal = wal_dir.join("1").join("1_active.wal");
    assert!(active_wal.exists());
    let metadata = std::fs::metadata(&active_wal).unwrap();
    assert!(metadata.len() > 0);
}

#[test]
fn config_applied_record_type() {
    assert_eq!(
        ConfigAppliedRecord::record_type(),
        rsx_messages::RECORD_CONFIG_APPLIED
    );
}

#[test]
fn config_applied_emits_event() {
    let tmp = TempDir::new().unwrap();
    let mut wal = WalWriter::new(1, tmp.path(), 64 * 1024 * 1024).expect("wal");

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

    {
        let framed = wal.prepare(&mut record).expect("prepare");
        wal.append_framed(&framed).expect("append");
    }
    wal.flush().expect("flush");

    // Read back the record
    let mut reader = WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    let raw = reader.next().unwrap().unwrap();

    assert_eq!(raw.header.record_type, RECORD_CONFIG_APPLIED);
    let r = decode_config_applied_record(&raw.payload).expect("decode");
    assert_eq!(r.symbol_id, 1);
    assert_eq!(r.config_version, 1);
    assert_eq!(r.applied_at_ns, ts);
}

#[test]
fn config_version_monotonic() {
    let tmp = TempDir::new().unwrap();
    let mut wal = WalWriter::new(1, tmp.path(), 64 * 1024 * 1024).expect("wal");

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
        {
            let framed = wal.prepare(&mut record).expect("prepare");
            wal.append_framed(&framed).expect("append");
        }
    }
    wal.flush().expect("flush");

    // Verify versions are monotonic
    let mut reader = WalReader::open_from_seq(1, 0, tmp.path()).unwrap();
    let mut last_version = 0u64;
    while let Ok(Some(raw)) = reader.next() {
        if raw.header.record_type == RECORD_CONFIG_APPLIED {
            let r = decode_config_applied_record(&raw.payload).expect("decode");
            assert!(r.config_version > last_version);
            last_version = r.config_version;
        }
    }
    assert_eq!(last_version, 5);
}
