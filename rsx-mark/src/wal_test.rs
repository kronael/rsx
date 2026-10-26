//! A restarted mark continues the WAL's seq numbering instead of
//! restarting at 1 after the records already on disk.

use crate::types::MarkPriceEvent;
use crate::wal::open_wal_writer;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::WalReader;
use rsx_cast::wal::WalWriter;
use rsx_types::Price;
use std::path::Path;

const STREAM_ID: u32 = 7;
const ROTATE_BYTES: u64 = 64 * 1024 * 1024;

fn event(symbol_id: u32) -> MarkPriceEvent {
    MarkPriceEvent {
        seq: 0,
        ts_ns: 1,
        symbol_id,
        _pad0: 0,
        mark_price: Price(50_000),
        source_mask: 1,
        source_count: 1,
        _pad1: [0; 24],
    }
}

fn append(writer: &mut WalWriter, count: usize) {
    for _ in 0..count {
        let framed = writer.prepare(&mut event(1)).unwrap();
        writer.append_framed(&framed).unwrap();
    }
    writer.flush().unwrap();
}

fn seqs_on_disk(wal_dir: &Path) -> Vec<u64> {
    let mut reader = WalReader::open_from_seq(STREAM_ID, 1, wal_dir).unwrap();
    let mut seqs = Vec::new();
    while let Some(record) = reader.next().unwrap() {
        seqs.push(extract_seq(&record.payload).unwrap());
    }
    seqs
}

#[test]
fn empty_wal_starts_at_seq_1() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = open_wal_writer(STREAM_ID, dir.path(), ROTATE_BYTES).unwrap();
    append(&mut writer, 3);
    assert_eq!(seqs_on_disk(dir.path()), vec![1, 2, 3]);
}

#[test]
fn restart_continues_after_the_last_seq_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = open_wal_writer(STREAM_ID, dir.path(), ROTATE_BYTES).unwrap();
    append(&mut first, 5);
    drop(first);

    let mut second = open_wal_writer(STREAM_ID, dir.path(), ROTATE_BYTES).unwrap();
    append(&mut second, 3);
    drop(second);

    assert_eq!(seqs_on_disk(dir.path()), vec![1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn restart_after_rotation_continues_after_the_rotated_last_seq() {
    let dir = tempfile::tempdir().unwrap();
    // A tiny rotate size rotates after every flush, leaving the active
    // file empty.
    let mut first = open_wal_writer(STREAM_ID, dir.path(), 1).unwrap();
    append(&mut first, 4);
    drop(first);

    let mut second = open_wal_writer(STREAM_ID, dir.path(), ROTATE_BYTES).unwrap();
    append(&mut second, 2);
    drop(second);

    assert_eq!(seqs_on_disk(dir.path()), vec![1, 2, 3, 4, 5, 6]);
}
