//! `rsx-cli account seed|show` and `risk overview` against a real Postgres.
//! Run under `make integration`.

mod common;

#[path = "common/postgres.rs"]
mod postgres;

use common::run_cli_with_db;
use serde_json::json;
use serde_json::Value;
use std::process::Output;

fn stdout_json(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("one json object")
}

#[tokio::test]
#[ignore = "needs docker for the postgres container"]
async fn seed_upserts_the_collateral_and_clears_only_the_seeded_frozen_orders() {
    let (_container, port) = postgres::start_postgres().await;
    let url = postgres::connstr(port);

    let seeded = stdout_json(&run_cli_with_db(
        &[
            "account",
            "seed",
            "--count",
            "5",
            "--collateral",
            "12345",
            "--json",
        ],
        &url,
    ));
    assert_eq!(seeded, json!({"accounts": 5, "collateral": 12345}));

    let client = postgres::connect(port).await;
    client
        .batch_execute(
            "UPDATE accounts SET version = 7 WHERE user_id = 2; \
             INSERT INTO accounts (user_id, collateral, version) VALUES (9, 5, 0); \
             INSERT INTO frozen_orders VALUES (2, 1, 2, 10, 40), (4, 1, 3, 10, 41), (9, 1, 4, 10, 42);",
        )
        .await
        .unwrap();

    let reseeded = stdout_json(&run_cli_with_db(
        &[
            "account",
            "seed",
            "--count",
            "3",
            "--collateral",
            "99",
            "--json",
        ],
        &url,
    ));
    assert_eq!(reseeded, json!({"accounts": 3, "collateral": 99}));

    let accounts = client
        .query(
            "SELECT user_id, collateral, version FROM accounts ORDER BY user_id",
            &[],
        )
        .await
        .unwrap();
    let rows: Vec<(i32, i64, i64)> = accounts
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    assert_eq!(
        rows,
        [
            (1, 99, 0),
            (2, 99, 7),
            (3, 99, 0),
            (4, 12345, 0),
            (5, 12345, 0),
            (9, 5, 0)
        ],
        "seeded users get the collateral and keep their version; others stay"
    );
    let frozen = client
        .query("SELECT user_id FROM frozen_orders ORDER BY user_id", &[])
        .await
        .unwrap();
    let users: Vec<i32> = frozen.iter().map(|row| row.get(0)).collect();
    assert_eq!(users, [4, 9], "frozen orders of users 1 to 3 are cleared");
}

#[tokio::test]
#[ignore = "needs docker for the postgres container"]
async fn show_and_overview_read_what_risk_persisted() {
    let (_container, port) = postgres::start_postgres().await;
    let url = postgres::connstr(port);
    stdout_json(&run_cli_with_db(
        &[
            "account",
            "seed",
            "--count",
            "3",
            "--collateral",
            "1000",
            "--json",
        ],
        &url,
    ));
    let client = postgres::connect(port).await;
    client
        .batch_execute(
            "INSERT INTO positions (user_id, symbol_id, long_qty, long_entry_cost) \
                VALUES (2, 10, 10, 1000); \
             INSERT INTO positions (user_id, symbol_id, short_qty, short_entry_cost) \
                VALUES (3, 11, 5, 500); \
             INSERT INTO positions (user_id, symbol_id) VALUES (3, 10); \
             INSERT INTO frozen_orders VALUES \
                (2, 1, 10, 10, 40), (2, -1, 11, 10, 2); \
             INSERT INTO insurance_fund VALUES (10, 700, 3), (11, 300, 1); \
             INSERT INTO liquidations VALUES \
                (3, 11, 1, 1, 90, 5, 10, 1, 100), (3, 11, 2, 1, 80, 5, 20, 1, 200);",
        )
        .await
        .unwrap();

    let shown = stdout_json(&run_cli_with_db(
        &["account", "show", "-u", "2", "--mark", "10=120", "--json"],
        &url,
    ));
    assert_eq!(
        shown,
        json!({
            "user_id": 2,
            "collateral": 1000,
            "positions": [{"symbol_id": 10, "net_qty": 10, "avg_px": 100, "upnl": 200}],
            "frozen": [
                {
                    "order_id": "ffffffffffffffff000000000000000b",
                    "symbol_id": 10,
                    "amount": 2,
                },
                {
                    "order_id": "0000000000000001000000000000000a",
                    "symbol_id": 10,
                    "amount": 40,
                },
            ],
        })
    );

    let unmarked = stdout_json(&run_cli_with_db(
        &["account", "show", "-u", "3", "--json"],
        &url,
    ));
    assert_eq!(
        unmarked["positions"],
        json!([{"symbol_id": 11, "net_qty": -5, "avg_px": 100, "upnl": null}]),
        "an empty position is not listed, an unmarked one has no upnl"
    );

    let missing = run_cli_with_db(&["account", "show", "-u", "42"], &url);
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("user 42 has no account"));

    let overview = stdout_json(&run_cli_with_db(&["risk", "overview", "--json"], &url));
    let users = overview["users"].as_array().unwrap();
    let ids: Vec<i64> = users
        .iter()
        .map(|u| u["user_id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, [1, 2, 3]);
    assert_eq!(users[1], shown_without_mark(&shown));
    assert_eq!(
        overview["insurance"],
        json!({
            "funds": [
                {"symbol_id": 10, "balance": 700, "version": 3},
                {"symbol_id": 11, "balance": 300, "version": 1},
            ],
            "total": 1000,
        })
    );
    let liquidations = overview["liquidations"].as_array().unwrap();
    let rounds: Vec<i64> = liquidations
        .iter()
        .map(|row| row["round"].as_i64().unwrap())
        .collect();
    assert_eq!(rounds, [2, 1], "newest first");
}

fn shown_without_mark(shown: &Value) -> Value {
    let mut unmarked = shown.clone();
    unmarked["positions"][0]["upnl"] = Value::Null;
    unmarked
}
