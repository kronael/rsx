mod common;

use common::gateway_server::serve;
use common::gateway_server::GatewayServer;
use common::gateway_server::Responder;
use common::gateway_server::Script;
use common::md_server;
use common::md_server::After;
use rsx_marketdata::types::L2Level;
use rsx_marketdata::types::L2Snapshot;
use rsx_marketdata::wire::encode_l2_snapshot;
use serde_json::Value;
use std::collections::HashSet;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::time::Duration;

const SECRET: &str = "test-secret-padded-to-32-bytes-minlen!";
const SYMBOLS: &str =
    r#"{"symbols":[{"id":10,"tick_size":1,"lot_size":1000,"price_decimals":2,"qty_decimals":3}]}"#;
const BID_PX: i64 = 49_000;
const ASK_PX: i64 = 51_000;
const LEGS: [&str; 5] = [
    "internal",
    "gateway_to_risk",
    "risk_to_me",
    "matching",
    "match_done_to_gateway",
];

fn gateway(responder: Responder) -> GatewayServer {
    serve(vec![Script::responding(responder)], Some(SYMBOLS))
}

fn book(bids: &[(i64, i64)], asks: &[(i64, i64)]) -> md_server::MdServer {
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
    let frame = encode_l2_snapshot(&L2Snapshot {
        symbol_id: 10,
        bids: levels(bids),
        asks: levels(asks),
        timestamp_ns: 2000,
        seq: 1,
    });
    md_server::serve(1, vec![frame], After::Hold)
}

fn stress_command(server: &GatewayServer, md: &md_server::MdServer, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rsx-cli"));
    command
        .arg("stress")
        .args([
            "-g",
            &server.url,
            "--md",
            &md.url,
            "--secret",
            SECRET,
            "--json",
        ])
        .args(args);
    command
        .env_remove("RSX_GW_JWT_SECRET")
        .env_remove("GATEWAY_URL")
        .env_remove("MARKETDATA_WS")
        .env_remove("DATABASE_URL");
    command
}

fn run_stress(server: &GatewayServer, args: &[&str]) -> Output {
    let md = book(&[(BID_PX, 500_000)], &[(ASK_PX, 500_000)]);
    stress_command(server, &md, args)
        .output()
        .expect("failed to run rsx-cli")
}

fn report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "no JSON report ({error}): stdout {} stderr {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn orders(server: &GatewayServer) -> Vec<Vec<Value>> {
    server
        .texts()
        .into_iter()
        .filter_map(|(_, text)| {
            let frame: Value = serde_json::from_str(&text).ok()?;
            frame.get("N")?.as_array().cloned()
        })
        .collect()
}

#[test]
fn load_sends_the_scheduled_orders_and_reports_the_contract() {
    let server = gateway(Responder::filling());
    let output = run_stress(
        &server,
        &["-m", "load", "-r", "200", "-d", "1", "-c", "2", "-u", "2"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = report(&output);
    assert_eq!(json["mode"], "load");
    assert_eq!(json["status"], "passed");
    assert_eq!(json["offered"], 200);
    assert_eq!(json["submitted"], 200);
    assert_eq!(json["send_errors"], 0);
    assert_eq!(json["accepted"], 200);
    assert_eq!(json["rejected"], 0);
    assert_eq!(json["timeouts"], 0);
    assert_eq!(json["errors"], 0);
    assert_eq!(json["filled_orders"], 200);
    assert_eq!(json["config"]["target_rate"], 200);
    assert_eq!(json["config"]["connections"], 2);
    let elapsed_ms = json["elapsed_ms"].as_u64().unwrap();
    assert!((1000..1500).contains(&elapsed_ms), "{elapsed_ms}");
    for key in ["n", "p50", "p95", "p99", "p99_9", "max"] {
        assert!(!json["latency_us"][key].is_null(), "{key}");
    }
    assert_eq!(json["latency_us"]["n"], 200);
    for leg in LEGS {
        assert_eq!(json["server_latency_ns"][leg]["n"], 200, "{leg}");
    }
    assert_eq!(json["server_latency_ns"]["internal"]["p50"], 10_000);
    assert_eq!(json["server_latency_ns"]["matching"]["p99"], 3000);
    assert_eq!(json["per_symbol"]["10"]["accepted"], 200);
}

#[test]
fn load_orders_have_unique_cids_and_the_minimum_size() {
    let server = gateway(Responder::filling());
    let output = run_stress(
        &server,
        &[
            "-r",
            "100",
            "-d",
            "1",
            "-c",
            "4",
            "--min-notional",
            "200000000",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sent = orders(&server);
    assert_eq!(sent.len(), 100);
    let cids: HashSet<&str> = sent
        .iter()
        .map(|order| order[4].as_str().unwrap())
        .collect();
    assert_eq!(cids.len(), 100);
    for order in &sent {
        let (side, price, qty, tif) = (
            order[1].as_i64().unwrap(),
            order[2].as_i64().unwrap(),
            order[3].as_i64().unwrap(),
            order[5].as_i64().unwrap(),
        );
        assert_eq!(tif, 1);
        match side {
            0 => assert_eq!((price, qty), (ASK_PX, 4000)),
            _ => assert_eq!((price, qty), (BID_PX, 5000)),
        }
        assert!(price * qty >= 200_000_000);
    }
    let users: HashSet<String> = server.auth().into_iter().collect();
    assert_eq!(users.len(), 4);
}

#[test]
fn load_keeps_sending_while_the_gateway_is_slow() {
    let server = gateway(Responder::delayed(Duration::from_millis(40)));
    let output = run_stress(&server, &["-r", "50", "-d", "1", "-c", "1", "-u", "1"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = report(&output);
    assert_eq!(json["accepted"], 50);
    let latency = &json["latency_us"];
    assert!(latency["p50"].as_u64().unwrap() > 300_000, "{latency}");
    assert!(latency["max"].as_u64().unwrap() > 900_000, "{latency}");
    assert!(latency["p99"].as_u64().unwrap() >= latency["p50"].as_u64().unwrap());
}

#[test]
fn load_counts_rejects_by_reason_name() {
    let server = gateway(Responder::rejecting_every(4, 14));
    let output = run_stress(&server, &["-r", "40", "-d", "1", "-c", "1", "-u", "1"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = report(&output);
    assert_eq!(json["submitted"], 40);
    assert_eq!(json["accepted"], 30);
    assert_eq!(json["rejected"], 10);
    assert_eq!(json["completed"], 40);
    assert_eq!(json["rejected_by_reason"]["ORDER_TOO_SMALL"], 10);
    assert_eq!(json["latency_us"]["n"], 30);
}

#[test]
fn probe_records_the_round_trip_of_each_order() {
    let server = gateway(Responder::delayed(Duration::from_millis(20)));
    let output = run_stress(&server, &["-m", "probe", "-n", "5", "-w", "2"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = report(&output);
    assert_eq!(json["mode"], "probe");
    assert_eq!(json["submitted"], 5);
    assert_eq!(json["latency_us"]["n"], 5);
    let p50 = json["latency_us"]["p50"].as_u64().unwrap();
    assert!((20_000..500_000).contains(&p50), "{p50}");
    assert!(json["latency_us"]["max"].as_u64().unwrap() >= p50);
    assert_eq!(orders(&server).len(), 7);
}

#[test]
fn reject_counts_the_gateway_error_and_discards_the_warmup() {
    let server = gateway(Responder::refusing_unknown_symbol());
    let md = book(&[], &[]);
    let output = stress_command(&server, &md, &["-m", "reject", "-n", "20", "-w", "5"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = report(&output);
    assert_eq!(json["mode"], "reject");
    assert_eq!(json["status"], "passed");
    assert_eq!(json["submitted"], 20);
    assert_eq!(json["accepted"], 0);
    assert_eq!(json["rejected_by_reason"]["E1007"], 20);
    assert_eq!(json["latency_us"]["n"], 20);
    assert_eq!(json["config"]["measures"], "rejected");
    let sent = orders(&server);
    assert_eq!(sent.len(), 25);
    assert!(sent.iter().all(|order| order[0] == 999_999));
}

#[test]
fn orders_the_gateway_never_answers_time_out_and_fail_the_run() {
    let server = gateway(Responder::answering_only(3));
    let output = run_stress(&server, &["-m", "probe", "-n", "5", "-w", "0", "-t", "200"]);
    assert_eq!(output.status.code(), Some(1));
    let json = report(&output);
    assert_eq!(json["status"], "failed");
    assert_eq!(json["accepted"], 3);
    assert_eq!(json["timeouts"], 2);
    assert!(String::from_utf8_lossy(&output.stderr).contains("answered orders below"));
}

#[test]
fn dry_run_prints_the_plan_and_sends_nothing() {
    let server = gateway(Responder::filling());
    let output = run_stress(&server, &["--dry-run", "--min-notional", "200000000"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = report(&output);
    let plan = &json["symbols"][0];
    assert_eq!(plan["symbol_id"], 10);
    assert_eq!(plan["bid_px"], BID_PX);
    assert_eq!(plan["ask_px"], ASK_PX);
    assert_eq!(plan["bid_qty"], 500_000);
    assert_eq!(plan["buy_lots"], 4);
    assert_eq!(plan["sell_lots"], 5);
    assert!(server.texts().is_empty());
}

#[test]
fn the_report_is_also_written_to_out() {
    let server = gateway(Responder::filling());
    let directory = common::make_test_dir("stress_out");
    let path = directory.join("nested").join("stress.json");
    let output = run_stress(
        &server,
        &[
            "-m",
            "probe",
            "-n",
            "3",
            "-w",
            "0",
            "-o",
            path.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(written, report(&output));
}

#[test]
fn a_book_without_asks_is_an_error() {
    let server = gateway(Responder::filling());
    let md = book(&[(BID_PX, 10)], &[]);
    let output = stress_command(&server, &md, &["-m", "probe", "-n", "1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no asks"), "{stderr}");
    assert!(server.texts().is_empty());
}

#[test]
fn an_unknown_symbol_is_misuse() {
    let server = gateway(Responder::filling());
    let output = run_stress(&server, &["-m", "probe", "-s", "77"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn an_interrupt_stops_sending_and_still_reports() {
    let server = gateway(Responder::filling());
    let md = book(&[(BID_PX, 500_000)], &[(ASK_PX, 500_000)]);
    let child = stress_command(
        &server,
        &md,
        &["-r", "100", "-d", "30", "-c", "1", "-u", "1"],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    // SAFETY: signals a child process this test spawned and still owns.
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = report(&output);
    assert_eq!(json["interrupted"], true);
    let submitted = json["submitted"].as_u64().unwrap();
    assert!((50..300).contains(&submitted), "{submitted}");
    assert_eq!(json["accepted"], json["submitted"]);
}

#[test]
fn orders_are_sized_to_the_minimum_the_gateway_serves() {
    let served = r#"{"symbols":[{"id":10,"tick_size":1,"lot_size":1000,"price_decimals":2,"qty_decimals":3,"min_notional":200000000}]}"#;
    let server = serve(vec![Script::responding(Responder::filling())], Some(served));
    let output = run_stress(&server, &["--dry-run"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan = &report(&output)["symbols"][0];
    assert_eq!(plan["buy_lots"], 4);
    assert_eq!(plan["sell_lots"], 5);
}
