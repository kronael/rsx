use crate::prune_archive;
use crate::read_last_seq;
use crate::segment_date;
use crate::watchdog;
use crate::RecorderState;
use chrono::NaiveDate;
use rsx_cast::encode_utils::compute_crc32;
use rsx_cast::header::WalHeader;
use rsx_cast::RawWalRecord;
use rsx_health::LoadGauges;
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

fn touch(dir: &Path, name: &str) {
    fs::write(dir.join(name), b"x").expect("write segment");
}

fn day(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("parse date")
}

#[test]
fn parses_only_matching_names() {
    assert_eq!(segment_date("7_2026-07-05.wal", 7), Some(day("2026-07-05")));
    // Wrong stream id.
    assert_eq!(segment_date("9_2026-07-05.wal", 7), None);
    // Not a segment.
    assert_eq!(segment_date("tip.bin", 7), None);
    assert_eq!(segment_date("7_notadate.wal", 7), None);
}

#[test]
fn prune_drops_old_keeps_recent_and_active() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    let stream_id = 7u32;
    let today = day("2026-07-05");

    // cutoff = today - 3 = 2026-07-02; prune date < cutoff.
    touch(dir, "7_2026-06-30.wal"); // 5 days old -> prune
    touch(dir, "7_2026-07-01.wal"); // 4 days old -> prune (< cutoff)
    touch(dir, "7_2026-07-02.wal"); // == cutoff -> keep
    touch(dir, "7_2026-07-03.wal"); // within window -> keep
    touch(dir, "7_2026-07-05.wal"); // active -> keep
                                    // A foreign file the prune must never touch.
    touch(dir, "notes.txt");
    // A different stream's segment -> never touched.
    touch(dir, "9_2026-01-01.wal");

    prune_archive(dir, stream_id, today, 3);

    assert!(!dir.join("7_2026-06-30.wal").exists(), "old pruned");
    assert!(
        !dir.join("7_2026-07-01.wal").exists(),
        "below cutoff pruned"
    );
    assert!(dir.join("7_2026-07-02.wal").exists(), "cutoff kept");
    assert!(dir.join("7_2026-07-03.wal").exists(), "within window kept");
    assert!(dir.join("7_2026-07-05.wal").exists(), "active kept");
    assert!(dir.join("notes.txt").exists(), "foreign untouched");
    assert!(
        dir.join("9_2026-01-01.wal").exists(),
        "other stream untouched"
    );
}

fn record(seq: u64) -> RawWalRecord {
    let mut payload = vec![0u8; 16];
    payload[..8].copy_from_slice(&seq.to_le_bytes());
    let header = WalHeader::new(0xFE, 16, compute_crc32(&payload));
    RawWalRecord { header, payload }
}

fn archive_file(archive: &Path) -> std::path::PathBuf {
    let mut files: Vec<_> = fs::read_dir(archive.join("7"))
        .expect("read stream dir")
        .map(|entry| entry.expect("entry").path())
        .collect();
    assert_eq!(files.len(), 1);
    files.remove(0)
}

#[test]
fn a_replayed_seq_is_written_once() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut state = RecorderState::new(tmp.path(), 7, 3).expect("open archive");
    for seq in [1, 2, 3, 2, 3, 4] {
        state.write_record(&record(seq)).expect("write");
    }
    state.flush().expect("flush");
    let path = archive_file(tmp.path());
    assert_eq!(fs::metadata(&path).expect("stat").len(), 4 * 32);
    assert_eq!(read_last_seq(&path).expect("read"), Some(4));
}

#[test]
fn a_reopened_archive_skips_what_it_already_holds() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut state = RecorderState::new(tmp.path(), 7, 3).expect("open archive");
    for seq in 1..=3 {
        state.write_record(&record(seq)).expect("write");
    }
    state.flush().expect("flush");
    drop(state);

    let mut state = RecorderState::new(tmp.path(), 7, 3).expect("reopen archive");
    for seq in 2..=5 {
        state.write_record(&record(seq)).expect("write");
    }
    state.flush().expect("flush");
    let path = archive_file(tmp.path());
    assert_eq!(fs::metadata(&path).expect("stat").len(), 5 * 32);
    assert_eq!(read_last_seq(&path).expect("read"), Some(5));
}

#[test]
fn a_torn_tail_does_not_hide_the_last_whole_record() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut state = RecorderState::new(tmp.path(), 7, 3).expect("open archive");
    state.write_record(&record(1)).expect("write");
    state.write_record(&record(2)).expect("write");
    state.flush().expect("flush");
    let path = archive_file(tmp.path());
    let mut bytes = fs::read(&path).expect("read");
    bytes.extend_from_within(..20);
    fs::write(&path, bytes).expect("write torn tail");
    assert_eq!(read_last_seq(&path).expect("read"), Some(2));
}

const STALL: Duration = Duration::from_millis(200);

async fn settle() {
    tokio::time::sleep(STALL * 4).await;
}

#[tokio::test]
async fn an_idle_caught_up_stream_stays_healthy_and_a_dropped_link_is_not() {
    let gauges = LoadGauges::new();
    gauges.live.store(true, Ordering::Relaxed);
    gauges.ready.store(true, Ordering::Relaxed);
    let caught_up = Arc::new(AtomicBool::new(true));
    let task = tokio::spawn(watchdog(gauges.clone(), caught_up.clone(), STALL));

    settle().await;
    assert!(gauges.live.load(Ordering::Relaxed), "idle but caught up");
    assert!(gauges.ready.load(Ordering::Relaxed));

    caught_up.store(false, Ordering::Relaxed);
    settle().await;
    assert!(!gauges.live.load(Ordering::Relaxed), "link dropped");
    assert!(!gauges.ready.load(Ordering::Relaxed));

    caught_up.store(true, Ordering::Relaxed);
    settle().await;
    assert!(gauges.live.load(Ordering::Relaxed), "caught up again");
    assert!(gauges.ready.load(Ordering::Relaxed));
    task.abort();
}

#[tokio::test]
async fn a_recorder_that_never_catches_up_turns_unhealthy() {
    let gauges = LoadGauges::new();
    gauges.live.store(true, Ordering::Relaxed);
    gauges.ready.store(true, Ordering::Relaxed);
    let caught_up = Arc::new(AtomicBool::new(false));
    let task = tokio::spawn(watchdog(gauges.clone(), caught_up, STALL));

    settle().await;
    assert!(!gauges.live.load(Ordering::Relaxed), "behind the stream");
    task.abort();
}
