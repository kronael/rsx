mod common;

use common::make_test_dir;
use common::run_cli;
use common::write_fixture_wal;
use common::FIXTURE_RECORDS;
use common::FIXTURE_STREAM;
use std::fs;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn golden(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {}", path.display(), err))
}

fn fixture_dir(name: &str) -> PathBuf {
    let dir = make_test_dir(name);
    write_fixture_wal(&dir);
    dir
}

fn assert_matches_golden(case: &str, dir: &Path, args: &[&str]) {
    let file = dir.join(FIXTURE_STREAM.to_string()).join("1_1_15.wal");
    let args: Vec<String> = args
        .iter()
        .map(|arg| {
            arg.replace("{dir}", dir.to_str().unwrap())
                .replace("{file}", file.to_str().unwrap())
        })
        .collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run_cli(&args);
    assert!(out.status.success(), "{case}: {:?}", out.status);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        golden(&format!("{case}.out")),
        "{case}: stdout"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        golden(&format!("{case}.err")),
        "{case}: stderr"
    );
}

#[test]
fn test_output_matches_goldens() {
    let dir = fixture_dir("golden");
    let cases: [(&str, &[&str]); 12] = [
        ("text", &["wal", "dump", "1", "{dir}"]),
        ("json", &["wal", "dump", "1", "{dir}", "--json"]),
        ("stats", &["wal", "stats", "1", "{dir}"]),
        (
            "types_json",
            &[
                "wal", "dump", "1", "{dir}", "--type", "FILL", "--type", "BBO", "--json",
            ],
        ),
        (
            "types_text",
            &["wal", "dump", "1", "{dir}", "--type", "ORDER_DONE"],
        ),
        (
            "symbol_json",
            &["wal", "dump", "1", "{dir}", "--symbol", "7", "--json"],
        ),
        (
            "user_json",
            &["wal", "dump", "1", "{dir}", "--user", "100", "--json"],
        ),
        (
            "window_json",
            &[
                "wal",
                "dump",
                "1",
                "{dir}",
                "--from-ts",
                "4000",
                "--to-ts",
                "9000",
                "--json",
            ],
        ),
        (
            "scale_text",
            &[
                "wal",
                "dump",
                "1",
                "{dir}",
                "--tick-size",
                "0.1",
                "--lot-size",
                "0.001",
            ],
        ),
        (
            "from_seq_json",
            &["wal", "dump", "1", "{dir}", "5", "--json"],
        ),
        (
            "stats_symbol",
            &["wal", "stats", "1", "{dir}", "--symbol", "7"],
        ),
        ("file", &["wal", "file", "{file}"]),
    ];
    for (case, args) in cases {
        assert_matches_golden(case, &dir, args);
    }
}

fn read_follow_lines(args: &[&str], count: usize) -> Vec<String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rsx-cli"))
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn rsx-cli");
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().take(count) {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let mut lines = Vec::new();
    while lines.len() < count {
        match receiver.recv_timeout(Duration::from_secs(10)) {
            Ok(line) => lines.push(line),
            Err(_) => break,
        }
    }
    child.kill().unwrap();
    child.wait().unwrap();
    lines
}

#[test]
fn test_follow_json_emits_the_stream_then_keeps_running() {
    let dir = fixture_dir("follow_json");
    let lines = read_follow_lines(
        &[
            "wal",
            "dump",
            "1",
            dir.to_str().unwrap(),
            "--json",
            "--follow",
        ],
        FIXTURE_RECORDS as usize,
    );
    let expected = golden("json.out");
    assert_eq!(lines.join("\n") + "\n", expected);
}

#[test]
fn test_follow_text_emits_the_stream_then_keeps_running() {
    let dir = fixture_dir("follow_text");
    let lines = read_follow_lines(
        &["wal", "dump", "1", dir.to_str().unwrap(), "--follow"],
        FIXTURE_RECORDS as usize,
    );
    let expected = golden("text.out");
    assert_eq!(lines.join("\n") + "\n", expected);
}

#[test]
fn test_unknown_record_type_exits_2() {
    let dir = fixture_dir("misuse");
    let out = run_cli(&["wal", "dump", "1", dir.to_str().unwrap(), "--type", "NOPE"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "Error: unknown record type: NOPE\n"
    );
    assert!(out.stdout.is_empty());
}

#[test]
fn test_unknown_record_type_under_json_is_one_json_line() {
    let dir = fixture_dir("misuse_json");
    let out = run_cli(&[
        "wal",
        "dump",
        "1",
        dir.to_str().unwrap(),
        "--type",
        "NOPE",
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "{\"error\":\"unknown record type: NOPE\"}\n"
    );
    assert!(out.stdout.is_empty());
}

#[test]
fn test_missing_file_exits_1() {
    let out = run_cli(&["wal", "file", "/nonexistent/rsx-cli/x.wal"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("Error: failed to open "));
}

#[test]
fn test_old_command_names_are_gone() {
    let dir = fixture_dir("old_names");
    let dir = dir.to_str().unwrap();
    assert_eq!(run_cli(&["wal-dump", "1", dir]).status.code(), Some(2));
    assert_eq!(run_cli(&["dump", dir]).status.code(), Some(2));
}
