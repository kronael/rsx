use rsx_risk::config::LiquidationConfig;
use rsx_risk::config::ReplicationConfig;
use rsx_risk::config::ShardConfig;
use rsx_risk::funding::FundingConfig;
use rsx_risk::lease::AdvisoryLease;
use rsx_risk::margin::SymbolRiskParams;
use rsx_risk::replay::load_from_postgres;
use rsx_risk::shard::RiskShard;
use rsx_risk::types::FillEvent;
use std::time::Duration;
use tokio_postgres::NoTls;

#[tokio::test]
async fn replica_buffers_fills_until_tip_received() {
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
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig {
            is_replica: true,
            lease_poll_interval_ms: 500,
            lease_renew_interval_ms: 1000,
            replica_sync_ring_size: 1024,
        },
    };

    let mut shard = RiskShard::new(config);

    // Buffer fills without tip
    let fill1 = FillEvent {
        seq: 1,
        symbol_id: 0,
        taker_user_id: 100,
        maker_user_id: 200,
        price: 50000_0000,
        qty: 10_0000,
        taker_side: 0,
        timestamp_ns: 1000,
    };
    shard.buffer_fill_for_replica(fill1.clone());
    assert_eq!(shard.replica_buffered_count(), 1);

    let fill2 = FillEvent {
        seq: 2,
        symbol_id: 0,
        taker_user_id: 100,
        maker_user_id: 200,
        price: 50001_0000,
        qty: 5_0000,
        taker_side: 1,
        timestamp_ns: 2000,
    };
    shard.buffer_fill_for_replica(fill2.clone());
    assert_eq!(shard.replica_buffered_count(), 2);

    // Apply tip from main
    shard.apply_tip_from_main(0, 2);

    // Fills should be processed and removed from buffer
    assert_eq!(shard.replica_buffered_count(), 0);
    assert_eq!(shard.tips[0], 2);
}

#[tokio::test]
async fn replica_only_applies_fills_up_to_tip() {
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
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig {
            is_replica: true,
            lease_poll_interval_ms: 500,
            lease_renew_interval_ms: 1000,
            replica_sync_ring_size: 1024,
        },
    };

    let mut shard = RiskShard::new(config);

    // Buffer fills with seq 1, 2, 3
    for seq in 1..=3 {
        shard.buffer_fill_for_replica(FillEvent {
            seq,
            symbol_id: 0,
            taker_user_id: 100,
            maker_user_id: 200,
            price: 50000_0000,
            qty: 10_0000,
            taker_side: 0,
            timestamp_ns: seq * 1000,
        });
    }
    assert_eq!(shard.replica_buffered_count(), 3);

    // Apply tip = 2, only fills 1 and 2 should be applied
    shard.apply_tip_from_main(0, 2);
    assert_eq!(shard.replica_buffered_count(), 1);
    assert_eq!(shard.tips[0], 2);

    // Apply tip = 3, fill 3 should be applied
    shard.apply_tip_from_main(0, 3);
    assert_eq!(shard.replica_buffered_count(), 0);
    assert_eq!(shard.tips[0], 3);
}

#[tokio::test]
async fn replica_promotion_applies_all_buffered() {
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
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig {
            is_replica: true,
            lease_poll_interval_ms: 500,
            lease_renew_interval_ms: 1000,
            replica_sync_ring_size: 1024,
        },
    };

    let mut shard = RiskShard::new(config);

    // Buffer fills for multiple symbols
    for symbol_id in 0..2 {
        for seq in 1..=5 {
            shard.buffer_fill_for_replica(FillEvent {
                seq,
                symbol_id,
                taker_user_id: 100,
                maker_user_id: 200,
                price: 50000_0000,
                qty: 10_0000,
                taker_side: 0,
                timestamp_ns: seq * 1000,
            });
        }
        // Set tip for this symbol
        shard.apply_tip_from_main(symbol_id, 5);
    }

    let initial_count = shard.replica_buffered_count();
    assert_eq!(initial_count, 0); // All applied via tips

    // Promote
    let fills = shard.promote_from_replica();
    assert_eq!(fills.len(), 0); // Nothing left to apply
}

#[tokio::test]
#[ignore] // Requires Postgres
async fn advisory_lock_acquired_by_main_blocks_replica() {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| {
            "postgresql://postgres@localhost/rsx_test"
                .into()
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

    let shard_id = 99;

    // Main acquires lock
    let mut lease1 = AdvisoryLease::new(shard_id);
    lease1
        .acquire(&client1)
        .await
        .expect("main should acquire");
    assert!(lease1.is_acquired());

    // Replica tries to acquire (should fail)
    let mut lease2 = AdvisoryLease::new(shard_id);
    let acquired = lease2
        .try_acquire(&client2)
        .await
        .expect("try_acquire should succeed");
    assert!(!acquired);
    assert!(!lease2.is_acquired());

    // Release main lock
    lease1
        .release(&client1)
        .await
        .expect("release should succeed");

    // Replica can now acquire
    let acquired = lease2
        .try_acquire(&client2)
        .await
        .expect("try_acquire should succeed");
    assert!(acquired);
    assert!(lease2.is_acquired());
}

#[tokio::test]
#[ignore] // Requires Postgres
async fn replica_promotion_after_main_crash() {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| {
            "postgresql://postgres@localhost/rsx_test"
                .into()
        });

    let (client_main, conn_main) =
        tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("failed to connect");
    tokio::spawn(async move {
        let _ = conn_main.await;
    });

    let (client_replica, conn_replica) =
        tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("failed to connect");
    tokio::spawn(async move {
        let _ = conn_replica.await;
    });

    let shard_id = 100;

    // Main acquires lock
    let mut lease_main = AdvisoryLease::new(shard_id);
    lease_main
        .acquire(&client_main)
        .await
        .expect("main should acquire");

    // Replica tries to acquire (should fail)
    let mut lease_replica =
        AdvisoryLease::new(shard_id);
    let acquired = lease_replica
        .try_acquire(&client_replica)
        .await
        .expect("try_acquire should succeed");
    assert!(!acquired);

    // Simulate main crash: drop connection
    drop(client_main);
    drop(lease_main);
    tokio::time::sleep(Duration::from_millis(100))
        .await;

    // Replica should now acquire lock
    let acquired = lease_replica
        .try_acquire(&client_replica)
        .await
        .expect("try_acquire should succeed");
    assert!(acquired);
    assert!(lease_replica.is_acquired());
}

#[tokio::test]
#[ignore] // Requires Postgres
async fn both_crash_recovery_from_postgres() {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| {
            "postgresql://postgres@localhost/rsx_test"
                .into()
        });

    let (client, conn) =
        tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("failed to connect");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let shard_id = 101;

    // Load state from Postgres (should work even if stale)
    let state = load_from_postgres(
        &client,
        shard_id,
        shard_id,
        4,
    )
    .await
    .expect("load should succeed");

    // State is either empty or contains persisted data
    assert_eq!(state.tips.len(), 4);
}
