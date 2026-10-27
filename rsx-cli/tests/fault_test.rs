mod common;

use common::make_test_dir;
use common::run_cli;
use common::spawn_recorded;
use common::write_fixture_wal;
use common::FIXTURE_STREAM;
use serde_json::Value;
use std::fs;
use std::process::Output;

fn stdout_json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|err| panic!("{err}: {}", String::from_utf8_lossy(&out.stdout)))
}

fn stderr_error(out: &Output) -> String {
    let line = String::from_utf8_lossy(&out.stderr);
    let value: Value =
        serde_json::from_str(line.trim()).unwrap_or_else(|err| panic!("{err}: {line}"));
    value["error"].as_str().unwrap().to_string()
}

#[test]
fn test_wal_corrupt_makes_wal_verify_report_the_stream_corrupt() {
    let dir = make_test_dir("fault_wal_corrupt");
    let wal_dir = dir.to_str().unwrap();
    write_fixture_wal(&dir);
    let clean = run_cli(&["wal", "verify", wal_dir, "--json"]);
    assert!(clean.status.success());

    let out = run_cli(&["fault", "wal-corrupt", "1", "--wal-dir", wal_dir, "--json"]);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let verify = run_cli(&["wal", "verify", wal_dir, "--json"]);
    assert_eq!(verify.status.code(), Some(1));
    let report = stdout_json(&verify);
    assert_eq!(report["status"], "corrupt");
    assert_eq!(report["streams"][0]["stream_id"], FIXTURE_STREAM);
    assert_eq!(report["streams"][0]["crc_errors"], 1);
}

#[test]
fn test_wal_corrupt_defaults_to_the_tmp_wal_of_the_root() {
    let root = make_test_dir("fault_wal_corrupt_root");
    write_fixture_wal(&root.join("tmp/wal"));

    let out = run_cli(&["fault", "wal-corrupt", "1", "-C", root.to_str().unwrap()]);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let verify = run_cli(&["wal", "verify", root.join("tmp/wal").to_str().unwrap()]);
    assert_eq!(verify.status.code(), Some(1));
}

#[test]
fn test_wal_corrupt_refuses_an_offset_outside_a_payload_and_changes_nothing() {
    let dir = make_test_dir("fault_wal_corrupt_offset");
    let segment = write_fixture_wal(&dir);
    let before = fs::read(&segment).unwrap();

    let out = run_cli(&[
        "fault",
        "wal-corrupt",
        "1",
        "--wal-dir",
        dir.to_str().unwrap(),
        "--offset",
        "3",
        "--json",
    ]);

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(stderr_error(&out).contains("not inside the payload"));
    assert_eq!(fs::read(&segment).unwrap(), before);
}

#[test]
fn test_wal_corrupt_of_a_missing_stream_is_an_error() {
    let dir = make_test_dir("fault_wal_corrupt_missing");
    write_fixture_wal(&dir);

    let out = run_cli(&[
        "fault",
        "wal-corrupt",
        "9",
        "--wal-dir",
        dir.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(out.status.code(), Some(1));
    assert!(stderr_error(&out).contains("no segment"));
}

#[test]
fn test_kill_stops_the_recorded_process_only() {
    let root = make_test_dir("fault_kill");
    let mut recorded = spawn_recorded(&root, "proc-a");
    let mut stranger = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();

    let out = run_cli(&["fault", "kill", "proc-a", "-C", root.to_str().unwrap()]);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let status = recorded.wait().unwrap();
    assert_eq!(status.code(), None, "ended by a signal");
    assert!(stranger.try_wait().unwrap().is_none());
    stranger.kill().unwrap();
    stranger.wait().unwrap();

    let again = run_cli(&[
        "fault",
        "kill",
        "proc-a",
        "-C",
        root.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(again.status.code(), Some(1));
    assert!(stderr_error(&again).contains("not running"));
}

#[test]
fn test_kill_refuses_a_name_the_cluster_did_not_start() {
    let root = make_test_dir("fault_kill_stranger");
    let mut stranger = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();

    let out = run_cli(&[
        "fault",
        "kill",
        "sleep",
        "-C",
        root.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(stderr_error(&out).contains("no record"));
    assert!(stranger.try_wait().unwrap().is_none());
    stranger.kill().unwrap();
    stranger.wait().unwrap();
}

#[test]
fn test_net_rejects_bad_arguments_before_running_tc() {
    for args in [
        &["--iface", "a b"][..],
        &["--iface=-h"],
        &["--loss-pct", "101"],
        &["--delay-ms", "0", "--loss-pct", "0"],
    ] {
        let mut full = vec!["fault", "net", "--json"];
        full.extend_from_slice(args);
        let out = run_cli(&full);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        stderr_error(&out);
    }
    let conflict = run_cli(&["fault", "net", "--clear", "--delay-ms", "5"]);
    assert_eq!(conflict.status.code(), Some(2));
}

#[test]
fn test_dup_order_without_the_matching_binary_is_an_error() {
    let root = make_test_dir("fault_dup_order_nobinary");

    let out = run_cli(&["fault", "dup-order", "-C", root.to_str().unwrap(), "--json"]);

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(stderr_error(&out).contains("binary not found"));
}
