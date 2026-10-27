mod common;

use common::md_server::closed_url;
use common::md_server::serve;
use common::md_server::After;
use common::run_cli;
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
use serde_json::json;
use serde_json::Value;
use std::io::BufRead;
use std::io::BufReader;
use std::process::Command;
use std::process::Stdio;

fn snapshot_frame(symbol_id: u32, bids: &[(i64, i64)], asks: &[(i64, i64)]) -> Vec<u8> {
    let levels = |pairs: &[(i64, i64)]| {
        pairs
            .iter()
            .map(|&(price, qty)| L2Level {
                price,
                qty,
                count: 1,
            })
            .collect()
    };
    encode_l2_snapshot(&L2Snapshot {
        symbol_id,
        bids: levels(bids),
        asks: levels(asks),
        timestamp_ns: 2000,
        seq: 99,
    })
}

fn bbo_frame() -> Vec<u8> {
    encode_bbo(&BboUpdate {
        symbol_id: 1,
        bid_px: 100,
        bid_qty: 5,
        bid_count: 2,
        ask_px: 101,
        ask_qty: 7,
        ask_count: 3,
        timestamp_ns: 1000,
        seq: 42,
    })
}

fn delta_frame() -> Vec<u8> {
    encode_l2_delta(&L2Delta {
        symbol_id: 1,
        side: 1,
        price: i64::MAX,
        qty: 0,
        count: 0,
        timestamp_ns: 3000,
        seq: 77,
    })
}

fn trade_frame() -> Vec<u8> {
    encode_trade(&TradeEvent {
        symbol_id: 1,
        price: 300,
        qty: 25,
        taker_side: 1,
        timestamp_ns: 8000,
        seq: 66,
    })
}

fn stdout_lines(stdout: &[u8]) -> Vec<Value> {
    String::from_utf8_lossy(stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn stderr_error(stderr: &[u8]) -> String {
    let value: Value = serde_json::from_slice(stderr).unwrap();
    value["error"].as_str().unwrap().to_string()
}

#[test]
fn test_snapshot_prints_the_requested_symbol_book() {
    let server = serve(
        1,
        vec![
            encode_heartbeat(1),
            snapshot_frame(8, &[(5, 5)], &[]),
            snapshot_frame(7, &[(100, 5), (99, 3)], &[(101, 7)]),
        ],
        After::Hold,
    );
    let out = run_cli(&["md", "snapshot", "7", "--json", "-u", &server.url]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        stdout_lines(&out.stdout),
        [json!({
            "symbol_id": 7,
            "bbo": {
                "bid_px": 100, "bid_qty": 5, "ask_px": 101, "ask_qty": 7,
                "seq": 99, "ts_ns": 2000,
            },
            "bids": [[100, 5], [99, 3]],
            "asks": [[101, 7]],
        })]
    );
    assert_eq!(*server.received.lock().unwrap(), [r#"{"S":[7,2]}"#]);
}

#[test]
fn test_snapshot_of_an_empty_book_succeeds() {
    let server = serve(1, vec![snapshot_frame(7, &[], &[])], After::Hold);
    let out = run_cli(&["md", "snapshot", "7", "--json", "-u", &server.url]);
    assert!(out.status.success());
    assert_eq!(
        stdout_lines(&out.stdout),
        [json!({"symbol_id": 7, "bbo": null, "bids": [], "asks": []})]
    );
}

#[test]
fn test_snapshot_text_lists_the_ladder() {
    let server = serve(
        1,
        vec![snapshot_frame(7, &[(100, 5)], &[(101, 7), (102, 1)])],
        After::Hold,
    );
    let out = run_cli(&["md", "snapshot", "7", "-u", &server.url]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "symbol 7 seq 99\nask 102 x 1\nask 101 x 7\nbid 100 x 5\n"
    );
}

#[test]
fn test_snapshot_without_an_answer_times_out_with_a_json_error() {
    let server = serve(1, vec![encode_heartbeat(1)], After::Hold);
    let out = run_cli(&[
        "md",
        "snapshot",
        "7",
        "--json",
        "-t",
        "300",
        "-u",
        &server.url,
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let error = stderr_error(&out.stderr);
    assert!(error.contains("no snapshot for symbol 7"), "{error}");
}

#[test]
fn test_snapshot_reports_a_refused_connection() {
    let url = closed_url();
    let out = run_cli(&["md", "snapshot", "7", "--json", "-u", &url]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let error = stderr_error(&out.stderr);
    assert!(error.contains("failed to connect"), "{error}");
}

#[test]
fn test_snapshot_reports_an_undecodable_frame() {
    let server = serve(1, vec![vec![0, 0, 0, 2, 10, 5]], After::Hold);
    let out = run_cli(&["md", "snapshot", "7", "--json", "-u", &server.url]);
    assert_eq!(out.status.code(), Some(1));
    let error = stderr_error(&out.stderr);
    assert!(error.contains("bad frame"), "{error}");
}

#[test]
fn test_follow_prints_one_json_line_per_frame_until_count() {
    let server = serve(
        1,
        vec![
            snapshot_frame(1, &[(100, 5)], &[(101, 7)]),
            bbo_frame(),
            delta_frame(),
            trade_frame(),
            encode_heartbeat(12345),
            bbo_frame(),
        ],
        After::Hold,
    );
    let out = run_cli(&["md", "follow", "1", "--json", "-n", "5", "-u", &server.url]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines = stdout_lines(&out.stdout);
    let kinds: Vec<&str> = lines
        .iter()
        .map(|line| line["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["l2_snapshot", "bbo", "l2_delta", "trade", "heartbeat"]
    );
    assert_eq!(lines[2]["px"].as_i64(), Some(i64::MAX));
    assert_eq!(lines[2]["qty"], 0);
    assert_eq!(lines[4]["ts_ms"], 12345);
    assert_eq!(*server.received.lock().unwrap(), [r#"{"S":[1,7]}"#]);
}

#[test]
fn test_follow_subscribes_every_symbol_with_the_channels() {
    let server = serve(2, vec![bbo_frame()], After::Hold);
    let out = run_cli(&[
        "md",
        "follow",
        "1",
        "10",
        "-c",
        "3",
        "-n",
        "1",
        "--json",
        "-u",
        &server.url,
    ]);
    assert!(out.status.success());
    assert_eq!(
        *server.received.lock().unwrap(),
        [r#"{"S":[1,3]}"#, r#"{"S":[10,3]}"#]
    );
}

#[test]
fn test_follow_fails_when_the_connection_closes_before_count() {
    let server = serve(1, vec![bbo_frame(), trade_frame()], After::Close);
    let out = run_cli(&["md", "follow", "1", "--json", "-n", "5", "-u", &server.url]);
    assert_eq!(out.status.code(), Some(1));
    let kinds: Vec<Value> = stdout_lines(&out.stdout);
    assert_eq!(kinds.len(), 2);
    let error = stderr_error(&out.stderr);
    assert!(error.contains("closed"), "{error}");
}

#[test]
fn test_follow_exits_cleanly_on_sigterm() {
    let server = serve(1, vec![bbo_frame()], After::Hold);
    let mut child = Command::new(env!("CARGO_BIN_EXE_rsx-cli"))
        .args(["md", "follow", "1", "--json", "-u", &server.url])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut first = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut first)
        .unwrap();
    assert!(first.contains("\"kind\":\"bbo\""), "{first}");
    let killed = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    assert!(child.wait().unwrap().success());
}

#[test]
fn test_follow_needs_a_symbol_and_a_valid_channel_mask() {
    let out = run_cli(&["md", "follow"]);
    assert_eq!(out.status.code(), Some(2));
    let out = run_cli(&["md", "follow", "1", "-c", "8"]);
    assert_eq!(out.status.code(), Some(2));
    let out = run_cli(&["md", "follow", "1", "-c", "0"]);
    assert_eq!(out.status.code(), Some(2));
}
