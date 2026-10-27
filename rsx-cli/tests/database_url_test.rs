//! A malformed `DATABASE_URL` stops the config and account commands before
//! any connection: exit 2, one `{"error"}` line under `--json`, nothing on
//! stdout, and no password in the message.

mod common;

use common::run_cli_with_db;
use serde_json::Value;

const MALFORMED: &str = "postgres://rsx:hunter2@127.0.0.1:notaport/rsx";

#[test]
fn a_malformed_database_url_is_misuse_for_every_database_command() {
    let commands: [&[&str]; 5] = [
        &["config", "show", "--json"],
        &[
            "config",
            "schedule",
            "-s",
            "10",
            "--set",
            "max_leverage=5",
            "--json",
        ],
        &["account", "seed", "--json"],
        &["account", "show", "-u", "1", "--json"],
        &["risk", "overview", "--json"],
    ];
    for args in commands {
        let out = run_cli_with_db(args, MALFORMED);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!stderr.contains("hunter2"), "{stderr}");
        let error: Value = serde_json::from_str(stderr.trim()).expect("one json line");
        assert!(
            error["error"]
                .as_str()
                .unwrap()
                .contains("not a valid postgres url"),
            "{stderr}"
        );
    }
}

#[test]
fn an_unreachable_database_is_a_runtime_failure_without_the_password() {
    let url = "postgres://rsx:hunter2@127.0.0.1:1/rsx";
    let out = run_cli_with_db(&["config", "show", "--json"], url);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("postgres is not reachable"), "{stderr}");
    assert!(!stderr.contains("hunter2"), "{stderr}");
}

#[test]
fn a_setting_outside_the_list_or_a_missing_one_is_misuse() {
    for args in [
        &["config", "schedule", "-s", "10", "--set", "status=halted"][..],
        &["config", "schedule", "-s", "10"][..],
        &["account", "seed", "--count", "0"][..],
        &["account", "seed", "--collateral", "-1"][..],
        &["account", "show", "-u", "1", "--mark", "10"][..],
    ] {
        let out = run_cli_with_db(args, "host=127.0.0.1 port=1 user=x dbname=x");
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}
