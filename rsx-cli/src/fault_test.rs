use crate::fault::check_iface;
use crate::fault::check_shaping;
use crate::fault::do_kill;
use crate::fault::flip_byte;
use crate::fault::net_json;
use crate::fault::tc_args;
use crate::fault::Shaping;
use crate::fault::DEFAULT_CORRUPT_OFFSET;
use crate::inspect::verify_stream;
use crate::output::Failure;
use crate::plan::Process;
use crate::procs::spawn;
use crate::procs::Record;
use crate::procs::Store;
use rsx_cast::encode_utils::encode_record;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use tempfile::tempdir;

const RECORD_TYPE: u16 = 0xFE;

fn frames(seqs: &[u64]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for seq in seqs {
        let mut payload = vec![0u8; 16];
        payload[..8].copy_from_slice(&seq.to_le_bytes());
        payload[8..16].copy_from_slice(&(seq * 1000).to_le_bytes());
        bytes.extend(encode_record(RECORD_TYPE, &payload));
    }
    bytes
}

fn write_segment(wal_dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let stream_dir = wal_dir.join("1");
    fs::create_dir_all(&stream_dir).unwrap();
    let path = stream_dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn sleeper(name: &str) -> Process {
    Process {
        name: name.to_string(),
        binary: "/bin/sh".to_string(),
        args: vec!["-c".to_string(), "sleep 60".to_string()],
        cwd: ".".to_string(),
        env: BTreeMap::new(),
        bound: Vec::new(),
    }
}

#[test]
fn default_flip_makes_one_crc_error_that_verify_reports_as_corrupt() {
    let dir = tempdir().unwrap();
    let original = frames(&[1, 2, 3]);
    let path = write_segment(dir.path(), "1_active.wal", &original);

    let flip = flip_byte(dir.path(), 1, DEFAULT_CORRUPT_OFFSET).unwrap();

    assert_eq!(flip.file, path);
    assert_eq!(flip.offset, 24);
    assert_eq!(flip.old_byte, original[24]);
    assert_eq!(flip.new_byte, !original[24]);
    let changed: Vec<usize> = fs::read(&path)
        .unwrap()
        .iter()
        .zip(&original)
        .enumerate()
        .filter(|(_, (after, before))| after != before)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(changed, [24]);
    let verdict = verify_stream(1, dir.path()).unwrap();
    assert_eq!(verdict.crc_errors, 1);
    assert_eq!(verdict.records, 2);
    assert!(verdict.is_corrupt());
}

#[test]
fn only_the_newest_segment_is_touched() {
    let dir = tempdir().unwrap();
    let older = frames(&[1, 2]);
    let older_path = write_segment(dir.path(), "1_1_2.wal", &older);
    let newest_path = write_segment(dir.path(), "1_active.wal", &frames(&[3, 4]));

    let flip = flip_byte(dir.path(), 1, DEFAULT_CORRUPT_OFFSET).unwrap();

    assert_eq!(flip.file, newest_path);
    assert_eq!(fs::read(older_path).unwrap(), older);
}

#[test]
fn a_later_record_can_be_the_target() {
    let dir = tempdir().unwrap();
    write_segment(dir.path(), "1_active.wal", &frames(&[1, 2, 3]));

    flip_byte(dir.path(), 1, 32 + 16 + 3).unwrap();

    let verdict = verify_stream(1, dir.path()).unwrap();
    assert_eq!(verdict.crc_errors, 1);
    assert_eq!(verdict.records, 2);
}

#[test]
fn an_offset_outside_a_record_payload_is_refused_and_changes_nothing() {
    let dir = tempdir().unwrap();
    let original = frames(&[1, 2]);
    let path = write_segment(dir.path(), "1_active.wal", &original);

    for offset in [0, 3, 15, 32, 40, 64, 1000] {
        let error = flip_byte(dir.path(), 1, offset).unwrap_err();
        assert!(
            error.message().contains("not inside the payload"),
            "offset {offset}: {}",
            error.message()
        );
        assert_eq!(fs::read(&path).unwrap(), original, "offset {offset}");
    }
}

#[test]
fn a_cut_record_cannot_be_the_target() {
    let dir = tempdir().unwrap();
    let mut bytes = frames(&[1]);
    bytes.extend(&frames(&[2])[..20]);
    write_segment(dir.path(), "1_active.wal", &bytes);

    assert!(flip_byte(dir.path(), 1, 32 + 16 + 2).is_err());
}

#[test]
fn a_stream_without_segments_is_a_runtime_failure() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("1")).unwrap();

    let error = flip_byte(dir.path(), 1, DEFAULT_CORRUPT_OFFSET).unwrap_err();
    assert!(matches!(error, Failure::Runtime(_)));
    assert!(
        error.message().contains("no segment"),
        "{}",
        error.message()
    );
    assert!(flip_byte(dir.path(), 7, DEFAULT_CORRUPT_OFFSET).is_err());
}

#[test]
fn kill_signals_only_the_recorded_process_and_leaves_the_record() {
    let root = tempdir().unwrap();
    let store = Store::new(root.path());
    let (mut child, record) = spawn(root.path(), &sleeper("proc-a")).unwrap();
    store.save(&record).unwrap();
    let mut stranger = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();

    let killed = do_kill(root.path(), "proc-a").unwrap();

    assert_eq!(killed.pid, record.pid);
    child.wait().unwrap();
    assert!(!record.is_running());
    assert_eq!(store.load("proc-a").unwrap(), Some(record));
    assert!(stranger.try_wait().unwrap().is_none());
    stranger.kill().unwrap();
    stranger.wait().unwrap();
}

#[test]
fn kill_of_a_name_without_a_record_signals_nothing() {
    let root = tempdir().unwrap();
    let mut stranger = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();

    let error = do_kill(root.path(), "sleep").unwrap_err();

    assert!(error.message().contains("no record"), "{}", error.message());
    assert!(stranger.try_wait().unwrap().is_none());
    stranger.kill().unwrap();
    stranger.wait().unwrap();
}

#[test]
fn kill_of_a_recycled_pid_signals_nothing() {
    let root = tempdir().unwrap();
    let store = Store::new(root.path());
    let mut stranger = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    store
        .save(&Record {
            name: "proc-b".to_string(),
            pid: stranger.id(),
            start_ticks: 1,
            started_ms: 1,
            log: "log/proc-b.log".to_string(),
        })
        .unwrap();

    let error = do_kill(root.path(), "proc-b").unwrap_err();

    assert!(
        error.message().contains("not running"),
        "{}",
        error.message()
    );
    assert!(stranger.try_wait().unwrap().is_none());
    stranger.kill().unwrap();
    stranger.wait().unwrap();
}

#[test]
fn kill_of_an_exited_process_is_a_runtime_failure() {
    let root = tempdir().unwrap();
    let store = Store::new(root.path());
    let (mut child, record) = spawn(root.path(), &sleeper("proc-c")).unwrap();
    store.save(&record).unwrap();
    child.kill().unwrap();
    child.wait().unwrap();

    let error = do_kill(root.path(), "proc-c").unwrap_err();

    assert!(matches!(error, Failure::Runtime(_)));
    assert!(
        error.message().contains("not running"),
        "{}",
        error.message()
    );
}

#[test]
fn tc_args_add_and_delete_the_root_discipline() {
    let shaping = Shaping {
        delay_ms: 50,
        loss_pct: 10.0,
    };
    assert_eq!(
        tc_args("lo", Some(&shaping)).join(" "),
        "qdisc add dev lo root netem delay 50ms loss 10%"
    );
    assert_eq!(tc_args("eth0", None).join(" "), "qdisc del dev eth0 root");
    let fractional = Shaping {
        delay_ms: 0,
        loss_pct: 0.5,
    };
    assert_eq!(
        tc_args("lo", Some(&fractional)).join(" "),
        "qdisc add dev lo root netem delay 0ms loss 0.5%"
    );
}

#[test]
fn interface_names_are_checked_before_they_reach_tc() {
    for name in ["lo", "eth0", "br-1", "veth_a.2"] {
        assert!(check_iface(name).is_ok(), "{name}");
    }
    for name in ["", "-h", "a b", "a;b", "a/b", "0123456789abcdef"] {
        assert!(
            matches!(check_iface(name), Err(Failure::Misuse(_))),
            "{name:?}"
        );
    }
}

#[test]
fn shaping_must_be_a_real_fault() {
    let shape = |delay_ms, loss_pct| check_shaping(&Shaping { delay_ms, loss_pct });
    assert!(shape(50, 10.0).is_ok());
    assert!(shape(0, 0.1).is_ok());
    assert!(shape(10, 100.0).is_ok());
    assert!(matches!(shape(0, 0.0), Err(Failure::Misuse(_))));
    assert!(matches!(shape(10, 100.5), Err(Failure::Misuse(_))));
    assert!(matches!(shape(10, -1.0), Err(Failure::Misuse(_))));
    assert!(matches!(shape(10, f64::NAN), Err(Failure::Misuse(_))));
}

#[test]
fn net_json_carries_the_shaping_or_nulls_for_a_clear() {
    let shaping = Shaping {
        delay_ms: 50,
        loss_pct: 10.0,
    };
    assert_eq!(
        net_json("lo", Some(shaping)),
        serde_json::json!({
            "iface": "lo", "delay_ms": 50, "loss_pct": 10.0, "active": true
        })
    );
    assert_eq!(
        net_json("lo", None),
        serde_json::json!({
            "iface": "lo", "delay_ms": null, "loss_pct": null, "active": false
        })
    );
}
