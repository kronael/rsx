use crate::prune_archive;
use crate::segment_date;
use chrono::NaiveDate;
use std::fs;
use std::path::Path;

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
