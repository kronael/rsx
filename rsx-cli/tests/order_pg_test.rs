//! `rsx-cli order place --min-size --db` against a real Postgres. Run under
//! `make integration`.

mod common;

#[path = "common/postgres.rs"]
mod postgres;

use common::gateway_server::serve;
use common::gateway_server::Script;
use rsx_risk::schema::run_migrations;
use rsx_types::client_order::min_order_qty;
use serde_json::json;
use serde_json::Value;
use std::process::Command;

const SECRET: &str = "test-secret-padded-to-32-bytes-minlen!";
const OID: &str = "0190000000007000800000000000000a";
const SYMBOL_ID: i32 = 10;
const PRICE: i64 = 2_000_000;

#[tokio::test]
#[ignore = "needs docker for the postgres container"]
async fn test_min_size_takes_the_minimum_notional_from_the_active_risk_config() {
    let (_container, port) = postgres::start_postgres().await;
    let client = postgres::connect(port).await;
    run_migrations(&client).await.expect("migrations");
    let row = client
        .query_one(
            "SELECT min_notional, lot_size FROM symbol_config_schedule \
             WHERE symbol_id = $1 ORDER BY config_version DESC LIMIT 1",
            &[&SYMBOL_ID],
        )
        .await
        .expect("active config row");
    let min_notional: i64 = row.get(0);
    let lot_size: i64 = row.get(1);
    let expected = min_order_qty(min_notional, PRICE, lot_size);

    let resting = format!("{{\"U\":[\"{OID}\",1,0,{expected},0,0,0,0,0,0]}}");
    let symbols = format!(
        "{{\"symbols\":[{{\"id\":{SYMBOL_ID},\"tick_size\":1,\"lot_size\":{lot_size},\
         \"price_decimals\":8,\"qty_decimals\":8}}]}}"
    );
    let server = serve(vec![Script::answering(&[&[&resting]])], Some(&symbols));
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
            "2000000",
            "--min-size",
            "--cid",
            "pg1",
            "--json",
            "-g",
            &server.url,
            "--secret",
            SECRET,
            "--db",
            &postgres::connstr(port),
        ])
        .env_remove("DATABASE_URL")
        .output()
        .expect("failed to run rsx-cli");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    server.wait_closed(1);
    let sent: Vec<Value> = server
        .texts()
        .iter()
        .map(|(_, text)| serde_json::from_str(text).unwrap())
        .filter(|value: &Value| value.get("N").is_some())
        .collect();
    assert_eq!(
        sent,
        [json!({"N": [10, 0, PRICE, expected, "pg1", 0, 0, 0]})]
    );
    assert!(expected > 0);
}
