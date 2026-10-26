mod common;

use common::corrupt_record;
use common::fixture_utils::Wal;
use common::fixture_utils::BUY;
use common::fixture_utils::SYMBOL;
use common::make_test_dir;
use common::run_cli;
use common::write_fixture_wal;
use common::FIXTURE_RECORDS;
use serde_json::Value;
use std::fs;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::Output;

fn keys(value: &Value) -> Vec<&str> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

fn stdout_json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!("{}: {}", err, String::from_utf8_lossy(&out.stdout));
    })
}

fn fixture(name: &str) -> (PathBuf, PathBuf) {
    let dir = make_test_dir(name);
    let segment = write_fixture_wal(&dir);
    (dir, segment)
}

const STREAM_VERDICT_KEYS: [&str; 8] = [
    "crc_errors",
    "error",
    "first_seq",
    "last_seq",
    "records",
    "seq_gaps",
    "stream_id",
    "torn_tail",
];

#[test]
fn test_ls_json_contract() {
    let (dir, segment) = fixture("contract_ls");
    let out = run_cli(&["wal", "ls", dir.to_str().unwrap(), "--json"]);
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    let report = stdout_json(&out);
    assert_eq!(keys(&report), ["streams", "wal_dir"]);
    assert_eq!(report["wal_dir"], dir.to_str().unwrap());

    let streams = report["streams"].as_array().unwrap();
    assert_eq!(streams.len(), 1);
    let stream = &streams[0];
    assert_eq!(
        keys(stream),
        [
            "bytes",
            "files",
            "first_seq",
            "last_seq",
            "last_ts_ns",
            "segments",
            "stream_id"
        ]
    );
    assert_eq!(stream["stream_id"], 1);
    assert_eq!(stream["files"], 1);
    assert_eq!(stream["bytes"], fs::metadata(&segment).unwrap().len());
    let segments = stream["segments"].as_array().unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(
        keys(&segments[0]),
        ["bytes", "is_active", "modified_ms", "name"]
    );
    assert_eq!(segments[0]["name"], "1_1_15.wal");
    assert_eq!(segments[0]["bytes"], stream["bytes"]);
    assert_eq!(segments[0]["is_active"], false);
    assert!(segments[0]["modified_ms"].as_u64().unwrap() > 0);
    assert_eq!(stream["first_seq"], 1);
    assert_eq!(stream["last_seq"], FIXTURE_RECORDS);
    assert_eq!(stream["last_ts_ns"], 15_000);
}

#[test]
fn test_ls_no_seq_keeps_the_keys_and_nulls_the_seqs() {
    let (dir, segment) = fixture("contract_ls_no_seq");
    let out = run_cli(&["wal", "ls", dir.to_str().unwrap(), "--json", "--no-seq"]);
    assert!(out.status.success());
    let report = stdout_json(&out);
    let stream = &report["streams"][0];
    assert_eq!(
        keys(stream),
        [
            "bytes",
            "files",
            "first_seq",
            "last_seq",
            "last_ts_ns",
            "segments",
            "stream_id"
        ]
    );
    assert_eq!(stream["bytes"], fs::metadata(&segment).unwrap().len());
    assert_eq!(stream["segments"].as_array().unwrap().len(), 1);
    assert!(stream["first_seq"].is_null());
    assert!(stream["last_seq"].is_null());
    assert!(stream["last_ts_ns"].is_null());
}

#[test]
fn test_ls_json_of_an_empty_stream_has_null_seqs() {
    let dir = make_test_dir("contract_ls_empty");
    fs::create_dir_all(dir.join("4")).unwrap();
    let out = run_cli(&["wal", "ls", dir.to_str().unwrap(), "--json"]);
    let report = stdout_json(&out);
    let stream = &report["streams"][0];
    assert_eq!(stream["stream_id"], 4);
    assert_eq!(stream["files"], 0);
    assert!(stream["first_seq"].is_null());
    assert!(stream["last_seq"].is_null());
    assert!(stream["last_ts_ns"].is_null());
}

#[test]
fn test_ls_text_has_a_header_and_a_row() {
    let (dir, _) = fixture("contract_ls_text");
    let out = run_cli(&["wal", "ls", dir.to_str().unwrap()]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("stream"));
    assert!(lines[1].starts_with("1 "));
}

#[test]
fn test_verify_json_contract_on_a_clean_wal() {
    let (dir, _) = fixture("contract_verify_ok");
    let out = run_cli(&["wal", "verify", dir.to_str().unwrap(), "--json"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty());
    let report = stdout_json(&out);
    assert_eq!(keys(&report), ["status", "streams"]);
    assert_eq!(report["status"], "ok");
    let stream = &report["streams"][0];
    assert_eq!(keys(stream), STREAM_VERDICT_KEYS);
    assert_eq!(stream["stream_id"], 1);
    assert_eq!(stream["records"], FIXTURE_RECORDS);
    assert_eq!(stream["first_seq"], 1);
    assert_eq!(stream["last_seq"], FIXTURE_RECORDS);
    assert_eq!(stream["crc_errors"], 0);
    assert_eq!(stream["seq_gaps"], serde_json::json!([]));
    assert_eq!(stream["torn_tail"], false);
    assert!(stream["error"].is_null());
}

#[test]
fn test_verify_flipped_byte_is_corrupt_exit_1() {
    let (dir, segment) = fixture("contract_verify_corrupt");
    corrupt_record(&segment, 4);
    let out = run_cli(&["wal", "verify", dir.to_str().unwrap(), "--json"]);
    assert_eq!(out.status.code(), Some(1));

    let report = stdout_json(&out);
    assert_eq!(keys(&report), ["status", "streams"]);
    assert_eq!(report["status"], "corrupt");
    let stream = &report["streams"][0];
    assert_eq!(keys(stream), STREAM_VERDICT_KEYS);
    assert_eq!(stream["crc_errors"], 1);
    assert_eq!(stream["records"], FIXTURE_RECORDS - 1);
    assert_eq!(stream["seq_gaps"], serde_json::json!([]));

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(stderr.lines().count(), 1);
    let error: Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(keys(&error), ["error"]);
    assert_eq!(error["error"], "wal corrupt: 1 of 1 streams");
}

#[test]
fn test_verify_reports_a_flipped_byte_in_the_last_record() {
    let (dir, segment) = fixture("contract_verify_last");
    corrupt_record(&segment, (FIXTURE_RECORDS - 1) as usize);
    let out = run_cli(&["wal", "verify", dir.to_str().unwrap(), "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let report = stdout_json(&out);
    assert_eq!(report["status"], "corrupt");
    assert_eq!(report["streams"][0]["crc_errors"], 1);
}

#[test]
fn test_verify_cut_record_at_the_tail_is_a_torn_tail_not_corruption() {
    let (dir, segment) = fixture("contract_verify_torn");
    let file = OpenOptions::new().write(true).open(&segment).unwrap();
    file.set_len(fs::metadata(&segment).unwrap().len() - 5)
        .unwrap();
    let out = run_cli(&["wal", "verify", dir.to_str().unwrap(), "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let report = stdout_json(&out);
    assert_eq!(report["status"], "ok");
    let stream = &report["streams"][0];
    assert_eq!(stream["torn_tail"], true);
    assert_eq!(stream["records"], FIXTURE_RECORDS - 1);
}

#[test]
fn test_verify_text_report() {
    let (dir, segment) = fixture("contract_verify_text");
    let out = run_cli(&["wal", "verify", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "stream 1: ok records=15 seq=1..15 crc_errors=0 torn_tail=false\nstatus: ok\n"
    );

    corrupt_record(&segment, 4);
    let out = run_cli(&["wal", "verify", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "stream 1: corrupt records=14 seq=1..15 crc_errors=1 torn_tail=false\nstatus: corrupt\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "Error: wal corrupt: 1 of 1 streams\n"
    );
}

#[test]
fn test_stats_json_contract() {
    let (dir, _) = fixture("contract_stats");
    let out = run_cli(&["wal", "stats", "1", dir.to_str().unwrap(), "--json"]);
    assert!(out.status.success());
    let report = stdout_json(&out);
    assert_eq!(keys(&report), ["by_type", "records", "stream_id"]);
    assert_eq!(report["stream_id"], 1);
    assert_eq!(report["records"], FIXTURE_RECORDS);
    let by_type = report["by_type"].as_object().unwrap();
    assert_eq!(by_type.len(), 15);
    assert_eq!(by_type["FILL"], 1);
    assert_eq!(by_type["UNKNOWN"], 1);
    assert_eq!(
        by_type.values().map(|n| n.as_u64().unwrap()).sum::<u64>(),
        15
    );
}

#[test]
fn test_stats_json_respects_filters() {
    let (dir, _) = fixture("contract_stats_filter");
    let out = run_cli(&[
        "wal",
        "stats",
        "1",
        dir.to_str().unwrap(),
        "--symbol",
        "7",
        "--json",
    ]);
    let report = stdout_json(&out);
    assert_eq!(report["records"], 7);
    assert_eq!(report["by_type"]["FILL"], 1);
}

#[test]
fn test_dump_json_lines_carry_the_common_keys() {
    let (dir, _) = fixture("contract_dump");
    let out = run_cli(&["wal", "dump", "1", dir.to_str().unwrap(), "--json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.lines().count() as u64, FIXTURE_RECORDS);
    for line in stdout.lines() {
        let record: Value = serde_json::from_str(line).unwrap();
        for key in ["seq", "type", "len", "crc32"] {
            assert!(record.get(key).is_some(), "{key} missing in {line}");
        }
    }
}

#[test]
fn test_errors_under_json_are_one_stderr_line_and_empty_stdout() {
    let dir = make_test_dir("contract_errors");
    let missing = dir.join("absent");
    for command in ["ls", "verify"] {
        let out = run_cli(&["wal", command, missing.to_str().unwrap(), "--json"]);
        assert_eq!(out.status.code(), Some(1), "{command}");
        assert!(out.stdout.is_empty(), "{command}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(stderr.lines().count(), 1, "{command}");
        let error: Value = serde_json::from_str(stderr.trim()).unwrap();
        assert_eq!(keys(&error), ["error"], "{command}");
    }
}

#[test]
fn test_stream_flag_limits_the_streams() {
    let (dir, _) = fixture("contract_stream_flag");
    fs::create_dir_all(dir.join("2")).unwrap();
    let all = stdout_json(&run_cli(&["wal", "ls", dir.to_str().unwrap(), "--json"]));
    assert_eq!(all["streams"].as_array().unwrap().len(), 2);
    let one = stdout_json(&run_cli(&[
        "wal",
        "ls",
        dir.to_str().unwrap(),
        "-s",
        "2",
        "--json",
    ]));
    assert_eq!(one["streams"].as_array().unwrap().len(), 1);
    assert_eq!(one["streams"][0]["stream_id"], 2);

    let out = run_cli(&["wal", "verify", dir.to_str().unwrap(), "-s", "9", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no wal directory for stream 9"));
}

#[test]
fn test_types_json_contract() {
    let out = run_cli(&["wal", "types", "--json"]);
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    let report = stdout_json(&out);
    assert_eq!(keys(&report), ["types"]);
    let types = report["types"].as_array().unwrap();
    assert_eq!(types.len(), 14);
    assert_eq!(types[0], "FILL");
    assert_eq!(types[13], "LIQUIDATION");
}

#[test]
fn test_verify_json_contract() {
    let dir = make_test_dir("contract_verify");
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5).inserted(1, 1, BUY, 100, 5);
    wal.write_stream(&dir, SYMBOL);
    let out = run_cli(&["verify", dir.to_str().unwrap(), "--json"]);
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    let report = stdout_json(&out);
    assert_eq!(keys(&report), ["checks", "status"]);
    assert_eq!(report["status"], "pass");

    let checks = report["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 15);
    for check in checks {
        assert_eq!(
            keys(check),
            [
                "checked",
                "detail",
                "invariant",
                "name",
                "status",
                "violation_count",
                "violations"
            ]
        );
        assert!(["pass", "fail", "skip"].contains(&check["status"].as_str().unwrap()));
    }
    let invariants: Vec<Option<u64>> = checks
        .iter()
        .map(|check| check["invariant"].as_u64())
        .collect();
    assert_eq!(
        invariants,
        [
            None,
            None,
            Some(1),
            Some(2),
            Some(3),
            Some(3),
            Some(4),
            Some(5),
            Some(5),
            Some(6),
            Some(6),
            Some(7),
            Some(8),
            Some(9),
            Some(10)
        ]
    );
}

#[test]
fn test_verify_violation_json_contract() {
    let dir = make_test_dir("contract_verify_violation");
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5).done(1, 1).done(1, 1);
    wal.write_stream(&dir, SYMBOL);
    let out = run_cli(&["verify", dir.to_str().unwrap(), "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let report = stdout_json(&out);
    assert_eq!(report["status"], "fail");
    let failed = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["status"] == "fail")
        .unwrap();
    assert_eq!(failed["violation_count"], 1);
    assert_eq!(
        keys(&failed["violations"][0]),
        [
            "message",
            "order_id",
            "seq",
            "stream_id",
            "user_id",
            "wal_dir"
        ]
    );
    let error: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(keys(&error), ["error"]);
}
