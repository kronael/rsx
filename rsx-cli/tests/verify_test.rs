mod common;

use common::fixture_utils::Wal;
use common::fixture_utils::BUY;
use common::fixture_utils::SELL;
use common::fixture_utils::SYMBOL;
use common::make_test_dir;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::Output;

const FILLS: &str = "Fills precede ORDER_DONE";
const COMPLETION: &str = "Every order completes exactly once";
const FIFO: &str = "Fills follow time priority within a price level";
const INTACT: &str = "WAL records are intact";
const SEQS: &str = "WAL seqs strictly increase";
const BBO: &str = "BBO records are not crossed";
const DATABASE: &str = "Postgres is reachable";

fn run_verify(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rsx-cli"))
        .arg("verify")
        .args(args)
        .env_remove("DATABASE_URL")
        .output()
        .expect("failed to run rsx-cli")
}

fn report_of(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "{}: {} / {}",
            err,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn verify_dir(dir: &Path) -> (Output, Value) {
    let out = run_verify(&[dir.to_str().expect("utf-8 path"), "--json"]);
    let report = report_of(&out);
    (out, report)
}

fn check<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["checks"]
        .as_array()
        .expect("checks array")
        .iter()
        .find(|check| check["name"] == name)
        .unwrap_or_else(|| panic!("no check named {}: {}", name, report))
}

fn clean_wal() -> Wal {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5)
        .bbo(100, 5, 0, 0)
        .accepted(2, 2, SELL, 101, 5)
        .inserted(2, 2, SELL, 101, 5)
        .bbo(100, 5, 101, 5)
        .accepted(3, 3, SELL, 100, 5)
        .fill((3, 3), (1, 1), 100, 5, SELL)
        .done(1, 1)
        .done(3, 3)
        .bbo(0, 0, 101, 5);
    wal
}

fn assert_fails(wal: &Wal, name: &str, name_of_dir: &str) -> Value {
    let dir = make_test_dir(name_of_dir);
    wal.write_stream(&dir, SYMBOL);
    let (out, report) = verify_dir(&dir);
    assert_eq!(out.status.code(), Some(1), "{}", report);
    assert_eq!(report["status"], "fail");
    let failed = check(&report, name);
    assert_eq!(failed["status"], "fail", "{}", failed);
    assert!(failed["violation_count"].as_u64().expect("count") >= 1);
    failed.clone()
}

#[test]
fn a_clean_wal_passes_every_wal_check_and_skips_the_database_ones() {
    let dir = make_test_dir("verify_clean");
    clean_wal().write_stream(&dir, SYMBOL);
    let (out, report) = verify_dir(&dir);
    assert!(out.status.success(), "{}", report);
    assert_eq!(report["status"], "pass");
    for name in [INTACT, FILLS, COMPLETION, FIFO, SEQS, BBO] {
        let verdict = check(&report, name);
        assert_eq!(verdict["status"], "pass", "{}", verdict);
        assert!(verdict["checked"].as_u64().expect("checked") > 0);
    }
    for name in [
        DATABASE,
        "Replayed book matches the WAL",
        "Position equals the sum of fills",
        "Risk tips stay within the ME WAL",
        "Replayed book is not crossed",
        "SPSC rings preserve FIFO",
        "Slab has no leak",
        "Funding sums to zero per symbol and interval",
        "At most one holder per shard lock",
    ] {
        assert_eq!(check(&report, name)["status"], "skip", "{}", name);
    }
}

#[test]
fn a_fill_after_the_makers_order_done_fails_invariant_1() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5)
        .accepted(3, 3, SELL, 100, 5)
        .done(1, 1)
        .fill((3, 3), (1, 1), 100, 5, SELL)
        .done(3, 3);
    let failed = assert_fails(&wal, FILLS, "verify_fill_after_done");
    assert_eq!(failed["invariant"], 1);
    assert_eq!(failed["violations"][0]["seq"], 5);
    assert_eq!(failed["violations"][0]["order_done_seq"], 4);
}

#[test]
fn a_second_order_done_fails_invariant_2() {
    let mut wal = clean_wal();
    wal.done(3, 3);
    let failed = assert_fails(&wal, COMPLETION, "verify_second_done");
    assert_eq!(failed["invariant"], 2);
}

#[test]
fn an_order_that_never_completes_fails_invariant_2() {
    let mut wal = clean_wal();
    wal.accepted(4, 4, BUY, 90, 5)
        .accepted(5, 5, BUY, 89, 5)
        .inserted(5, 5, BUY, 89, 5);
    let failed = assert_fails(&wal, COMPLETION, "verify_stranded");
    assert_eq!(failed["violations"][0]["user_id"], 4);
}

#[test]
fn a_fill_that_skips_an_earlier_order_fails_invariant_3() {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5)
        .accepted(2, 2, BUY, 100, 5)
        .inserted(2, 2, BUY, 100, 5)
        .accepted(3, 3, SELL, 100, 5)
        .fill((3, 3), (2, 2), 100, 5, SELL)
        .done(2, 2)
        .done(3, 3);
    let failed = assert_fails(&wal, FIFO, "verify_fifo");
    assert_eq!(failed["invariant"], 3);
    assert_eq!(failed["violations"][0]["earlier_user_id"], 1);
}

#[test]
fn a_repeated_seq_fails_invariant_5() {
    let mut wal = clean_wal();
    wal.set_next_seq(3).bbo(100, 5, 101, 5);
    let failed = assert_fails(&wal, SEQS, "verify_repeated_seq");
    assert_eq!(failed["invariant"], 5);
    assert_eq!(failed["violations"][0]["seq"], 3);
}

#[test]
fn a_bbo_with_bid_above_ask_fails_invariant_6() {
    let mut wal = clean_wal();
    wal.bbo(102, 5, 101, 5);
    let failed = assert_fails(&wal, BBO, "verify_crossed_bbo");
    assert_eq!(failed["invariant"], 6);
    assert_eq!(failed["violations"][0]["bid_px"], 102);
}

#[test]
fn a_flipped_payload_byte_fails_the_integrity_check() {
    let dir = make_test_dir("verify_flipped_byte");
    let segment = clean_wal().write_stream(&dir, SYMBOL);
    let mut bytes = fs::read(&segment).expect("read segment");
    bytes[16 + 8] ^= 0xFF;
    fs::write(&segment, bytes).expect("write segment");
    let (out, report) = verify_dir(&dir);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(check(&report, INTACT)["status"], "fail");
}

#[test]
fn a_record_too_short_for_its_type_fails_the_integrity_check() {
    let mut wal = clean_wal();
    wal.raw(rsx_messages::RECORD_FILL, 16);
    let failed = assert_fails(&wal, INTACT, "verify_short_record");
    assert!(failed["violations"][0]["message"]
        .as_str()
        .expect("message")
        .contains("shorter than its record"));
}

#[test]
fn every_given_wal_dir_is_verified() {
    let good = make_test_dir("verify_two_dirs_good");
    let bad = make_test_dir("verify_two_dirs_bad");
    clean_wal().write_stream(&good, SYMBOL);
    let mut wal = clean_wal();
    wal.done(3, 3);
    wal.write_stream(&bad, SYMBOL);
    let out = run_verify(&[
        good.to_str().expect("utf-8 path"),
        bad.to_str().expect("utf-8 path"),
        "--json",
    ]);
    let report = report_of(&out);
    assert_eq!(out.status.code(), Some(1));
    let failed = check(&report, COMPLETION);
    assert_eq!(
        failed["violations"][0]["wal_dir"],
        bad.to_str().expect("utf-8")
    );
}

#[test]
fn skipped_replay_names_its_reason_once_however_many_streams_there_are() {
    let dir = make_test_dir("verify_replay_reason_once");
    clean_wal().write_stream(&dir, SYMBOL);
    clean_wal().write_stream(&dir, SYMBOL + 1);
    let (_, report) = verify_dir(&dir);
    assert_eq!(
        check(&report, "Replayed book matches the WAL")["detail"],
        "no database url (pass --db or set DATABASE_URL)"
    );
    assert_eq!(check(&report, FILLS)["checked"], 4);
}

#[test]
fn a_directory_without_streams_checks_nothing_and_says_so() {
    let dir = make_test_dir("verify_no_streams");
    let (out, report) = verify_dir(&dir);
    assert!(out.status.success());
    assert_eq!(report["status"], "skip");
    assert_eq!(check(&report, INTACT)["status"], "skip");
}

#[test]
fn a_stream_that_does_not_exist_is_an_error() {
    let dir = make_test_dir("verify_unknown_stream");
    clean_wal().write_stream(&dir, SYMBOL);
    let out = run_verify(&[dir.to_str().expect("utf-8 path"), "-s", "99", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let error: Value = serde_json::from_slice(&out.stderr).expect("error json");
    assert!(error["error"]
        .as_str()
        .expect("error")
        .contains("stream 99"));
}

#[test]
fn an_unreachable_database_fails_its_check_and_skips_the_ones_that_need_it() {
    let dir = make_test_dir("verify_unreachable_db");
    clean_wal().write_stream(&dir, SYMBOL);
    let out = run_verify(&[
        dir.to_str().expect("utf-8 path"),
        "--db",
        "host=127.0.0.1 port=1 user=nobody connect_timeout=2",
        "--json",
    ]);
    let report = report_of(&out);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(check(&report, DATABASE)["status"], "fail");
    assert_eq!(
        check(&report, "Slab has no leak")["detail"],
        "postgres unreachable"
    );
    assert_eq!(check(&report, FILLS)["status"], "pass");
}

#[test]
fn the_text_report_has_one_line_per_check_and_the_status() {
    let dir = make_test_dir("verify_text");
    clean_wal().write_stream(&dir, SYMBOL);
    let out = run_verify(&[dir.to_str().expect("utf-8 path")]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("PASS   1  Fills precede ORDER_DONE"),
        "{}",
        text
    );
    assert!(
        text.contains("SKIP   7  SPSC rings preserve FIFO"),
        "{}",
        text
    );
    assert!(text.trim_end().ends_with("status: pass"), "{}", text);
}
