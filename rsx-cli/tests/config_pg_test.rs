//! `rsx-cli config show|schedule` against a real Postgres. Run under
//! `make integration`.

mod common;

#[path = "common/postgres.rs"]
mod postgres;

use common::run_cli_with_db;
use rsx_matching::config::poll_scheduled_configs;
use rsx_risk::config::load_symbol_configs;
use serde_json::Value;
use std::process::Output;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const FAR_FUTURE_MS: u64 = 4_102_444_800_000;

fn stdout_json(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("one json object")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[tokio::test]
#[ignore = "needs docker for the postgres container"]
async fn a_scheduled_version_is_applied_by_risk_and_matching_once_effective() {
    let (_container, port) = postgres::start_postgres().await;
    let url = postgres::connstr(port);

    let shown = stdout_json(&run_cli_with_db(&["config", "show", "--json"], &url));
    let symbols = shown["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 8, "the seeded catalog");
    let pengu = symbols.iter().find(|s| s["symbol_id"] == 10).unwrap();
    assert_eq!(pengu["name"], "PENGU");
    assert_eq!(pengu["active_version"], 2);
    assert!(pengu["applied"].is_null(), "no matching engine ran");
    assert_eq!(pengu["scheduled"].as_array().unwrap().len(), 2);

    let future = FAR_FUTURE_MS.to_string();
    let scheduled = stdout_json(&run_cli_with_db(
        &[
            "config",
            "schedule",
            "-s",
            "10",
            "--set",
            "max_leverage=5",
            "--set",
            "initial_margin_rate_bps=2000",
            "--set",
            "maintenance_margin_rate_bps=1000",
            "--effective-at-ms",
            &future,
            "--json",
        ],
        &url,
    ));
    assert_eq!(scheduled["config_version"], 3);
    assert_eq!(scheduled["effective_at_ms"], FAR_FUTURE_MS);

    let one = stdout_json(&run_cli_with_db(
        &["config", "show", "-s", "10", "--json"],
        &url,
    ));
    let pengu = &one["symbols"][0];
    assert_eq!(one["symbols"].as_array().unwrap().len(), 1);
    assert_eq!(pengu["scheduled"].as_array().unwrap().len(), 3);
    assert_eq!(pengu["scheduled"][2]["max_leverage"], 5);
    assert_eq!(
        pengu["scheduled"][2]["tick_size"],
        pengu["scheduled"][1]["tick_size"]
    );
    assert_eq!(pengu["active_version"], 2, "v3 is not effective yet");

    let client = postgres::connect(port).await;
    let due = poll_scheduled_configs(&client, 10, 2, now_ms())
        .await
        .unwrap();
    assert!(due.is_empty(), "matching does not see a future version");
    let in_force = load_symbol_configs(&client, 16, now_ms()).await.unwrap();
    let pengu_in_force = in_force.iter().find(|c| c.symbol_id == 10).unwrap();
    assert_eq!(pengu_in_force.config_version, 2);

    let before_pending = run_cli_with_db(
        &[
            "config",
            "schedule",
            "-s",
            "10",
            "--set",
            "taker_fee_bps=7",
            "--json",
        ],
        &url,
    );
    assert_eq!(before_pending.status.code(), Some(2));
    assert!(before_pending.stdout.is_empty());
    let error: Value = serde_json::from_slice(&before_pending.stderr).expect("one json line");
    let message = error["error"].as_str().unwrap();
    assert!(
        message.contains("v3") && message.contains(&future) && message.contains("at or after"),
        "{error}"
    );
    let count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM symbol_config_schedule WHERE symbol_id = 10",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 3, "the refused schedule wrote nothing");

    let at_pending = stdout_json(&run_cli_with_db(
        &[
            "config",
            "schedule",
            "-s",
            "10",
            "--set",
            "taker_fee_bps=7",
            "--effective-at-ms",
            &future,
            "--json",
        ],
        &url,
    ));
    assert_eq!(at_pending["config_version"], 4);
    let due = poll_scheduled_configs(&client, 10, 2, FAR_FUTURE_MS)
        .await
        .unwrap();
    let versions: Vec<u64> = due.iter().map(|c| c.config_version).collect();
    assert_eq!(versions, [3, 4], "matching applies the versions in order");
    let in_force = load_symbol_configs(&client, 16, FAR_FUTURE_MS)
        .await
        .unwrap();
    let pengu_in_force = in_force.iter().find(|c| c.symbol_id == 10).unwrap();
    assert_eq!(pengu_in_force.config_version, 4);
    assert_eq!(pengu_in_force.taker_fee_bps, 7);
    assert_eq!(
        pengu_in_force.params.max_leverage, 5,
        "v4 copies the pending v3"
    );
}

#[tokio::test]
#[ignore = "needs docker for the postgres container"]
async fn a_refused_schedule_writes_nothing_and_reports_the_reason() {
    let (_container, port) = postgres::start_postgres().await;
    let url = postgres::connstr(port);

    let refused = run_cli_with_db(
        &[
            "config",
            "schedule",
            "-s",
            "10",
            "--set",
            "maintenance_margin_rate_bps=1000",
            "--json",
        ],
        &url,
    );
    assert_eq!(refused.status.code(), Some(2));
    assert!(refused.stdout.is_empty());
    let error: Value = serde_json::from_slice(&refused.stderr).expect("one json line");
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("must be below initial_margin_rate_bps"),
        "{error}"
    );

    let unknown = run_cli_with_db(
        &["config", "schedule", "-s", "99", "--set", "max_leverage=3"],
        &url,
    );
    assert_eq!(unknown.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("symbol 99 has no config row"));

    let absent = run_cli_with_db(&["config", "show", "-s", "99"], &url);
    assert_eq!(absent.status.code(), Some(1));

    let client = postgres::connect(port).await;
    let count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM symbol_config_schedule WHERE symbol_id = 10",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 2, "nothing was inserted");
}
