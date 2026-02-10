/// Integration tests for insurance fund persistence.

use rsx_risk::insurance::InsuranceFund;
use rsx_risk::persist::flush_batch;
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
    let (client, conn) =
        tokio_postgres::connect(&connstr, NoTls)
            .await
            .unwrap();
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
async fn insurance_fund_upsert_new() {
    let (_container, mut client) = setup_pg().await;
    let fund = InsuranceFund::new(100, 50_000);
    let events =
        vec![PersistEvent::InsuranceFund(fund.clone())];
    flush_batch(&mut client, 0, &events).await.unwrap();

    let row = client
        .query_one(
            "SELECT balance, version \
             FROM insurance_fund WHERE symbol_id = $1",
            &[&100i32],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 50_000);
    assert_eq!(row.get::<_, i64>(1), 0);
}

#[tokio::test]
#[ignore]
async fn insurance_fund_upsert_update() {
    let (_container, mut client) = setup_pg().await;

    let fund1 = InsuranceFund::new(100, 50_000);
    let events1 =
        vec![PersistEvent::InsuranceFund(fund1.clone())];
    flush_batch(&mut client, 0, &events1).await.unwrap();

    let mut fund2 = InsuranceFund::new(100, 50_000);
    fund2.deduct(10_000);
    let events2 =
        vec![PersistEvent::InsuranceFund(fund2.clone())];
    flush_batch(&mut client, 0, &events2).await.unwrap();

    let row = client
        .query_one(
            "SELECT balance, version \
             FROM insurance_fund WHERE symbol_id = $1",
            &[&100i32],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 40_000);
    assert_eq!(row.get::<_, i64>(1), 1);
}

#[tokio::test]
#[ignore]
async fn insurance_fund_negative_balance_persisted() {
    let (_container, mut client) = setup_pg().await;
    let mut fund = InsuranceFund::new(100, 10_000);
    fund.deduct(20_000);
    let events =
        vec![PersistEvent::InsuranceFund(fund.clone())];
    flush_batch(&mut client, 0, &events).await.unwrap();

    let row = client
        .query_one(
            "SELECT balance FROM insurance_fund \
             WHERE symbol_id = $1",
            &[&100i32],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), -10_000);
}

#[tokio::test]
#[ignore]
async fn multiple_symbols_independent_funds() {
    let (_container, mut client) = setup_pg().await;

    let fund1 = InsuranceFund::new(100, 50_000);
    let fund2 = InsuranceFund::new(200, 75_000);
    let events = vec![
        PersistEvent::InsuranceFund(fund1.clone()),
        PersistEvent::InsuranceFund(fund2.clone()),
    ];
    flush_batch(&mut client, 0, &events).await.unwrap();

    let rows = client
        .query(
            "SELECT symbol_id, balance \
             FROM insurance_fund ORDER BY symbol_id",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<_, i32>(0), 100);
    assert_eq!(rows[0].get::<_, i64>(1), 50_000);
    assert_eq!(rows[1].get::<_, i32>(0), 200);
    assert_eq!(rows[1].get::<_, i64>(1), 75_000);
}

#[tokio::test]
#[ignore]
async fn insurance_fund_version_increments() {
    let (_container, mut client) = setup_pg().await;

    let fund1 = InsuranceFund::new(100, 100_000);
    let events1 =
        vec![PersistEvent::InsuranceFund(fund1.clone())];
    flush_batch(&mut client, 0, &events1).await.unwrap();

    let mut fund2 = fund1.clone();
    fund2.deduct(10_000);
    let events2 =
        vec![PersistEvent::InsuranceFund(fund2.clone())];
    flush_batch(&mut client, 0, &events2).await.unwrap();

    let mut fund3 = fund2.clone();
    fund3.add(5_000);
    let events3 =
        vec![PersistEvent::InsuranceFund(fund3.clone())];
    flush_batch(&mut client, 0, &events3).await.unwrap();

    let row = client
        .query_one(
            "SELECT version FROM insurance_fund \
             WHERE symbol_id = $1",
            &[&100i32],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 2);
}

#[tokio::test]
#[ignore]
async fn batch_with_mixed_events() {
    let (_container, mut client) = setup_pg().await;

    let fund = InsuranceFund::new(100, 50_000);
    let events = vec![
        PersistEvent::Tip {
            symbol_id: 100,
            seq: 42,
        },
        PersistEvent::InsuranceFund(fund.clone()),
    ];
    flush_batch(&mut client, 0, &events).await.unwrap();

    let fund_row = client
        .query_one(
            "SELECT balance FROM insurance_fund \
             WHERE symbol_id = $1",
            &[&100i32],
        )
        .await
        .unwrap();
    assert_eq!(fund_row.get::<_, i64>(0), 50_000);

    let tip_row = client
        .query_one(
            "SELECT last_seq FROM tips \
             WHERE symbol_id = $1 AND instance_id = $2",
            &[&100i32, &0i32],
        )
        .await
        .unwrap();
    assert_eq!(tip_row.get::<_, i64>(0), 42);
}
