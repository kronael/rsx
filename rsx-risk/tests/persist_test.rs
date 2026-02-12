use rsx_risk::Account;
use rsx_risk::FundingConfig;
use rsx_risk::LiquidationConfig;
use rsx_risk::Position;
use rsx_risk::ReplicationConfig;
use rsx_risk::RiskShard;
use rsx_risk::ShardConfig;
use rsx_risk::SymbolRiskParams;
use rsx_risk::persist::FundingPaymentRecord;
use rsx_risk::persist::PersistFill;
use rsx_risk::persist::PersistEvent;
use rsx_risk::persist::flush_batch;
use rsx_risk::persist::insert_fills;
use rsx_risk::persist::insert_funding;
use rsx_risk::persist::upsert_accounts;
use rsx_risk::persist::upsert_positions;
use rsx_risk::persist::upsert_tips;
use rsx_risk::replay::acquire_advisory_lock;
use rsx_risk::replay::load_from_postgres;
use rsx_risk::replay::replay_from_wal;
use rsx_risk::schema::run_migrations;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio_postgres::NoTls;

async fn pg_client() -> (
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
async fn persist_positions_roundtrip() {
    let (_c, mut client) = pg_client().await;
    let mut pos = Position::new(1, 0);
    pos.long_qty = 100;
    pos.long_entry_cost = 5000;
    pos.version = 3;
    let tx = client.transaction().await.unwrap();
    upsert_positions(&tx, &[pos.clone()]).await.unwrap();
    tx.commit().await.unwrap();

    let rows = client
        .query(
            "SELECT long_qty, version FROM positions \
             WHERE user_id = 1 AND symbol_id = 0",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, i64>(0), 100);
    assert_eq!(rows[0].get::<_, i64>(1), 3);
}

#[tokio::test]
#[ignore]
async fn persist_accounts_roundtrip() {
    let (_c, mut client) = pg_client().await;
    let mut acct = Account::new(42, 10_000);
    acct.frozen_margin = 500;
    acct.version = 7;
    let tx = client.transaction().await.unwrap();
    upsert_accounts(&tx, &[acct]).await.unwrap();
    tx.commit().await.unwrap();

    let rows = client
        .query(
            "SELECT collateral, frozen_margin, version \
             FROM accounts WHERE user_id = 42",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, i64>(0), 10_000);
    assert_eq!(rows[0].get::<_, i64>(1), 500);
    assert_eq!(rows[0].get::<_, i64>(2), 7);
}

#[tokio::test]
#[ignore]
async fn persist_fills_batch_insert() {
    let (_c, mut client) = pg_client().await;
    let fills: Vec<PersistFill> = (0..5)
        .map(|i| PersistFill {
            symbol_id: 0,
            taker_user_id: 1,
            maker_user_id: 2,
            price: 1000,
            qty: 10,
            taker_fee: 1,
            maker_fee: 0,
            taker_side: 0,
            seq: i,
            timestamp_ns: 1000 + i,
        })
        .collect();
    let tx = client.transaction().await.unwrap();
    insert_fills(&tx, &fills).await.unwrap();
    tx.commit().await.unwrap();

    let rows = client
        .query("SELECT count(*) FROM fills", &[])
        .await
        .unwrap();
    assert_eq!(rows[0].get::<_, i64>(0), 5);
}

#[tokio::test]
#[ignore]
async fn persist_fills_symbol_seq_is_unique() {
    let (_c, mut client) = pg_client().await;
    let first = PersistFill {
        symbol_id: 7,
        taker_user_id: 1,
        maker_user_id: 2,
        price: 1000,
        qty: 10,
        taker_fee: 1,
        maker_fee: 0,
        taker_side: 0,
        seq: 42,
        timestamp_ns: 1000,
    };
    let dup = PersistFill {
        symbol_id: 7,
        taker_user_id: 3,
        maker_user_id: 4,
        price: 1001,
        qty: 11,
        taker_fee: 1,
        maker_fee: 0,
        taker_side: 1,
        seq: 42,
        timestamp_ns: 1001,
    };

    let tx = client.transaction().await.unwrap();
    insert_fills(&tx, &[first]).await.unwrap();
    tx.commit().await.unwrap();

    let tx = client.transaction().await.unwrap();
    let result = insert_fills(&tx, &[dup]).await;
    assert!(result.is_err());
    let _ = tx.rollback().await;
}

#[tokio::test]
#[ignore]
async fn persist_tips_roundtrip() {
    let (_c, mut client) = pg_client().await;
    let tx = client.transaction().await.unwrap();
    upsert_tips(&tx, 0, &[(0, 100), (1, 200)])
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let rows = client
        .query(
            "SELECT symbol_id, seq FROM tips \
             WHERE instance_id = 0 ORDER BY symbol_id",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<_, i64>(1), 100);
    assert_eq!(rows[1].get::<_, i64>(1), 200);
}

#[tokio::test]
#[ignore]
async fn persist_funding_payments_append() {
    let (_c, mut client) = pg_client().await;
    let payments = vec![
        FundingPaymentRecord {
            user_id: 1,
            symbol_id: 0,
            amount: 50,
            rate: 10,
            settlement_ts: 28800,
        },
        FundingPaymentRecord {
            user_id: 1,
            symbol_id: 0,
            amount: -30,
            rate: -5,
            settlement_ts: 57600,
        },
    ];
    let tx = client.transaction().await.unwrap();
    insert_funding(&tx, &payments).await.unwrap();
    tx.commit().await.unwrap();

    let rows = client
        .query(
            "SELECT count(*) FROM funding_payments",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows[0].get::<_, i64>(0), 2);
}

#[tokio::test]
#[ignore]
async fn persist_empty_batch_no_transaction() {
    let (_c, mut client) = pg_client().await;
    flush_batch(&mut client, 0, &[]).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn persist_position_overwritten_by_later_version() {
    let (_c, mut client) = pg_client().await;

    let mut pos = Position::new(1, 0);
    pos.long_qty = 100;
    pos.version = 1;
    let tx = client.transaction().await.unwrap();
    upsert_positions(&tx, &[pos.clone()]).await.unwrap();
    tx.commit().await.unwrap();

    pos.long_qty = 200;
    pos.version = 2;
    let tx = client.transaction().await.unwrap();
    upsert_positions(&tx, &[pos]).await.unwrap();
    tx.commit().await.unwrap();

    let rows = client
        .query(
            "SELECT long_qty, version FROM positions \
             WHERE user_id = 1",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows[0].get::<_, i64>(0), 200);
    assert_eq!(rows[0].get::<_, i64>(1), 2);
}

#[tokio::test]
#[ignore]
async fn persist_no_version_guard_on_upsert() {
    let (_c, mut client) = pg_client().await;
    let mut pos = Position::new(1, 0);
    pos.version = 5;
    let tx = client.transaction().await.unwrap();
    upsert_positions(&tx, &[pos.clone()]).await.unwrap();
    tx.commit().await.unwrap();

    // Lower version overwrites (no guard)
    pos.version = 3;
    pos.long_qty = 999;
    let tx = client.transaction().await.unwrap();
    upsert_positions(&tx, &[pos]).await.unwrap();
    tx.commit().await.unwrap();

    let rows = client
        .query(
            "SELECT long_qty, version FROM positions \
             WHERE user_id = 1",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows[0].get::<_, i64>(0), 999);
    assert_eq!(rows[0].get::<_, i64>(1), 3);
}

#[tokio::test]
#[ignore]
async fn cold_start_loads_positions() {
    let (_c, mut client) = pg_client().await;
    let mut pos = Position::new(0, 0);
    pos.long_qty = 50;
    pos.version = 2;
    let tx = client.transaction().await.unwrap();
    upsert_positions(&tx, &[pos]).await.unwrap();
    tx.commit().await.unwrap();

    let state =
        load_from_postgres(&client, 0, 1, 4)
            .await
            .unwrap();
    let p = &state.positions[&(0, 0)];
    assert_eq!(p.long_qty, 50);
    assert_eq!(p.version, 2);
}

#[tokio::test]
#[ignore]
async fn cold_start_loads_accounts() {
    let (_c, mut client) = pg_client().await;
    let acct = Account::new(0, 5000);
    let tx = client.transaction().await.unwrap();
    upsert_accounts(&tx, &[acct]).await.unwrap();
    tx.commit().await.unwrap();

    let state =
        load_from_postgres(&client, 0, 1, 4)
            .await
            .unwrap();
    assert_eq!(state.accounts[&0].collateral, 5000);
}

#[tokio::test]
#[ignore]
async fn cold_start_loads_tips() {
    let (_c, mut client) = pg_client().await;
    let tx = client.transaction().await.unwrap();
    upsert_tips(&tx, 0, &[(0, 42), (2, 99)])
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let state =
        load_from_postgres(&client, 0, 1, 4)
            .await
            .unwrap();
    assert_eq!(state.tips[0], 42);
    assert_eq!(state.tips[1], 0);
    assert_eq!(state.tips[2], 99);
}

#[tokio::test]
#[ignore]
async fn cold_start_with_empty_postgres() {
    let (_c, client) = pg_client().await;
    let state =
        load_from_postgres(&client, 0, 1, 4)
            .await
            .unwrap();
    assert!(state.accounts.is_empty());
    assert!(state.positions.is_empty());
    assert_eq!(state.tips, vec![0u64; 4]);
}

#[tokio::test]
#[ignore]
async fn upsert_idempotent_on_replay() {
    let (_c, mut client) = pg_client().await;
    let acct = Account::new(1, 1000);
    for _ in 0..3 {
        let tx = client.transaction().await.unwrap();
        upsert_accounts(&tx, &[acct.clone()])
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    let rows = client
        .query(
            "SELECT count(*) FROM accounts \
             WHERE user_id = 1",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows[0].get::<_, i64>(0), 1);
}

#[tokio::test]
#[ignore]
async fn advisory_lock_exclusive() {
    let (_c, client) = pg_client().await;
    acquire_advisory_lock(&client, 0).await.unwrap();
    // Second lock on same connection succeeds
    // (pg_advisory_lock is reentrant per session)
    acquire_advisory_lock(&client, 0).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn replay_from_wal_rebuilds_positions() {
    use rsx_dxs::FillRecord;
    use rsx_dxs::WalWriter;

    let wal_dir =
        tempfile::tempdir().unwrap();
    let wal_path = wal_dir.path();

    // Write 3 fills for symbol 0
    let mut writer = WalWriter::new(
        0,
        wal_path,
        None,
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();
    for i in 1..=3u64 {
        let mut fill = FillRecord {
            seq: i,
            ts_ns: 1000 + i,
            symbol_id: 0,
            taker_user_id: 0,
            maker_user_id: 2,
            _pad0: 0,
            taker_order_id_hi: 0,
            taker_order_id_lo: 0,
            maker_order_id_hi: 0,
            maker_order_id_lo: 0,
            price: rsx_types::Price(5000),
            qty: rsx_types::Qty(10),
            taker_side: 0,
            reduce_only: 0,
            tif: 0,
            post_only: 0,
            _pad1: [0; 4],
        };
        writer.append(&mut fill).unwrap();
    }
    writer.flush().unwrap();

    let config = ShardConfig {
        shard_id: 0,
        shard_count: 1,
        max_symbols: 4,
        symbol_params: vec![
            SymbolRiskParams {
                initial_margin_rate: 1000,
                maintenance_margin_rate: 500,
                max_leverage: 10,
            };
            4
        ],
        taker_fee_bps: vec![5; 4],
        maker_fee_bps: vec![-1; 4],
        funding_config: FundingConfig::default(),
        liquidation_config:
            LiquidationConfig::default(),
        replication_config:
            ReplicationConfig::default(),
    };
    let mut shard = RiskShard::new(config);
    // Give user 0 collateral so account exists
    shard.accounts.insert(
        0,
        Account::new(0, 1_000_000),
    );
    shard.accounts.insert(
        2,
        Account::new(2, 1_000_000),
    );

    let replayed =
        replay_from_wal(&mut shard, wal_path, &[0])
            .unwrap();
    assert_eq!(replayed, 3);
    assert_eq!(shard.tips[0], 3);

    // Taker (user 0) bought 30 total
    let pos = &shard.positions[&(0, 0)];
    assert_eq!(pos.long_qty, 30);

    // Maker (user 2) sold 30 total
    let pos = &shard.positions[&(2, 0)];
    assert_eq!(pos.short_qty, 30);
}

#[tokio::test]
#[ignore]
async fn replay_from_wal_releases_frozen_on_order_done() {
    use rsx_dxs::OrderDoneRecord;
    use rsx_dxs::WalWriter;
    use rsx_risk::types::OrderRequest;

    let wal_dir = tempfile::tempdir().unwrap();
    let wal_path = wal_dir.path();

    let mut writer = WalWriter::new(
        0,
        wal_path,
        None,
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();

    let config = ShardConfig {
        shard_id: 0,
        shard_count: 1,
        max_symbols: 4,
        symbol_params: vec![
            SymbolRiskParams {
                initial_margin_rate: 1000,
                maintenance_margin_rate: 500,
                max_leverage: 10,
            };
            4
        ],
        taker_fee_bps: vec![5; 4],
        maker_fee_bps: vec![-1; 4],
        funding_config: FundingConfig::default(),
        liquidation_config:
            LiquidationConfig::default(),
        replication_config:
            ReplicationConfig::default(),
    };
    let mut shard = RiskShard::new(config);
    shard.accounts.insert(0, Account::new(0, 1_000_000));
    shard.mark_prices[0] = 10_000;

    let order = OrderRequest {
        seq: 0,
        user_id: 0,
        symbol_id: 0,
        price: 10_000,
        qty: 10,
        order_id_hi: 55,
        order_id_lo: 77,
        timestamp_ns: 1_000,
        side: 0,
        tif: 0,
        reduce_only: false,
        post_only: false,
        is_liquidation: false,
        _pad: [0; 3],
    };
    let _ = shard.process_order(&order);
    assert!(shard.accounts[&0].frozen_margin > 0);

    let mut done = OrderDoneRecord {
        seq: 1,
        ts_ns: 2_000,
        symbol_id: 0,
        user_id: 0,
        order_id_hi: 55,
        order_id_lo: 77,
        filled_qty: rsx_types::Qty(0),
        remaining_qty: rsx_types::Qty(10),
        final_status: 2,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };
    writer.append(&mut done).unwrap();
    writer.flush().unwrap();

    let _ = replay_from_wal(&mut shard, wal_path, &[0]).unwrap();
    assert_eq!(shard.accounts[&0].frozen_margin, 0);
}

#[tokio::test]
#[ignore]
async fn persist_fills_partitioning_by_symbol() {
    let (_c, mut client) = pg_client().await;
    let fills: Vec<PersistFill> = (0..3)
        .flat_map(|sym| {
            (0..4).map(move |i| PersistFill {
                symbol_id: sym,
                taker_user_id: 1,
                maker_user_id: 2,
                price: 1000,
                qty: 10,
                taker_fee: 0,
                maker_fee: 0,
                taker_side: 0,
                seq: i,
                timestamp_ns: 1000 + i,
            })
        })
        .collect();
    let tx = client.transaction().await.unwrap();
    insert_fills(&tx, &fills).await.unwrap();
    tx.commit().await.unwrap();

    // Query per symbol
    for sym in 0..3i32 {
        let rows = client
            .query(
                "SELECT count(*) FROM fills \
                 WHERE symbol_id = $1",
                &[&sym],
            )
            .await
            .unwrap();
        assert_eq!(rows[0].get::<_, i64>(0), 4);
    }
}

#[tokio::test]
#[ignore]
async fn persist_backpressure_ring_full() {
    // Create a tiny ring (capacity 2)
    let (mut producer, _consumer) =
        rtrb::RingBuffer::<PersistEvent>::new(2);

    // Fill the ring
    producer
        .push(PersistEvent::Tip {
            symbol_id: 0,
            seq: 1,
        })
        .unwrap();
    producer
        .push(PersistEvent::Tip {
            symbol_id: 0,
            seq: 2,
        })
        .unwrap();

    // Third push should fail (ring full)
    let result = producer.push(PersistEvent::Tip {
        symbol_id: 0,
        seq: 3,
    });
    assert!(result.is_err());
}

