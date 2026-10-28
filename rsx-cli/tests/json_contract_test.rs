mod common;

#[path = "common/postgres.rs"]
mod postgres;

use common::corrupt_record;
use common::fixture_utils::Wal;
use common::fixture_utils::BUY;
use common::fixture_utils::SYMBOL;
use common::gateway_server;
use common::make_test_dir;
use common::md_server::serve;
use common::md_server::After;
use common::run_cli;
use common::run_cli_with_db;
use common::write_fixture_wal;
use common::FIXTURE_RECORDS;
use rsx_marketdata::types::BboUpdate;
use rsx_marketdata::types::L2Delta;
use rsx_marketdata::types::L2Level;
use rsx_marketdata::types::L2Snapshot;
use rsx_marketdata::types::TradeEvent;
use rsx_marketdata::wire::encode_bbo;
use rsx_marketdata::wire::encode_heartbeat;
use rsx_marketdata::wire::encode_l2_delta;
use rsx_marketdata::wire::encode_l2_snapshot;
use rsx_marketdata::wire::encode_trade;
use serde_json::Value;
use std::fs;
use std::fs::OpenOptions;
use std::path::Path;
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

#[test]
fn test_md_snapshot_json_contract() {
    let level = |price| L2Level {
        price,
        qty: 1,
        count: 1,
    };
    let server = serve(
        1,
        vec![encode_l2_snapshot(&L2Snapshot {
            symbol_id: 7,
            bids: vec![level(100)],
            asks: vec![level(101)],
            timestamp_ns: 5,
            seq: 6,
        })],
        After::Hold,
    );
    let out = run_cli(&["md", "snapshot", "7", "--json", "-u", &server.url]);
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    let report = stdout_json(&out);
    assert_eq!(keys(&report), ["asks", "bbo", "bids", "symbol_id"]);
    assert_eq!(
        keys(&report["bbo"]),
        ["ask_px", "ask_qty", "bid_px", "bid_qty", "seq", "ts_ns"]
    );
    assert_eq!(report["bids"][0].as_array().unwrap().len(), 2);
    assert_eq!(report["asks"][0].as_array().unwrap().len(), 2);
}

#[test]
fn test_md_follow_json_contract() {
    let level = L2Level {
        price: 100,
        qty: 1,
        count: 1,
    };
    let server = serve(
        1,
        vec![
            encode_bbo(&BboUpdate {
                symbol_id: 1,
                bid_px: 1,
                bid_qty: 1,
                bid_count: 1,
                ask_px: 2,
                ask_qty: 1,
                ask_count: 1,
                timestamp_ns: 1,
                seq: 1,
            }),
            encode_l2_snapshot(&L2Snapshot {
                symbol_id: 1,
                bids: vec![level],
                asks: Vec::new(),
                timestamp_ns: 1,
                seq: 1,
            }),
            encode_l2_delta(&L2Delta {
                symbol_id: 1,
                side: 0,
                price: 1,
                qty: 1,
                count: 1,
                timestamp_ns: 1,
                seq: 1,
            }),
            encode_trade(&TradeEvent {
                symbol_id: 1,
                price: 1,
                qty: 1,
                taker_side: 0,
                timestamp_ns: 1,
                seq: 1,
            }),
            encode_heartbeat(1),
        ],
        After::Hold,
    );
    let out = run_cli(&["md", "follow", "1", "--json", "-n", "5", "-u", &server.url]);
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    let lines: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 5);
    assert_eq!(
        keys(&lines[0]),
        [
            "ask_count",
            "ask_px",
            "ask_qty",
            "bid_count",
            "bid_px",
            "bid_qty",
            "kind",
            "seq",
            "symbol_id",
            "ts_ns"
        ]
    );
    assert_eq!(
        keys(&lines[1]),
        ["asks", "bids", "kind", "seq", "symbol_id", "ts_ns"]
    );
    assert_eq!(lines[1]["bids"][0].as_array().unwrap().len(), 3);
    assert_eq!(
        keys(&lines[2]),
        [
            "count",
            "kind",
            "px",
            "qty",
            "seq",
            "side",
            "symbol_id",
            "ts_ns"
        ]
    );
    assert_eq!(
        keys(&lines[3]),
        [
            "kind",
            "px",
            "qty",
            "seq",
            "symbol_id",
            "taker_side",
            "ts_ns"
        ]
    );
    assert_eq!(keys(&lines[4]), ["kind", "ts_ms"]);
}

const ORDER_ID: &str = "0190000000007000800000000000000a";
const GATEWAY_SECRET: &str = "test-secret-padded-to-32-bytes-minlen!";

#[test]
fn test_order_place_json_contract() {
    let fill = format!("{{\"F\":[\"{ORDER_ID}\",\"{ORDER_ID}\",150,3,5000,0,0,0,0,0,0]}}");
    let done = format!("{{\"U\":[\"{ORDER_ID}\",0,3,0,0,9,0,0,0,0]}}");
    let server = gateway_server::serve(
        vec![gateway_server::Script::answering(&[&[&fill, &done]])],
        None,
    );
    let out = run_cli(&[
        "order",
        "place",
        "-u",
        "1",
        "-s",
        "10",
        "--side",
        "buy",
        "--px",
        "150",
        "--qty-raw",
        "3",
        "--json",
        "-g",
        &server.url,
        "--secret",
        GATEWAY_SECRET,
    ]);
    assert!(out.status.success());
    let report = stdout_json(&out);
    assert_eq!(
        keys(&report),
        [
            "cid",
            "fills",
            "frames",
            "order_id",
            "reason",
            "reason_code",
            "reply",
            "rtt_us",
            "status"
        ]
    );
    assert_eq!(keys(&report["fills"][0]), ["px", "qty", "ts_ns"]);
}

#[test]
fn test_order_cancel_json_contract() {
    let cancelled = format!("{{\"U\":[\"{ORDER_ID}\",2,0,3,0,0,0,0,0,0]}}");
    let server = gateway_server::serve(
        vec![gateway_server::Script::answering(&[&[&cancelled]])],
        None,
    );
    let out = run_cli(&[
        "order",
        "cancel",
        "-u",
        "1",
        "abc",
        "--json",
        "-g",
        &server.url,
        "--secret",
        GATEWAY_SECRET,
    ]);
    assert!(out.status.success());
    let report = stdout_json(&out);
    assert_eq!(
        keys(&report),
        [
            "cid",
            "frames",
            "order_id",
            "reason",
            "reason_code",
            "reply",
            "rtt_us",
            "status"
        ]
    );
}

#[test]
fn test_order_batch_json_lines_carry_the_place_keys() {
    let resting = format!("{{\"U\":[\"{ORDER_ID}\",1,0,3,0,0,0,0,0,0]}}");
    let server = gateway_server::serve(
        vec![gateway_server::Script::answering(&[&[&resting]])],
        None,
    );
    let path = std::env::temp_dir().join(format!("rsx_cli_contract_{}.jsonl", std::process::id()));
    fs::write(
        &path,
        r#"{"user_id":1,"symbol_id":10,"side":"buy","px":150,"qty_raw":3}"#,
    )
    .unwrap();
    let out = run_cli(&[
        "order",
        "batch",
        path.to_str().unwrap(),
        "--json",
        "-g",
        &server.url,
        "--secret",
        GATEWAY_SECRET,
    ]);
    fs::remove_file(&path).unwrap();
    assert!(out.status.success());
    let report = stdout_json(&out);
    assert_eq!(
        keys(&report),
        [
            "cid",
            "fills",
            "frames",
            "order_id",
            "reason",
            "reason_code",
            "reply",
            "rtt_us",
            "status"
        ]
    );
}

fn stderr_error(out: &Output) -> String {
    let line = String::from_utf8_lossy(&out.stderr);
    let value: Value =
        serde_json::from_str(line.trim()).unwrap_or_else(|err| panic!("{err}: {line}"));
    value["error"].as_str().unwrap().to_string()
}

#[test]
fn test_cluster_scenarios_json_contract() {
    let out = run_cli(&["cluster", "scenarios", "--json"]);
    assert!(out.status.success());
    let report = stdout_json(&out);
    assert_eq!(keys(&report), ["scenarios"]);
    let scenarios = report["scenarios"].as_array().unwrap();
    assert_eq!(scenarios.len(), 15);
    for scenario in scenarios {
        assert_eq!(keys(scenario), ["load", "name", "replication", "symbols"]);
    }
    let ultra = scenarios
        .iter()
        .find(|s| s["name"] == "stress-ultra")
        .unwrap();
    assert_eq!(keys(&ultra["load"]), ["duration", "rate", "type"]);
    assert_eq!(ultra["replication"], "replica");
    let minimal = scenarios.iter().find(|s| s["name"] == "minimal").unwrap();
    assert!(minimal["load"].is_null());
}

#[test]
fn test_cluster_ps_and_down_json_contracts_on_an_empty_root() {
    let root = make_test_dir("contract_cluster_empty");
    let root = root.to_str().unwrap();
    let ps = run_cli(&["cluster", "ps", "-C", root, "--json"]);
    assert!(ps.status.success());
    assert_eq!(
        stdout_json(&ps),
        serde_json::json!({ "scenario": null, "processes": [] })
    );
    let down = run_cli(&["cluster", "down", "-C", root, "--json"]);
    assert!(down.status.success());
    assert_eq!(
        stdout_json(&down),
        serde_json::json!({ "started": [], "stopped": [], "skipped": [] })
    );
}

#[test]
fn test_cluster_ps_json_rows_carry_the_contract_keys() {
    let root = make_test_dir("contract_cluster_ps");
    let state = root.join("tmp/cluster");
    fs::create_dir_all(state.join("pids")).unwrap();
    fs::write(
        state.join("state.json"),
        r#"{"scenario":"minimal","release":false,"pin_cores":false,"processes":["gw-0","me-pengu"]}"#,
    )
    .unwrap();
    fs::write(
        state.join("pids/gw-0.json"),
        r#"{"name":"gw-0","pid":4000000,"start_ticks":1,"started_ms":99,"log":"log/gw-0.log"}"#,
    )
    .unwrap();
    let out = run_cli(&["cluster", "ps", "-C", root.to_str().unwrap(), "--json"]);
    assert!(out.status.success());
    let report = stdout_json(&out);
    assert_eq!(keys(&report), ["processes", "scenario"]);
    assert_eq!(report["scenario"], "minimal");
    let rows = report["processes"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(keys(row), ["log", "name", "pid", "started_ms", "state"]);
    }
    assert_eq!(rows[0]["name"], "gw-0");
    assert_eq!(rows[0]["state"], "exited");
    assert_eq!(rows[0]["pid"], 4_000_000);
    assert_eq!(rows[0]["started_ms"], 99);
    assert_eq!(rows[1]["state"], "absent");
    assert!(rows[1]["pid"].is_null());
}

#[test]
fn test_cluster_up_unknown_scenario_is_misuse() {
    let root = make_test_dir("contract_cluster_unknown");
    let out = run_cli(&[
        "cluster",
        "up",
        "-C",
        root.to_str().unwrap(),
        "-s",
        "nope",
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(stderr_error(&out).contains("unknown scenario"));
}

#[test]
fn test_cluster_up_with_postgres_down_fails_without_leaking_the_password() {
    let root = make_test_dir("contract_cluster_nopg");
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let url = format!("postgres://rsx:hunter2@127.0.0.1:{port}/rsx");
    let out = run_cli(&[
        "cluster",
        "up",
        "-C",
        root.to_str().unwrap(),
        "--db",
        &url,
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let error = stderr_error(&out);
    assert!(error.contains("postgres is not reachable"), "{error}");
    assert!(!error.contains("hunter2"), "{error}");
    assert!(
        fs::read_dir(root.join("tmp")).is_err(),
        "nothing was started"
    );
}

#[test]
fn test_fault_kill_json_contract() {
    let root = make_test_dir("contract_fault_kill");
    let mut child = common::spawn_recorded(&root, "me-test");
    let out = run_cli(&[
        "fault",
        "kill",
        "me-test",
        "-C",
        root.to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    let report = stdout_json(&out);
    assert_eq!(keys(&report), ["name", "pid", "signal"]);
    assert_eq!(report["name"], "me-test");
    assert_eq!(report["pid"], child.id());
    assert_eq!(report["signal"], "SIGKILL");
    child.wait().unwrap();

    let ps = run_cli(&["cluster", "ps", "-C", root.to_str().unwrap(), "--json"]);
    assert_eq!(stdout_json(&ps)["processes"][0]["state"], "exited");

    let unknown = run_cli(&[
        "fault",
        "kill",
        "nope",
        "-C",
        root.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(unknown.status.code(), Some(1));
    assert!(unknown.stdout.is_empty());
    assert!(stderr_error(&unknown).contains("no record"));
}

#[test]
fn test_fault_wal_corrupt_json_contract() {
    let (dir, segment) = fixture("contract_fault_wal_corrupt");
    let before = fs::read(&segment).unwrap();
    let out = run_cli(&[
        "fault",
        "wal-corrupt",
        "1",
        "--wal-dir",
        dir.to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    let report = stdout_json(&out);
    assert_eq!(
        keys(&report),
        ["file", "new_byte", "offset", "old_byte", "stream_id"]
    );
    assert_eq!(report["stream_id"], 1);
    assert_eq!(report["file"], segment.to_str().unwrap());
    assert_eq!(report["offset"], 24);
    assert_eq!(report["old_byte"], before[24]);
    assert_eq!(report["new_byte"], !before[24]);
    assert_eq!(fs::read(&segment).unwrap()[24], !before[24]);
}

#[test]
#[ignore = "needs sudo, tc and the dummy interface module"]
fn test_fault_net_json_contract_on_a_dummy_interface() {
    const IFACE: &str = "rsxfault0";
    let ip = |args: &[&str]| {
        let status = std::process::Command::new("sudo")
            .args(["-n", "ip"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "ip {args:?}");
    };
    ip(&["link", "add", IFACE, "type", "dummy"]);
    let qdisc = || {
        let out = std::process::Command::new("sudo")
            .args(["-n", "tc", "qdisc", "show", "dev", IFACE])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    let applied = run_cli(&[
        "fault",
        "net",
        "-i",
        IFACE,
        "--delay-ms",
        "5",
        "--loss-pct",
        "1.5",
        "--json",
    ]);
    let netem = qdisc();
    let cleared = run_cli(&["fault", "net", "-i", IFACE, "--clear", "--json"]);
    let after = qdisc();
    ip(&["link", "del", IFACE]);

    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let report = stdout_json(&applied);
    assert_eq!(keys(&report), ["active", "delay_ms", "iface", "loss_pct"]);
    assert_eq!(report["iface"], IFACE);
    assert_eq!(report["delay_ms"], 5);
    assert_eq!(report["loss_pct"], 1.5);
    assert_eq!(report["active"], true);
    assert!(
        netem.contains("netem") && netem.contains("loss 1.5%"),
        "{netem}"
    );
    assert!(
        cleared.status.success(),
        "{}",
        String::from_utf8_lossy(&cleared.stderr)
    );
    let report = stdout_json(&cleared);
    assert_eq!(keys(&report), ["active", "delay_ms", "iface", "loss_pct"]);
    assert_eq!(report["active"], false);
    assert!(report["delay_ms"].is_null() && report["loss_pct"].is_null());
    assert!(!after.contains("netem"), "{after}");
}

#[test]
#[ignore = "needs target/debug/rsx-matching built"]
fn test_fault_dup_order_json_contract_and_wal_records() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    assert!(
        root.join("target/debug/rsx-matching").exists(),
        "run `cargo build -p rsx-matching` first"
    );
    let dir = make_test_dir("contract_fault_dup_order");
    let out = run_cli(&[
        "fault",
        "dup-order",
        "-C",
        root.to_str().unwrap(),
        "--dir",
        dir.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stderr.is_empty());
    let report = stdout_json(&out);
    assert_eq!(keys(&report), ["order_id", "records", "sent"]);
    assert_eq!(report["sent"], 2);
    let order_id = report["order_id"].as_str().unwrap();
    assert_eq!(order_id.len(), 32);

    let records = report["records"].as_array().unwrap();
    let of_type = |name: &str| -> Vec<&Value> {
        records
            .iter()
            .filter(|record| record["type"] == name)
            .collect()
    };
    let accepted = of_type("ORDER_ACCEPTED");
    let failed = of_type("ORDER_FAILED");
    assert_eq!(accepted.len(), 1, "{records:?}");
    assert_eq!(failed.len(), 1, "{records:?}");
    assert!(accepted[0]["seq"].as_u64() < failed[0]["seq"].as_u64());
    assert_eq!(failed[0]["reason"], 3, "FailureReason::DuplicateOrderId");
    for record in records {
        assert_eq!(record["oid"], order_id);
    }

    let dump = run_cli(&[
        "wal",
        "dump",
        "999",
        dir.join("wal").to_str().unwrap(),
        "--type",
        "ORDER_FAILED",
        "--json",
    ]);
    assert!(dump.status.success());
    let line = String::from_utf8_lossy(&dump.stdout);
    assert_eq!(line.lines().count(), 1, "{line}");
    assert!(line.contains(order_id));
}

fn stdout_json_of(args: &[&str], database_url: &str) -> Value {
    let out = run_cli_with_db(args, database_url);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stderr.is_empty(), "{args:?}");
    stdout_json(&out)
}

#[tokio::test]
#[ignore = "needs docker for the postgres container"]
async fn test_config_json_contracts() {
    let (_container, port) = postgres::start_postgres().await;
    let url = postgres::connstr(port);

    let shown = stdout_json_of(&["config", "show", "-s", "10", "--json"], &url);
    assert_eq!(keys(&shown), ["symbols"]);
    let symbol = &shown["symbols"][0];
    assert_eq!(
        keys(symbol),
        [
            "active_version",
            "applied",
            "name",
            "scheduled",
            "symbol_id"
        ]
    );
    assert!(symbol["applied"].is_null());
    assert_eq!(
        keys(&symbol["scheduled"][0]),
        [
            "config_version",
            "created_at_ms",
            "effective_at_ms",
            "funding_interval_sec",
            "funding_rate_max_bps",
            "funding_rate_min_bps",
            "initial_margin_rate_bps",
            "lot_size",
            "maintenance_margin_rate_bps",
            "maker_fee_bps",
            "max_leverage",
            "max_order_qty",
            "min_notional",
            "price_decimals",
            "qty_decimals",
            "status",
            "symbol_id",
            "taker_fee_bps",
            "tick_size",
        ]
    );

    let scheduled = stdout_json_of(
        &[
            "config",
            "schedule",
            "-s",
            "10",
            "--set",
            "max_leverage=5",
            "--json",
        ],
        &url,
    );
    assert_eq!(
        keys(&scheduled),
        ["config_version", "effective_at_ms", "symbol_id"]
    );
    assert_eq!(scheduled["config_version"], 3);

    let client = postgres::connect(port).await;
    client
        .execute(
            "INSERT INTO symbol_config_applied \
             SELECT symbol_id, config_version, effective_at_ms, 5, tick_size, \
                    lot_size, price_decimals, qty_decimals, status, NULL, NULL, NULL, \
                    NULL, NULL, NULL, NULL, NULL, NULL, NULL \
             FROM symbol_config_schedule WHERE symbol_id = 10 AND config_version = 2",
            &[],
        )
        .await
        .unwrap();
    let shown = stdout_json_of(&["config", "show", "-s", "10", "--json"], &url);
    assert_eq!(
        keys(&shown["symbols"][0]["applied"]),
        [
            "applied_at_ns",
            "config_version",
            "effective_at_ms",
            "funding_interval_sec",
            "funding_rate_max_bps",
            "funding_rate_min_bps",
            "initial_margin_rate_bps",
            "lot_size",
            "maintenance_margin_rate_bps",
            "maker_fee_bps",
            "max_leverage",
            "max_order_qty",
            "min_notional",
            "price_decimals",
            "qty_decimals",
            "status",
            "symbol_id",
            "taker_fee_bps",
            "tick_size",
        ]
    );
}

#[tokio::test]
#[ignore = "needs docker for the postgres container"]
async fn test_account_and_risk_json_contracts() {
    let (_container, port) = postgres::start_postgres().await;
    let url = postgres::connstr(port);

    let seeded = stdout_json_of(
        &[
            "account",
            "seed",
            "--count",
            "2",
            "--collateral",
            "1000",
            "--json",
        ],
        &url,
    );
    assert_eq!(keys(&seeded), ["accounts", "collateral"]);

    let client = postgres::connect(port).await;
    client
        .batch_execute(
            "INSERT INTO positions (user_id, symbol_id, long_qty, long_entry_cost) \
                VALUES (1, 10, 10, 1000); \
             INSERT INTO frozen_orders VALUES (1, 1, 10, 10, 40); \
             INSERT INTO insurance_fund VALUES (10, 700, 3); \
             INSERT INTO liquidations VALUES (2, 11, 1, 1, 90, 5, 10, 1, 100);",
        )
        .await
        .unwrap();

    let account = stdout_json_of(&["account", "show", "-u", "1", "--json"], &url);
    assert_eq!(
        keys(&account),
        ["collateral", "frozen", "positions", "user_id"]
    );
    assert_eq!(
        keys(&account["positions"][0]),
        ["avg_px", "net_qty", "symbol_id", "upnl"]
    );
    assert_eq!(
        keys(&account["frozen"][0]),
        ["amount", "order_id", "symbol_id"]
    );

    let overview = stdout_json_of(&["risk", "overview", "--json"], &url);
    assert_eq!(keys(&overview), ["insurance", "liquidations", "users"]);
    assert_eq!(overview["users"][0], account);
    assert_eq!(keys(&overview["insurance"]), ["funds", "total"]);
    assert_eq!(
        keys(&overview["insurance"]["funds"][0]),
        ["balance", "symbol_id", "version"]
    );
    assert_eq!(
        keys(&overview["liquidations"][0]),
        [
            "price",
            "qty",
            "round",
            "side",
            "slippage_bps",
            "status",
            "symbol_id",
            "timestamp_ns",
            "user_id",
        ]
    );
}
