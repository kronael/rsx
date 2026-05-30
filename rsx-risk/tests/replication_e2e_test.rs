//! Leader-election + crash-recovery integration tests.
//!
//! After the replica-mode deletion, every risk-shard process is
//! a candidate main: it blocks on `pg_advisory_lock(shard_id)`,
//! then loads PG state + replays WAL. These tests cover the two
//! durable guarantees that protocol rests on:
//!   - invariant #10: the advisory lock is exclusive (at most one
//!     main per shard; the next process acquires only after the
//!     holder's session drops).
//!   - crash recovery: a cold-started shard rebuilds its state
//!     from a Postgres `ColdStartState` snapshot.
//!
//! The advisory-lock tests are `#[ignore]`d (need a live PG; run
//! under `make integration`).

use rsx_risk::account::Account;
use rsx_risk::config::LiquidationConfig;
use rsx_risk::config::ReplicationConfig;
use rsx_risk::config::ShardConfig;
use rsx_risk::funding::FundingConfig;
use rsx_risk::lease::AdvisoryLease;
use rsx_risk::margin::SymbolRiskParams;
use rsx_risk::position::Position;
use rsx_risk::replay::ColdStartState;
use rsx_risk::shard::RiskShard;
use rustc_hash::FxHashMap;
use std::time::Duration;
use tokio_postgres::NoTls;

fn test_config() -> ShardConfig {
    ShardConfig {
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
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig {
            lease_poll_interval_ms: 500,
            lease_renew_interval_ms: 1000,
        },
    }
}

#[tokio::test]
#[ignore]
async fn lease_renewal_keeps_main_alive() {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| {
            "postgresql://postgres@localhost/rsx_test".into()
        });

    let (client, conn) =
        tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("failed to connect");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let shard_id = 200;
    let mut lease = AdvisoryLease::new(shard_id);
    lease.acquire(&client).await.expect("acquire");

    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(500))
            .await;
        let held = lease.renew(&client).await.expect("renew");
        assert!(held);
    }

    lease.release(&client).await.expect("release");
}

#[tokio::test]
#[ignore]
async fn standby_detects_main_crash_via_lock() {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| {
            "postgresql://postgres@localhost/rsx_test".into()
        });

    let (client_main, conn_main) =
        tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("failed to connect");
    tokio::spawn(async move {
        let _ = conn_main.await;
    });

    let (client_standby, conn_standby) =
        tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("failed to connect");
    tokio::spawn(async move {
        let _ = conn_standby.await;
    });

    let shard_id = 201;

    let mut lease_main = AdvisoryLease::new(shard_id);
    lease_main
        .acquire(&client_main)
        .await
        .expect("main acquire");

    let mut lease_standby = AdvisoryLease::new(shard_id);
    let acquired = lease_standby
        .try_acquire(&client_standby)
        .await
        .expect("standby try");
    assert!(!acquired);

    drop(client_main);
    drop(lease_main);

    tokio::time::sleep(Duration::from_millis(200)).await;

    let acquired = lease_standby
        .try_acquire(&client_standby)
        .await
        .expect("standby acquire after crash");
    assert!(acquired);
}

#[tokio::test]
#[ignore]
async fn cold_start_from_postgres() {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| {
            "postgresql://postgres@localhost/rsx_test".into()
        });

    let (_client, conn) =
        tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("failed to connect");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let mut shard = RiskShard::new(test_config());

    let mut accounts = FxHashMap::default();
    accounts.insert(
        100u32,
        Account {
            user_id: 100,
            collateral: 1000_0000_0000,
            version: 1,
        },
    );

    let mut positions = FxHashMap::default();
    positions.insert(
        (100u32, 0u32),
        Position {
            user_id: 100,
            symbol_id: 0,
            long_qty: 100_0000,
            short_qty: 0,
            long_entry_cost: 50000_0000 * 100_0000,
            short_entry_cost: 0,
            realized_pnl: 0,
            last_fill_seq: 10,
            version: 5,
        },
    );

    let tips = vec![10u64, 0, 0, 0];

    let state = ColdStartState {
        accounts,
        positions,
        insurance_funds: FxHashMap::default(),
        tips,
        frozen_orders: FxHashMap::default(),
    };

    shard.load_state(state);

    assert_eq!(shard.tips[0], 10);
    assert_eq!(shard.accounts.len(), 1);
    assert_eq!(shard.positions.len(), 1);

    let pos = shard.positions.get(&(100u32, 0u32)).unwrap();
    assert_eq!(pos.long_qty, 100_0000);
    assert_eq!(pos.last_fill_seq, 10);
}

#[tokio::test]
#[ignore]
async fn split_brain_prevented_by_advisory_lock() {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| {
            "postgresql://postgres@localhost/rsx_test".into()
        });

    let (client1, conn1) =
        tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("failed to connect");
    tokio::spawn(async move {
        let _ = conn1.await;
    });

    let (client2, conn2) =
        tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("failed to connect");
    tokio::spawn(async move {
        let _ = conn2.await;
    });

    let shard_id = 203;

    let mut lease1 = AdvisoryLease::new(shard_id);
    lease1.acquire(&client1).await.expect("lease1");

    let mut lease2 = AdvisoryLease::new(shard_id);
    let acquired =
        lease2.try_acquire(&client2).await.expect("try");
    assert!(!acquired);

    for _ in 0..10 {
        let acquired = lease2
            .try_acquire(&client2)
            .await
            .expect("try");
        assert!(!acquired);
    }

    lease1.release(&client1).await.expect("release");
}
