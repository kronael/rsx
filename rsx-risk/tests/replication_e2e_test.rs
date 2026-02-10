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
use rsx_risk::types::FillEvent;
use rustc_hash::FxHashMap;
use std::time::Duration;
use tokio_postgres::NoTls;

fn test_config(is_replica: bool) -> ShardConfig {
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
            is_replica,
            lease_poll_interval_ms: 500,
            lease_renew_interval_ms: 1000,
            replica_sync_ring_size: 1024,
        },
    }
}

#[tokio::test]
async fn replica_stays_in_sync_with_main_via_tip_sync() {
    let mut main_shard = RiskShard::new(test_config(false));
    let mut replica_shard =
        RiskShard::new(test_config(true));

    let fills = vec![
        FillEvent {
            seq: 1,
            symbol_id: 0,
            taker_user_id: 100,
            maker_user_id: 200,
            price: 50000_0000,
            qty: 10_0000,
            taker_side: 0,
            timestamp_ns: 1000,
        },
        FillEvent {
            seq: 2,
            symbol_id: 0,
            taker_user_id: 100,
            maker_user_id: 200,
            price: 50001_0000,
            qty: 5_0000,
            taker_side: 1,
            timestamp_ns: 2000,
        },
        FillEvent {
            seq: 3,
            symbol_id: 1,
            taker_user_id: 100,
            maker_user_id: 200,
            price: 30000_0000,
            qty: 20_0000,
            taker_side: 0,
            timestamp_ns: 3000,
        },
    ];

    for fill in &fills {
        main_shard.process_fill(fill);
        replica_shard.buffer_fill_for_replica(fill.clone());
    }

    assert_eq!(main_shard.tips[0], 2);
    assert_eq!(main_shard.tips[1], 3);
    assert_eq!(replica_shard.replica_buffered_count(), 3);

    replica_shard.apply_tip_from_main(0, 2);
    replica_shard.apply_tip_from_main(1, 3);

    assert_eq!(replica_shard.replica_buffered_count(), 0);
    assert_eq!(replica_shard.tips[0], 2);
    assert_eq!(replica_shard.tips[1], 3);

    let main_pos = main_shard
        .positions
        .get(&(100u32, 0u32))
        .unwrap();
    let replica_pos = replica_shard
        .positions
        .get(&(100u32, 0u32))
        .unwrap();

    assert_eq!(main_pos.long_qty, replica_pos.long_qty);
    assert_eq!(main_pos.short_qty, replica_pos.short_qty);
    assert_eq!(
        main_pos.realized_pnl,
        replica_pos.realized_pnl
    );
}

#[tokio::test]
async fn replica_buffers_fills_ahead_of_tip_sync() {
    let mut replica_shard =
        RiskShard::new(test_config(true));

    for seq in 1..=10 {
        replica_shard.buffer_fill_for_replica(FillEvent {
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

    assert_eq!(replica_shard.replica_buffered_count(), 10);

    replica_shard.apply_tip_from_main(0, 5);
    assert_eq!(replica_shard.replica_buffered_count(), 5);
    assert_eq!(replica_shard.tips[0], 5);

    replica_shard.apply_tip_from_main(0, 10);
    assert_eq!(replica_shard.replica_buffered_count(), 0);
    assert_eq!(replica_shard.tips[0], 10);
}

#[tokio::test]
async fn replica_promotion_no_data_loss() {
    let mut replica_shard =
        RiskShard::new(test_config(true));

    // Add 100 fills for symbol 0
    for seq in 1..=100 {
        replica_shard.buffer_fill_for_replica(FillEvent {
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

    // Add 50 fills for symbol 1
    for seq in 1..=50 {
        replica_shard.buffer_fill_for_replica(FillEvent {
            seq,
            symbol_id: 1,
            taker_user_id: 300,
            maker_user_id: 400,
            price: 40000_0000,
            qty: 5_0000,
            taker_side: 1,
            timestamp_ns: seq * 1000,
        });
    }

    // Add 25 more fills for symbol 2 (won't get tip applied)
    for seq in 1..=25 {
        replica_shard.buffer_fill_for_replica(FillEvent {
            seq,
            symbol_id: 2,
            taker_user_id: 500,
            maker_user_id: 600,
            price: 30000_0000,
            qty: 3_0000,
            taker_side: 0,
            timestamp_ns: seq * 1000,
        });
    }

    // Apply tips for symbols 0 and 1 only
    replica_shard.apply_tip_from_main(0, 100);
    replica_shard.apply_tip_from_main(1, 50);

    // Symbol 2 should still have 25 buffered fills
    let fills_before = replica_shard.replica_buffered_count();
    assert_eq!(fills_before, 25);

    // Promotion should apply nothing (no new tips)
    let applied_fills = replica_shard.promote_from_replica();
    assert_eq!(applied_fills.len(), 0);

    // Verify positions from applied fills
    let pos0 = replica_shard.positions.get(&(100u32, 0u32));
    assert!(pos0.is_some());
    assert_eq!(pos0.unwrap().long_qty, 100 * 10_0000);

    let pos1 = replica_shard.positions.get(&(300u32, 1u32));
    assert!(pos1.is_some());
    assert_eq!(pos1.unwrap().short_qty, 50 * 5_0000);
}

#[tokio::test]
async fn promotion_invariant_only_up_to_last_tip() {
    let mut replica_shard =
        RiskShard::new(test_config(true));

    for seq in 1..=20 {
        replica_shard.buffer_fill_for_replica(FillEvent {
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

    replica_shard.apply_tip_from_main(0, 15);

    assert_eq!(replica_shard.replica_buffered_count(), 5);

    let fills = replica_shard.promote_from_replica();
    assert_eq!(fills.len(), 0);

    assert_eq!(replica_shard.tips[0], 15);

    let pos = replica_shard.positions.get(&(100u32, 0u32));
    assert!(pos.is_some());
    assert_eq!(pos.unwrap().long_qty, 15 * 10_0000);
}

#[tokio::test]
async fn multi_symbol_fill_interleaving_with_replica() {
    let mut main_shard = RiskShard::new(test_config(false));
    let mut replica_shard =
        RiskShard::new(test_config(true));

    for i in 0..50 {
        let symbol_id = (i % 4) as u32;
        let seq = (i / 4) + 1;
        let fill = FillEvent {
            seq,
            symbol_id,
            taker_user_id: 100,
            maker_user_id: 200,
            price: 50000_0000 + (i as i64 * 100),
            qty: 10_0000,
            taker_side: (i % 2) as u8,
            timestamp_ns: i as u64 * 1000,
        };
        main_shard.process_fill(&fill);
        replica_shard.buffer_fill_for_replica(fill);
    }

    for symbol_id in 0..4 {
        let tip = main_shard.tips[symbol_id];
        replica_shard.apply_tip_from_main(symbol_id as u32, tip);
    }

    assert_eq!(replica_shard.replica_buffered_count(), 0);

    for symbol_id in 0..4 {
        assert_eq!(
            main_shard.tips[symbol_id],
            replica_shard.tips[symbol_id]
        );
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
async fn replica_detects_main_crash_via_lock_poll() {
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

    let (client_replica, conn_replica) =
        tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("failed to connect");
    tokio::spawn(async move {
        let _ = conn_replica.await;
    });

    let shard_id = 201;

    let mut lease_main = AdvisoryLease::new(shard_id);
    lease_main
        .acquire(&client_main)
        .await
        .expect("main acquire");

    let mut lease_replica = AdvisoryLease::new(shard_id);
    let acquired = lease_replica
        .try_acquire(&client_replica)
        .await
        .expect("replica try");
    assert!(!acquired);

    drop(client_main);
    drop(lease_main);

    tokio::time::sleep(Duration::from_millis(200)).await;

    let acquired = lease_replica
        .try_acquire(&client_replica)
        .await
        .expect("replica acquire after crash");
    assert!(acquired);
}

#[tokio::test]
#[ignore]
async fn both_crash_cold_start_from_postgres() {
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

    let _shard_id = 202;
    let _max_symbols = 4;

    let mut shard = RiskShard::new(test_config(false));

    let mut accounts = FxHashMap::default();
    accounts.insert(
        100u32,
        Account {
            user_id: 100,
            collateral: 1000_0000_0000,
            frozen_margin: 0,
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
async fn replica_applies_fills_in_seq_order() {
    let mut replica_shard =
        RiskShard::new(test_config(true));

    let seqs = vec![5, 3, 1, 4, 2];
    for &seq in &seqs {
        replica_shard.buffer_fill_for_replica(FillEvent {
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

    replica_shard.apply_tip_from_main(0, 5);

    let pos = replica_shard.positions.get(&(100u32, 0u32));
    assert!(pos.is_some());
    assert_eq!(pos.unwrap().long_qty, 5 * 10_0000);
    assert_eq!(pos.unwrap().last_fill_seq, 5);
}

#[tokio::test]
async fn replica_handles_seq_gaps() {
    let mut replica_shard =
        RiskShard::new(test_config(true));

    let seqs = vec![1, 2, 5, 6, 10];
    for &seq in &seqs {
        replica_shard.buffer_fill_for_replica(FillEvent {
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

    replica_shard.apply_tip_from_main(0, 10);

    let pos = replica_shard.positions.get(&(100u32, 0u32));
    assert!(pos.is_some());
    assert_eq!(pos.unwrap().long_qty, 5 * 10_0000);
    assert_eq!(replica_shard.tips[0], 10);
}

#[tokio::test]
async fn replica_dedup_on_duplicate_fills() {
    let mut replica_shard =
        RiskShard::new(test_config(true));

    let fill = FillEvent {
        seq: 1,
        symbol_id: 0,
        taker_user_id: 100,
        maker_user_id: 200,
        price: 50000_0000,
        qty: 10_0000,
        taker_side: 0,
        timestamp_ns: 1000,
    };

    replica_shard.buffer_fill_for_replica(fill.clone());
    replica_shard.buffer_fill_for_replica(fill.clone());
    replica_shard.buffer_fill_for_replica(fill);

    assert_eq!(replica_shard.replica_buffered_count(), 1);

    replica_shard.apply_tip_from_main(0, 1);

    let pos = replica_shard.positions.get(&(100u32, 0u32));
    assert!(pos.is_some());
    assert_eq!(pos.unwrap().long_qty, 10_0000);
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
