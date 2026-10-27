//! `rsx-cli cluster plan` against a real Postgres. Run under
//! `make integration`.

mod common;

#[path = "common/postgres.rs"]
mod postgres;

use common::run_cli;
use serde_json::Value;
use std::fs;
use std::process::Output;

fn golden(name: &str) -> Value {
    let path = format!(
        "{}/tests/golden/cluster/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn plan(url: &str, scenario: &str) -> Output {
    run_cli(&["cluster", "plan", "-s", scenario, "--db", url, "--json"])
}

fn stderr_error(out: &Output) -> String {
    let line = String::from_utf8_lossy(&out.stderr);
    let value: Value = serde_json::from_str(line.trim()).expect("stderr is one JSON line");
    value["error"].as_str().unwrap().to_string()
}

fn url_of(port: u16) -> String {
    format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres")
}

#[tokio::test]
#[ignore = "needs docker for the postgres container"]
async fn test_plan_reads_the_seeded_symbols_and_matches_the_runtime_plan() {
    let (_container, port) = postgres::start_postgres().await;
    let url = url_of(port);
    let out = plan(&url, "trio");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let produced: Value = serde_json::from_slice(&out.stdout).unwrap();
    let expected = golden("trio");
    let produced = produced["processes"].as_array().unwrap();
    let expected = expected["processes"].as_array().unwrap();
    let daemons: Vec<&Value> = produced
        .iter()
        .filter(|process| process["name"] != "maker" && process["name"] != "auth")
        .collect();
    assert_eq!(daemons.len(), expected.len());
    for (process, want) in daemons.iter().zip(expected) {
        assert_eq!(process["name"], want["name"]);
        assert_eq!(process["binary"], want["binary"]);
        let mut env = process["env"].as_object().unwrap().clone();
        let mut want_env = want["env"].as_object().unwrap().clone();
        for key in ["RSX_ME_DATABASE_URL", "DATABASE_URL", "RSX_GW_DATABASE_URL"] {
            if env.contains_key(key) {
                assert_eq!(env[key], url.as_str());
                env.remove(key);
                want_env.remove(key);
            }
        }
        assert_eq!(env, want_env, "{}", process["name"]);
    }
    assert!(produced.iter().any(|process| process["name"] == "maker"));
}

#[tokio::test]
#[ignore = "needs docker for the postgres container"]
async fn test_plan_fails_clearly_when_symbol_static_is_empty() {
    let (_container, port) = postgres::start_postgres().await;
    let url = url_of(port);
    assert!(plan(&url, "minimal").status.success());
    let client = postgres::connect(port).await;
    client
        .batch_execute("DELETE FROM symbol_static")
        .await
        .unwrap();
    let out = plan(&url, "minimal");
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(stderr_error(&out).contains("symbol_static is empty"));
}

#[tokio::test]
#[ignore = "needs docker for the postgres container"]
async fn test_plan_names_a_scenario_symbol_postgres_lacks() {
    let (_container, port) = postgres::start_postgres().await;
    let url = url_of(port);
    assert!(plan(&url, "minimal").status.success());
    let client = postgres::connect(port).await;
    client
        .batch_execute("DELETE FROM symbol_config_schedule WHERE symbol_id = 3")
        .await
        .unwrap();
    assert!(plan(&url, "minimal").status.success());
    let out = plan(&url, "trio");
    assert_eq!(out.status.code(), Some(1));
    let error = stderr_error(&out);
    assert!(error.contains("trio") && error.contains("SOL"), "{error}");
}

#[tokio::test]
#[ignore = "needs docker for the postgres container"]
async fn test_reset_truncates_risk_state_keeps_accounts_and_wipes_the_wal() {
    let (_container, port) = postgres::start_postgres().await;
    let url = url_of(port);
    assert!(plan(&url, "minimal").status.success());
    let client = postgres::connect(port).await;
    client
        .batch_execute(
            "INSERT INTO tips (instance_id, symbol_id, last_seq) VALUES (0, 1, 99); \
             INSERT INTO accounts (user_id, collateral) VALUES (7, 1000)",
        )
        .await
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let wal = root.path().join("tmp/wal/pengu");
    fs::create_dir_all(&wal).unwrap();
    fs::write(wal.join("1.wal"), "x").unwrap();
    fs::write(root.path().join("tmp/md.tip"), "5").unwrap();

    let out = run_cli(&[
        "cluster",
        "reset",
        "-C",
        root.path().to_str().unwrap(),
        "--db",
        &url,
        "--json",
    ]);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        report["cleared"],
        serde_json::json!(["risk-state", "wal", "tips"])
    );
    let tips = client.query("SELECT 1 FROM tips", &[]).await.unwrap();
    assert!(tips.is_empty());
    let accounts = client.query("SELECT 1 FROM accounts", &[]).await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert!(!wal.exists());
    assert!(!root.path().join("tmp/md.tip").exists());
}
