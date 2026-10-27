mod common;

use common::gateway_server::closed_url;
use common::gateway_server::serve;
use common::gateway_server::GatewayServer;
use common::gateway_server::Script;
use rsx_gateway::jwt::validate_jwt;
use serde_json::json;
use serde_json::Value;
use std::collections::HashSet;
use std::io::Write;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

const SECRET: &str = "test-secret-padded-to-32-bytes-minlen!";
const OID: &str = "0190000000007000800000000000000a";
const OTHER: &str = "0190000000007000800000000000000b";
const SYMBOLS: &str =
    r#"{"symbols":[{"id":10,"tick_size":1,"lot_size":1000,"price_decimals":2,"qty_decimals":3}]}"#;

fn update(
    order_id: &str,
    status: u8,
    filled: i64,
    remaining: i64,
    reason: u8,
    stamp: u64,
) -> String {
    format!("{{\"U\":[\"{order_id}\",{status},{filled},{remaining},{reason},{stamp},0,0,0,0]}}")
}

fn fill(taker: &str, maker: &str, price: i64, qty: i64) -> String {
    format!("{{\"F\":[\"{taker}\",\"{maker}\",{price},{qty},5000,0,0,0,0,0,0]}}")
}

fn error(code: u32, message: &str) -> String {
    format!("{{\"E\":[{code},\"{message}\"]}}")
}

fn run_order(server: &GatewayServer, args: &[&str], stdin: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rsx-cli"));
    command
        .arg("order")
        .args(args)
        .args(["-g", &server.url, "--secret", SECRET]);
    command
        .env_remove("RSX_GW_JWT_SECRET")
        .env_remove("GATEWAY_URL");
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("failed to run rsx-cli");
    let mut input = child.stdin.take().unwrap();
    if let Some(text) = stdin {
        input.write_all(text.as_bytes()).unwrap();
    }
    drop(input);
    child.wait_with_output().unwrap()
}

fn place(server: &GatewayServer, extra: &[&str]) -> Output {
    let mut args = vec![
        "place",
        "-u",
        "7",
        "-s",
        "10",
        "--side",
        "buy",
        "--px",
        "150",
        "--qty-raw",
        "3",
        "--json",
    ];
    args.extend_from_slice(extra);
    run_order(server, &args, None)
}

fn stdout_json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: {} / {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn stdout_lines(out: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn stderr_error(out: &Output) -> String {
    let value: Value = serde_json::from_slice(&out.stderr)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&out.stderr)));
    value["error"].as_str().unwrap().to_string()
}

fn sent_orders(server: &GatewayServer) -> Vec<Value> {
    server
        .texts()
        .into_iter()
        .map(|(_, text)| serde_json::from_str::<Value>(&text).unwrap())
        .filter(|value| value.get("H").is_none())
        .collect()
}

fn single_server(replies: &[&[&str]]) -> GatewayServer {
    serve(vec![Script::answering(replies)], None)
}

#[test]
fn test_place_resting_sends_the_new_order_frame_with_a_valid_token() {
    let resting = update(OID, 1, 0, 3, 0, 0);
    let server = single_server(&[&[&resting]]);
    let out = place(&server, &["--cid", "abc123"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value = stdout_json(&out);
    assert_eq!(value["cid"], "abc123");
    assert_eq!(value["status"], "resting");
    assert_eq!(value["order_id"], OID);
    assert_eq!(value["reply"], "U");
    assert_eq!(value["fills"], json!([]));
    assert_eq!(
        value["frames"],
        json!([serde_json::from_str::<Value>(&resting).unwrap()])
    );
    assert!(value["rtt_us"].as_u64().is_some());
    server.wait_closed(1);
    assert_eq!(
        sent_orders(&server),
        [json!({"N": [10, 0, 150, 3, "abc123", 0, 0, 0]})]
    );
    let token = server.auth()[0]
        .strip_prefix("Bearer ")
        .unwrap()
        .to_string();
    assert_eq!(validate_jwt(&token, SECRET), Ok(7));
}

#[test]
fn test_place_sends_side_tif_and_flags() {
    let resting = update(OID, 1, 0, 3, 0, 0);
    let server = single_server(&[&[&resting]]);
    let out = run_order(
        &server,
        &[
            "place",
            "-u",
            "7",
            "-s",
            "10",
            "--side",
            "sell",
            "--px",
            "150",
            "--qty-raw",
            "3",
            "--tif",
            "fok",
            "--reduce-only",
            "--post-only",
            "--cid",
            "x1",
            "--json",
        ],
        None,
    );
    assert_eq!(out.status.code(), Some(0));
    server.wait_closed(1);
    assert_eq!(
        sent_orders(&server),
        [json!({"N": [10, 1, 150, 3, "x1", 2, 1, 1]})]
    );
}

#[test]
fn test_place_filled_lists_the_fills() {
    let first = fill(OID, OTHER, 150, 2);
    let second = fill(OID, OTHER, 151, 1);
    let done = update(OID, 0, 3, 0, 0, 9);
    let server = single_server(&[&[&first, &second, &done]]);
    let out = place(&server, &[]);
    assert_eq!(out.status.code(), Some(0));
    let value = stdout_json(&out);
    assert_eq!(value["status"], "filled");
    assert_eq!(
        value["fills"],
        json!([{"px": 150, "qty": 2, "ts_ns": 5000}, {"px": 151, "qty": 1, "ts_ns": 5000}])
    );
    assert_eq!(value["frames"].as_array().unwrap().len(), 3);
}

#[test]
fn test_place_partial_ioc_is_accepted() {
    let partial = fill(OID, OTHER, 150, 2);
    let done = update(OID, 2, 2, 1, 0, 9);
    let server = single_server(&[&[&partial, &done]]);
    let out = place(&server, &["--tif", "ioc"]);
    assert_eq!(out.status.code(), Some(0));
    let value = stdout_json(&out);
    assert_eq!(value["status"], "partial");
    assert_eq!(value["reason_code"], Value::Null);
}

#[test]
fn test_place_post_only_cancel_names_the_cancel_reason() {
    let cancelled = update(OID, 2, 0, 3, 4, 0);
    let server = single_server(&[&[&cancelled]]);
    let out = place(&server, &["--post-only"]);
    assert_eq!(out.status.code(), Some(0));
    let value = stdout_json(&out);
    assert_eq!(value["status"], "cancelled");
    assert_eq!(value["reason_code"], 4);
    assert_eq!(value["reason"], "POST_ONLY_REJECT");
}

#[test]
fn test_place_rejected_by_risk_exits_1_after_the_outcome() {
    let failed = update(OID, 3, 0, 0, 4, 0);
    let server = single_server(&[&[&failed]]);
    let out = place(&server, &[]);
    assert_eq!(out.status.code(), Some(1));
    let value = stdout_json(&out);
    assert_eq!(value["status"], "rejected");
    assert_eq!(value["reason_code"], 4);
    assert_eq!(value["reason"], "INSUFFICIENT_MARGIN");
    assert_eq!(value["reply"], "U");
    assert!(stderr_error(&out).contains("1 of 1 orders"));
}

#[test]
fn test_place_rejects_name_every_failure_reason() {
    let names = [
        "INVALID_TICK_SIZE",
        "INVALID_LOT_SIZE",
        "SYMBOL_NOT_FOUND",
        "DUPLICATE_ORDER_ID",
        "INSUFFICIENT_MARGIN",
        "OVERLOADED",
        "INTERNAL_ERROR",
        "REDUCE_ONLY_VIOLATION",
        "NETWORK_ERROR",
        "RATE_LIMIT",
        "TIMEOUT",
        "USER_IN_LIQUIDATION",
        "WRONG_SHARD",
        "FILL_OR_KILL",
        "ORDER_TOO_SMALL",
    ];
    for (code, name) in names.iter().enumerate() {
        let failed = update(OID, 3, 0, 0, code as u8, 0);
        let server = single_server(&[&[&failed]]);
        let out = place(&server, &[]);
        assert_eq!(out.status.code(), Some(1), "{name}");
        let value = stdout_json(&out);
        assert_eq!(value["reason_code"], code, "{name}");
        assert_eq!(value["reason"], *name);
        let status = if *name == "TIMEOUT" {
            "timeout"
        } else {
            "rejected"
        };
        assert_eq!(value["status"], status, "{name}");
    }
}

#[test]
fn test_place_duplicate_cid_is_rejected_with_the_first_order_id() {
    let duplicate = update(OID, 3, 0, 0, 3, 0);
    let server = serve(
        vec![
            Script::answering(&[&[&update(OID, 1, 0, 3, 0, 0)]]),
            Script::answering(&[&[&duplicate]]),
        ],
        None,
    );
    let first = place(&server, &["--cid", "dup1"]);
    assert_eq!(first.status.code(), Some(0));
    let second = place(&server, &["--cid", "dup1"]);
    assert_eq!(second.status.code(), Some(1));
    let value = stdout_json(&second);
    assert_eq!(value["cid"], "dup1");
    assert_eq!(value["status"], "rejected");
    assert_eq!(value["reason"], "DUPLICATE_ORDER_ID");
    assert_eq!(value["order_id"], OID);
    server.wait_closed(2);
    let cids: Vec<String> = sent_orders(&server)
        .iter()
        .map(|order| order["N"][4].as_str().unwrap().to_string())
        .collect();
    assert_eq!(cids, ["dup1", "dup1"]);
}

#[test]
fn test_place_reports_every_gateway_error_code() {
    let codes = [
        (5, "overloaded"),
        (8, "network error"),
        (1001, "invalid utf8"),
        (1002, "invalid json"),
        (1004, "unsupported"),
        (1006, "rate limited"),
        (1007, "unknown symbol"),
        (1008, "price not tick aligned"),
        (1009, "qty not lot aligned"),
    ];
    for (code, message) in codes {
        let frame = error(code, message);
        let server = single_server(&[&[&frame]]);
        let out = place(&server, &[]);
        assert_eq!(out.status.code(), Some(1), "E[{code}]");
        let value = stdout_json(&out);
        assert_eq!(value["status"], "rejected", "E[{code}]");
        assert_eq!(value["reason_code"], code, "E[{code}]");
        assert_eq!(value["reason"], message, "E[{code}]");
        assert_eq!(value["reply"], "E", "E[{code}]");
        assert_eq!(value["order_id"], Value::Null, "E[{code}]");
    }
}

#[test]
fn test_place_gateway_timeout_frame_is_a_timeout() {
    let timed_out = update(OID, 3, 0, 0, 10, 0);
    let server = single_server(&[&[&timed_out]]);
    let out = place(&server, &[]);
    assert_eq!(out.status.code(), Some(1));
    let value = stdout_json(&out);
    assert_eq!(value["status"], "timeout");
    assert_eq!(value["reason"], "TIMEOUT");
    assert_eq!(value["reply"], "U");
}

#[test]
fn test_place_without_an_answer_times_out_locally() {
    let server = single_server(&[]);
    let out = place(&server, &["-t", "300"]);
    assert_eq!(out.status.code(), Some(1));
    let value = stdout_json(&out);
    assert_eq!(value["status"], "timeout");
    assert_eq!(value["reply"], Value::Null);
    assert_eq!(value["reason_code"], Value::Null);
    assert_eq!(value["frames"], json!([]));
    assert!(value["rtt_us"].as_u64().unwrap() >= 300_000);
}

#[test]
fn test_gateway_heartbeats_are_echoed_before_and_during_the_wait() {
    let resting = update(OID, 1, 0, 3, 0, 0);
    let server = single_server(&[&[r#"{"H":[2]}"#, &resting]]);
    let out = place(&server, &[]);
    assert_eq!(out.status.code(), Some(0));
    server.wait_closed(1);
    let heartbeats = server
        .texts()
        .into_iter()
        .filter(|(_, text)| text.starts_with("{\"H\":["))
        .count();
    assert_eq!(heartbeats, 2);
    assert_eq!(stdout_json(&out)["status"], "resting");
}

#[test]
fn test_heartbeat_frames_are_not_listed_among_the_order_frames() {
    let resting = update(OID, 1, 0, 3, 0, 0);
    let server = single_server(&[&[r#"{"H":[2]}"#, &resting]]);
    let out = place(&server, &[]);
    assert_eq!(stdout_json(&out)["frames"].as_array().unwrap().len(), 1);
}

#[test]
fn test_a_refused_token_is_a_runtime_failure_naming_the_status() {
    let server = serve(vec![Script::refusing()], None);
    let out = place(&server, &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let message = stderr_error(&out);
    assert!(message.contains("401"), "{message}");
    assert!(message.contains("RSX_GW_JWT_SECRET"), "{message}");
}

#[test]
fn test_unreachable_gateway_is_a_runtime_failure() {
    let url = closed_url();
    let out = Command::new(env!("CARGO_BIN_EXE_rsx-cli"))
        .args([
            "order",
            "place",
            "-u",
            "1",
            "-s",
            "10",
            "--side",
            "buy",
            "--px",
            "1",
            "--qty-raw",
            "1",
            "--json",
            "-g",
            &url,
            "--secret",
            SECRET,
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr_error(&out).contains("failed to connect"));
}

#[test]
fn test_missing_secret_is_misuse() {
    let out = Command::new(env!("CARGO_BIN_EXE_rsx-cli"))
        .args([
            "order",
            "place",
            "-u",
            "1",
            "-s",
            "10",
            "--side",
            "buy",
            "--px",
            "1",
            "--qty-raw",
            "1",
            "--json",
            "-g",
            "ws://127.0.0.1:9",
        ])
        .env_remove("RSX_GW_JWT_SECRET")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr_error(&out).contains("no JWT secret"));
}

#[test]
fn test_place_converts_human_units_with_the_gateway_catalog() {
    let resting = update(OID, 1, 0, 2000, 0, 0);
    let server = serve(vec![Script::answering(&[&[&resting]])], Some(SYMBOLS));
    let out = run_order(
        &server,
        &[
            "place", "-u", "7", "-s", "10", "--side", "buy", "-p", "1.5", "-q", "2", "--cid", "h1",
            "--json",
        ],
        None,
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    server.wait_closed(1);
    assert_eq!(
        sent_orders(&server),
        [json!({"N": [10, 0, 150, 2000, "h1", 0, 0, 0]})]
    );
}

#[test]
fn test_place_min_size_sizes_to_the_symbol_minimum() {
    let resting = update(OID, 1, 0, 4000, 0, 0);
    let server = serve(vec![Script::answering(&[&[&resting]])], Some(SYMBOLS));
    let out = run_order(
        &server,
        &[
            "place",
            "-u",
            "7",
            "-s",
            "10",
            "--side",
            "buy",
            "--px",
            "30",
            "--min-size",
            "--cid",
            "m1",
            "--json",
        ],
        None,
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    server.wait_closed(1);
    assert_eq!(
        sent_orders(&server),
        [json!({"N": [10, 0, 30, 4000, "m1", 0, 0, 0]})]
    );
}

#[test]
fn test_place_misuse_exits_2_and_connects_to_nothing() {
    let server = serve(vec![Script::answering(&[])], Some(SYMBOLS));
    let cases: [&[&str]; 6] = [
        &[
            "place",
            "-u",
            "1",
            "-s",
            "10",
            "--side",
            "buy",
            "--qty-raw",
            "1",
        ],
        &["place", "-u", "1", "-s", "10", "--side", "buy", "--px", "1"],
        &[
            "place",
            "-u",
            "1",
            "-s",
            "10",
            "--side",
            "buy",
            "--px",
            "1",
            "-p",
            "1",
            "--qty-raw",
            "1",
        ],
        &[
            "place",
            "-u",
            "1",
            "-s",
            "10",
            "--side",
            "buy",
            "--px",
            "1",
            "--qty-raw",
            "1",
            "--cid",
            "bad cid",
        ],
        &[
            "place", "-u", "1", "-s", "10", "--side", "buy", "-p", "1.234", "-q", "1",
        ],
        &[
            "place", "-u", "1", "-s", "99", "--side", "buy", "-p", "1", "-q", "1",
        ],
    ];
    for args in cases {
        let out = run_order(&server, args, None);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stdout.is_empty(), "{args:?}");
    }
    assert!(server.auth().is_empty());
}

#[test]
fn test_cids_differ_between_runs() {
    let resting = update(OID, 1, 0, 3, 0, 0);
    let server = serve(vec![Script::answering(&[&[&resting]])], None);
    let first = stdout_json(&place(&server, &[]))["cid"]
        .as_str()
        .unwrap()
        .to_string();
    let second = stdout_json(&place(&server, &[]))["cid"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(first, second);
    assert!(first.len() <= 20 && second.len() <= 20);
    assert!(first.bytes().all(|byte| byte.is_ascii_alphanumeric()));
}

#[test]
fn test_cancel_by_cid_sends_the_cid_and_reports_the_cancelled_update() {
    let cancelled = update(OID, 2, 0, 3, 0, 0);
    let server = single_server(&[&[&cancelled]]);
    let out = run_order(&server, &["cancel", "-u", "7", "abc123", "--json"], None);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value = stdout_json(&out);
    assert_eq!(value["cid"], "abc123");
    assert_eq!(value["status"], "cancelled");
    assert_eq!(value["reason"], "USER_CANCEL");
    assert_eq!(value["order_id"], OID);
    assert!(value.get("fills").is_none());
    server.wait_closed(1);
    assert_eq!(sent_orders(&server), [json!({"C": ["abc123"]})]);
    let token = server.auth()[0]
        .strip_prefix("Bearer ")
        .unwrap()
        .to_string();
    assert_eq!(validate_jwt(&token, SECRET), Ok(7));
}

#[test]
fn test_cancel_by_order_id_ignores_updates_of_other_orders() {
    let other = update(OTHER, 2, 0, 3, 0, 0);
    let cancelled = update(OID, 2, 0, 3, 0, 0);
    let server = single_server(&[&[&other, &cancelled]]);
    let out = run_order(&server, &["cancel", "-u", "7", OID, "--json"], None);
    assert_eq!(out.status.code(), Some(0));
    let value = stdout_json(&out);
    assert_eq!(value["cid"], Value::Null);
    assert_eq!(value["order_id"], OID);
    server.wait_closed(1);
    assert_eq!(sent_orders(&server), [json!({"C": [OID]})]);
}

#[test]
fn test_cancel_of_an_unknown_order_exits_1_with_the_error_code() {
    let frame = error(1005, "order not found");
    let server = single_server(&[&[&frame]]);
    let out = run_order(&server, &["cancel", "-u", "7", "nope", "--json"], None);
    assert_eq!(out.status.code(), Some(1));
    let value = stdout_json(&out);
    assert_eq!(value["status"], "rejected");
    assert_eq!(value["reason_code"], 1005);
    assert_eq!(value["reason"], "order not found");
    assert_eq!(value["reply"], "E");
}

#[test]
fn test_cancel_without_an_answer_times_out() {
    let server = single_server(&[]);
    let out = run_order(
        &server,
        &["cancel", "-u", "7", "abc", "-t", "300", "--json"],
        None,
    );
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stdout_json(&out)["status"], "timeout");
}

#[test]
fn test_cancel_with_a_bad_key_is_misuse() {
    let server = single_server(&[]);
    let out = run_order(&server, &["cancel", "-u", "7", "no way", "--json"], None);
    assert_eq!(out.status.code(), Some(2));
}

fn batch_line(user: u32, price: i64, extra: &str) -> String {
    format!(
        "{{\"user_id\":{user},\"symbol_id\":10,\"side\":\"buy\",\"px\":{price},\"qty_raw\":3{extra}}}\n"
    )
}

#[test]
fn test_batch_prints_one_outcome_per_order_in_input_order_on_a_connection_per_user() {
    let resting = update(OID, 1, 0, 3, 0, 0);
    let failed = update(OTHER, 3, 0, 0, 4, 0);
    let filled = update(OID, 0, 3, 0, 0, 9);
    let server = serve(
        vec![
            Script::answering(&[&[&resting], &[&filled]]),
            Script::answering(&[&[&failed]]),
        ],
        None,
    );
    let input = [
        batch_line(1, 100, ""),
        batch_line(2, 101, ""),
        batch_line(1, 102, ""),
    ]
    .concat();
    let out = run_order(&server, &["batch", "--json"], Some(&input));
    assert_eq!(out.status.code(), Some(1));
    let lines = stdout_lines(&out);
    let statuses: Vec<&str> = lines
        .iter()
        .map(|line| line["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["resting", "rejected", "filled"]);
    assert_eq!(lines[1]["reason"], "INSUFFICIENT_MARGIN");
    assert!(stderr_error(&out).contains("1 of 3 orders"));
    server.wait_closed(2);
    let users: Vec<u32> = server
        .auth()
        .iter()
        .map(|header| validate_jwt(header.strip_prefix("Bearer ").unwrap(), SECRET).unwrap())
        .collect();
    assert_eq!(users, [1, 2]);
    let by_connection: Vec<(usize, i64)> = server
        .texts()
        .into_iter()
        .filter(|(_, text)| !text.starts_with("{\"H\""))
        .map(|(connection, text)| {
            let value: Value = serde_json::from_str(&text).unwrap();
            (connection, value["N"][2].as_i64().unwrap())
        })
        .collect();
    assert_eq!(by_connection, [(0, 100), (1, 101), (0, 102)]);
}

#[test]
fn test_batch_reads_a_file_and_gives_every_order_its_own_cid() {
    let resting = update(OID, 1, 0, 3, 0, 0);
    let frames: Vec<&str> = vec![resting.as_str()];
    let replies: Vec<&[&str]> = (0..50).map(|_| frames.as_slice()).collect();
    let server = single_server(&replies);
    let path = std::env::temp_dir().join(format!("rsx_cli_batch_{}.jsonl", std::process::id()));
    let input: String = (0..50)
        .map(|index| batch_line(1, 100 + index, ""))
        .collect();
    std::fs::write(&path, input).unwrap();
    let out = run_order(&server, &["batch", path.to_str().unwrap(), "--json"], None);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cids: HashSet<String> = stdout_lines(&out)
        .iter()
        .map(|line| line["cid"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(cids.len(), 50);
    let sent: HashSet<String> = sent_orders(&server)
        .iter()
        .map(|order| order["N"][4].as_str().unwrap().to_string())
        .collect();
    assert_eq!(sent, cids);
}

#[test]
fn test_batch_with_human_units_fetches_the_catalog_once() {
    let resting = update(OID, 1, 0, 2000, 0, 0);
    let server = serve(
        vec![Script::answering(&[&[&resting], &[&resting]])],
        Some(SYMBOLS),
    );
    let input = concat!(
        r#"{"user_id":1,"symbol_id":10,"side":"sell","price":"1.5","qty":2,"cid":"b1"}"#,
        "\n",
        r#"{"user_id":1,"symbol_id":10,"side":"sell","price":1.25,"min_size":true,"cid":"b2"}"#,
        "\n"
    );
    let out = run_order(&server, &["batch", "--json"], Some(input));
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    server.wait_closed(1);
    assert_eq!(
        sent_orders(&server),
        [
            json!({"N": [10, 1, 150, 2000, "b1", 0, 0, 0]}),
            json!({"N": [10, 1, 125, 1000, "b2", 0, 0, 0]}),
        ]
    );
}

#[test]
fn test_batch_with_a_bad_line_sends_nothing_and_exits_2() {
    let server = single_server(&[]);
    let input = format!("{}{{\"user_id\":1}}\n", batch_line(1, 100, ""));
    let out = run_order(&server, &["batch", "--json"], Some(&input));
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(stderr_error(&out).starts_with("line 2"));
    assert!(server.auth().is_empty());
}

#[test]
fn test_batch_text_mode_prints_one_line_per_order() {
    let resting = update(OID, 1, 0, 3, 0, 0);
    let server = single_server(&[&[&resting]]);
    let out = run_order(
        &server,
        &["batch"],
        Some(&batch_line(1, 100, ",\"cid\":\"t1\"")),
    );
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        text.starts_with(&format!("resting t1 order_id={OID} rtt_us=")),
        "{text}"
    );
}
