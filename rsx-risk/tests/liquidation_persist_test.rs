//! Integration tests for liquidation record persistence.

use rsx_risk::persist::flush_batch;
use rsx_risk::persist::insert_liquidations;
use rsx_risk::persist::LiquidationRecord;
use rsx_risk::persist::PersistEvent;
use rsx_risk::schema::run_migrations;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio_postgres::NoTls;

async fn setup_pg() -> (
    testcontainers::ContainerAsync<Postgres>,
    tokio_postgres::Client,
) {
    let container: testcontainers::ContainerAsync<Postgres> =
        Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let connstr = format!(
        "host=localhost port={port} user=postgres \
         password=postgres dbname=postgres"
    );
    let (client, conn) = tokio_postgres::connect(&connstr, NoTls).await.unwrap();
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("pg conn error: {e}");
        }
    });
    run_migrations(&client).await.unwrap();
    (container, client)
}

#[tokio::test]
#[ignore]
async fn liquidation_record_persists() {
    let (_container, mut client) = setup_pg().await;

    let rec = LiquidationRecord {
        user_id: 1,
        symbol_id: 0,
        round: 1,
        side: 0,
        price: 50_000_000,
        qty: 1_000,
        slippage_bps: 25,
        status: 0,
        timestamp_ns: 1_700_000_000_000_000_000,
    };
    let events = vec![PersistEvent::Liquidation(rec)];
    flush_batch(&mut client, 0, &events).await.unwrap();

    let row = client
        .query_one(
            "SELECT user_id, symbol_id, round, side, price, \
              qty, slippage_bps, status, timestamp_ns \
             FROM liquidations WHERE user_id = $1",
            &[&1i32],
        )
        .await
        .unwrap();

    assert_eq!(row.get::<_, i32>(0), 1i32);
    assert_eq!(row.get::<_, i32>(1), 0i32);
    assert_eq!(row.get::<_, i32>(2), 1i32);
    assert_eq!(row.get::<_, i16>(3), 0i16);
    assert_eq!(row.get::<_, i64>(4), 50_000_000i64);
    assert_eq!(row.get::<_, i64>(5), 1_000i64);
    assert_eq!(row.get::<_, i32>(6), 25i32);
    assert_eq!(row.get::<_, i16>(7), 0i16);
    assert_eq!(row.get::<_, i64>(8), 1_700_000_000_000_000_000i64,);
}

#[tokio::test]
#[ignore]
async fn multiple_rounds_persist() {
    let (_container, mut client) = setup_pg().await;

    let rec1 = LiquidationRecord {
        user_id: 2,
        symbol_id: 1,
        round: 1,
        side: 0,
        price: 48_000_000,
        qty: 500,
        slippage_bps: 10,
        status: 0,
        timestamp_ns: 1_700_000_000_000_000_001,
    };
    let rec2 = LiquidationRecord {
        user_id: 2,
        symbol_id: 1,
        round: 2,
        side: 1,
        price: 47_500_000,
        qty: 500,
        slippage_bps: 15,
        status: 0,
        timestamp_ns: 1_700_000_000_000_000_002,
    };

    let tx = client.transaction().await.unwrap();
    insert_liquidations(&tx, &[rec1, rec2]).await.unwrap();
    tx.commit().await.unwrap();

    let rows = client
        .query(
            "SELECT round FROM liquidations \
             WHERE user_id = $1 AND symbol_id = $2 \
             ORDER BY round",
            &[&2i32, &1i32],
        )
        .await
        .unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<_, i32>(0), 1i32);
    assert_eq!(rows[1].get::<_, i32>(0), 2i32);
}

#[tokio::test]
#[ignore]
async fn liquidation_status_variants_persist() {
    let (_container, mut client) = setup_pg().await;

    // status=0: filled; status=1: partial; status=2: cancelled
    let records: Vec<LiquidationRecord> = [0u8, 1, 2]
        .iter()
        .enumerate()
        .map(|(i, &status)| LiquidationRecord {
            user_id: 3,
            symbol_id: 0,
            round: i as u32 + 1,
            side: 0,
            price: 50_000_000,
            qty: 1_000,
            slippage_bps: 0,
            status,
            timestamp_ns: 1_700_000_000_000_000_000 + i as u64,
        })
        .collect();

    let events: Vec<PersistEvent> = records.into_iter().map(PersistEvent::Liquidation).collect();
    flush_batch(&mut client, 0, &events).await.unwrap();

    let rows = client
        .query(
            "SELECT status FROM liquidations \
             WHERE user_id = $1 ORDER BY round",
            &[&3i32],
        )
        .await
        .unwrap();

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].get::<_, i16>(0), 0i16);
    assert_eq!(rows[1].get::<_, i16>(0), 1i16);
    assert_eq!(rows[2].get::<_, i16>(0), 2i16);
}
