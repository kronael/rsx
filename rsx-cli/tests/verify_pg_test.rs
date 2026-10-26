//! `rsx-cli verify` against a real Postgres. Run under `make integration`.

mod common;

#[path = "common/postgres.rs"]
mod postgres;

use common::fixture_utils::Wal;
use common::fixture_utils::BUY;
use common::fixture_utils::SELL;
use common::fixture_utils::SYMBOL;
use common::make_test_dir;
use rsx_risk::schema::run_migrations;
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tokio_postgres::Client;

const POSITIONS: &str = "Position equals the sum of fills";
const TIPS: &str = "Risk tips stay within the ME WAL";
const REPLAY_MATCH: &str = "Replayed book matches the WAL";
const REPLAY_CROSSED: &str = "Replayed book is not crossed";
const SLAB: &str = "Slab has no leak";
const FUNDING: &str = "Funding sums to zero per symbol and interval";
const LOCKS: &str = "At most one holder per shard lock";
const DATABASE: &str = "Postgres is reachable";

async fn migrated_postgres() -> (
    testcontainers::ContainerAsync<postgres::PostgresNoLogWait>,
    u16,
    Client,
) {
    let (container, port) = postgres::start_postgres().await;
    let client = postgres::connect(port).await;
    run_migrations(&client).await.expect("migrations");
    (container, port, client)
}

fn verify_with_db(wal_dir: &Path, port: u16) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_rsx-cli"))
        .arg("verify")
        .arg(wal_dir)
        .args(["--db", &postgres::connstr(port), "--json"])
        .env_remove("DATABASE_URL")
        .output()
        .expect("failed to run rsx-cli");
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "{}: {} / {}",
            err,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn status<'a>(report: &'a Value, name: &str) -> &'a str {
    check(report, name)["status"].as_str().expect("status")
}

fn check<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["checks"]
        .as_array()
        .expect("checks array")
        .iter()
        .find(|check| check["name"] == name)
        .unwrap_or_else(|| panic!("no check named {}: {}", name, report))
}

fn traded_wal() -> Wal {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .inserted(1, 1, BUY, 100, 5)
        .accepted(2, 2, SELL, 100, 5)
        .fill((2, 2), (1, 1), 100, 5, SELL)
        .done(1, 1)
        .done(2, 2);
    wal
}

async fn insert_position(client: &Client, user_id: i32, long_qty: i64, short_qty: i64) {
    client
        .execute(
            "INSERT INTO positions (user_id, symbol_id, long_qty, short_qty, \
             long_entry_cost, short_entry_cost, realized_pnl, last_fill_seq, version) \
             VALUES ($1, 1, $2, $3, $4, $5, 0, 4, 1)",
            &[
                &user_id,
                &long_qty,
                &short_qty,
                &(long_qty * 100),
                &(short_qty * 100),
            ],
        )
        .await
        .expect("insert position");
}

async fn insert_applied_config(client: &Client) {
    client
        .execute(
            "INSERT INTO symbol_config_applied (symbol_id, config_version, \
             effective_at_ms, applied_at_ns, tick_size, lot_size, price_decimals, \
             qty_decimals, status) VALUES (1, 1, 0, 0, 1, 1, 2, 3, 'active')",
            &[],
        )
        .await
        .expect("insert applied config");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_reachable_database_passes_its_check() {
    let (_container, port, _client) = migrated_postgres().await;
    let dir = make_test_dir("verify_pg_reachable");
    let report = verify_with_db(&dir, port);
    assert_eq!(status(&report, DATABASE), "pass");
    assert_eq!(status(&report, LOCKS), "skip");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn positions_must_equal_the_sum_of_the_wal_fills() {
    let (_container, port, client) = migrated_postgres().await;
    let dir = make_test_dir("verify_pg_positions");
    traded_wal().write_stream(&dir, SYMBOL);
    insert_position(&client, 1, 5, 0).await;
    insert_position(&client, 2, 0, 5).await;
    client
        .execute(
            "INSERT INTO tips (instance_id, symbol_id, last_seq) VALUES (0, 1, 4)",
            &[],
        )
        .await
        .expect("insert tip");

    let report = verify_with_db(&dir, port);
    assert_eq!(
        status(&report, POSITIONS),
        "pass",
        "{}",
        check(&report, POSITIONS)
    );
    assert_eq!(status(&report, TIPS), "pass");

    client
        .execute("UPDATE positions SET long_qty = 4 WHERE user_id = 1", &[])
        .await
        .expect("corrupt position");
    let report = verify_with_db(&dir, port);
    let failed = check(&report, POSITIONS);
    assert_eq!(failed["status"], "fail");
    assert_eq!(failed["violations"][0]["user_id"], 1);
    assert_eq!(failed["violations"][0]["wal"]["long_qty"], 5);
    assert_eq!(failed["violations"][0]["postgres"]["long_qty"], 4);

    client
        .execute("UPDATE positions SET long_qty = 5 WHERE user_id = 1", &[])
        .await
        .expect("restore position");
    client
        .execute("DELETE FROM positions WHERE user_id = 2", &[])
        .await
        .expect("drop position");
    let report = verify_with_db(&dir, port);
    let failed = check(&report, POSITIONS);
    assert_eq!(failed["status"], "fail", "{}", failed);
    assert_eq!(failed["violations"][0]["user_id"], 2);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_risk_tip_past_the_me_wal_fails() {
    let (_container, port, client) = migrated_postgres().await;
    let dir = make_test_dir("verify_pg_tips");
    traded_wal().write_stream(&dir, SYMBOL);
    client
        .execute(
            "INSERT INTO tips (instance_id, symbol_id, last_seq) VALUES (0, 1, 99)",
            &[],
        )
        .await
        .expect("insert tip");
    let report = verify_with_db(&dir, port);
    let failed = check(&report, TIPS);
    assert_eq!(failed["status"], "fail");
    assert_eq!(failed["violations"][0]["tip"], 99);
    assert_eq!(failed["violations"][0]["wal_last_seq"], 6);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn funding_must_sum_to_zero_per_symbol_and_interval() {
    let (_container, port, client) = migrated_postgres().await;
    let dir = make_test_dir("verify_pg_funding");
    let insert_payment = |user_id: i32, symbol_id: i32, interval_id: i64, amount: i64| {
        let client = &client;
        async move {
            client
                .execute(
                    "INSERT INTO funding (user_id, symbol_id, amount, rate, settlement_ts, \
                     interval_id) VALUES ($1, $2, $3, 0, 0, $4)",
                    &[&user_id, &symbol_id, &amount, &interval_id],
                )
                .await
                .expect("insert payment");
        }
    };
    let settle = |shard_id: i32, symbol_id: i32, interval_id: i64| {
        let client = &client;
        async move {
            client
                .execute(
                    "INSERT INTO funding_intervals (shard_id, symbol_id, interval_id) \
                     VALUES ($1, $2, $3)",
                    &[&shard_id, &symbol_id, &interval_id],
                )
                .await
                .expect("settle interval");
        }
    };

    assert_eq!(status(&verify_with_db(&dir, port), FUNDING), "skip");

    settle(0, 1, 5).await;
    insert_payment(1, 1, 5, 100).await;
    insert_payment(2, 1, 5, -100).await;
    let report = verify_with_db(&dir, port);
    assert_eq!(status(&report, FUNDING), "pass");
    assert_eq!(check(&report, FUNDING)["checked"], 1);

    settle(0, 1, 6).await;
    insert_payment(1, 1, 6, 100).await;
    insert_payment(2, 1, 6, -90).await;
    let report = verify_with_db(&dir, port);
    let failed = check(&report, FUNDING);
    assert_eq!(failed["status"], "fail");
    assert_eq!(failed["violations"][0]["interval_id"], 6);
    assert_eq!(failed["violations"][0]["total"], "10");

    client
        .execute("DELETE FROM funding WHERE interval_id = 6", &[])
        .await
        .expect("drop interval");
    settle(0, 2, 7).await;
    settle(1, 2, 7).await;
    insert_payment(1, 2, 7, 49).await;
    insert_payment(2, 2, 7, -50).await;
    let report = verify_with_db(&dir, port);
    assert_eq!(
        status(&report, FUNDING),
        "pass",
        "{}",
        check(&report, FUNDING)
    );
    assert_eq!(check(&report, FUNDING)["checked"], 1);
    assert!(check(&report, FUNDING)["detail"]
        .as_str()
        .expect("detail")
        .contains("1 intervals not settled by every shard"));

    settle(0, 3, 8).await;
    insert_payment(1, 3, 8, 999).await;
    let report = verify_with_db(&dir, port);
    assert_eq!(check(&report, FUNDING)["checked"], 1);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_shard_lock_held_under_two_key_forms_fails() {
    let (_container, port, _client) = migrated_postgres().await;
    let dir = make_test_dir("verify_pg_locks");

    let first = postgres::connect(port).await;
    first
        .execute("SELECT pg_advisory_lock(7)", &[])
        .await
        .expect("take the shard lock");
    let report = verify_with_db(&dir, port);
    assert_eq!(status(&report, LOCKS), "pass");
    assert_eq!(check(&report, LOCKS)["checked"], 1);

    let second = postgres::connect(port).await;
    second
        .execute("SELECT pg_advisory_lock(0, 7)", &[])
        .await
        .expect("take the same shard lock in the two-int form");
    let report = verify_with_db(&dir, port);
    let failed = check(&report, LOCKS);
    assert_eq!(failed["status"], "fail");
    assert_eq!(failed["violations"][0]["lock_key"], 7);
    assert_eq!(failed["violations"][0]["holders"], 2);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn the_replayed_book_must_match_the_wal() {
    let (_container, port, client) = migrated_postgres().await;
    insert_applied_config(&client).await;

    let dir = make_test_dir("verify_pg_replay");
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5).inserted(1, 1, BUY, 100, 5);
    wal.write_stream(&dir, SYMBOL);
    let report = verify_with_db(&dir, port);
    for name in [REPLAY_MATCH, REPLAY_CROSSED, SLAB] {
        assert_eq!(status(&report, name), "pass", "{}", check(&report, name));
    }

    let dir = make_test_dir("verify_pg_replay_missing_insert");
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5)
        .accepted(2, 2, BUY, 99, 5)
        .inserted(2, 2, BUY, 99, 5);
    wal.write_stream(&dir, SYMBOL);
    let report = verify_with_db(&dir, port);
    let failed = check(&report, REPLAY_MATCH);
    assert_eq!(failed["status"], "fail", "{}", failed);
    assert_eq!(failed["violations"][0]["user_id"], 1);
    assert_eq!(status(&report, REPLAY_CROSSED), "pass");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_symbol_without_a_config_row_is_not_replayed() {
    let (_container, port, client) = migrated_postgres().await;
    client
        .execute(
            "DELETE FROM symbol_config_schedule WHERE symbol_id = 1",
            &[],
        )
        .await
        .expect("drop seeded config");
    let dir = make_test_dir("verify_pg_replay_no_config");
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5).inserted(1, 1, BUY, 100, 5);
    wal.write_stream(&dir, SYMBOL);
    let report = verify_with_db(&dir, port);
    let skipped = check(&report, REPLAY_MATCH);
    assert_eq!(skipped["status"], "skip");
    assert!(skipped["detail"]
        .as_str()
        .expect("detail")
        .contains("no symbol config"));
}
