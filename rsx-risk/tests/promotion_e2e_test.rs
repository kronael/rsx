//! E2E coverage for the replica → main promotion path.
//! These tests pin the observable contract for
//! `main.rs::run_replica` / `run_main`:
//!
//! 1. A replica polling `pg_try_advisory_lock` flips to
//!    "promoted" the first poll after the main session
//!    releases the lock.
//! 2. After promotion, the new main can re-acquire the lock
//!    (blocking-acquire path used in `run_main`) and reload
//!    its state from Postgres with a fresh in-memory shard.
//!    Fills processed after that update positions correctly.
//! 3. A second replica racing the first cannot grab the lock
//!    while the new main holds it (advisory-lock exclusivity).
//!
//! Tests use a testcontainer Postgres and a polling loop that
//! mirrors `run_replica`'s lock-poll cadence (500ms).

use rsx_risk::account::Account;
use rsx_risk::config::LiquidationConfig;
use rsx_risk::config::ReplicationConfig;
use rsx_risk::config::ShardConfig;
use rsx_risk::funding::FundingConfig;
use rsx_risk::lease::AdvisoryLease;
use rsx_risk::margin::SymbolRiskParams;
use rsx_risk::replay::load_from_postgres;
use rsx_risk::schema::run_migrations;
use rsx_risk::shard::RiskShard;
use rsx_risk::types::FillEvent;
use std::time::Duration;
use std::time::Instant;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio_postgres::Client;
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
            lease_poll_interval_ms: 50,
            lease_renew_interval_ms: 1000,
            replica_sync_ring_size: 1024,
        },
    }
}

async fn connect(
    connstr: &str,
) -> Client {
    let (client, conn) =
        tokio_postgres::connect(connstr, NoTls)
            .await
            .expect("pg connect");
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("pg conn error: {e}");
        }
    });
    client
}

async fn setup_pg() -> (
    testcontainers::ContainerAsync<Postgres>,
    String,
) {
    let container: testcontainers::ContainerAsync<Postgres> =
        Postgres::default().start().await.unwrap();
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .unwrap();
    let connstr = format!(
        "host=localhost port={port} user=postgres \
         password=postgres dbname=postgres"
    );
    let migrate_client = connect(&connstr).await;
    run_migrations(&migrate_client).await.unwrap();
    drop(migrate_client);
    (container, connstr)
}

/// Seed accounts so a promoted-main reload from Postgres has
/// non-empty state to verify against.
async fn seed_account(
    client: &Client,
    user_id: u32,
    collateral: i64,
) {
    client
        .execute(
            "INSERT INTO accounts (user_id, collateral, version) \
             VALUES ($1, $2, 1) \
             ON CONFLICT (user_id) DO UPDATE \
             SET collateral = EXCLUDED.collateral",
            &[&(user_id as i32), &collateral],
        )
        .await
        .expect("seed account");
}

/// Drive the replica's lock-poll loop. Returns `true` on
/// promotion (lock acquired), `false` on timeout.
///
/// Mirrors `main.rs::run_replica`'s polling behavior without
/// depending on its current `set_var`+recursion shape, so this
/// test survives the planned T3.2 refactor.
async fn poll_until_promoted(
    lease: &mut AdvisoryLease,
    client: &Client,
    poll_ms: u64,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let acquired = lease
            .try_acquire(client)
            .await
            .expect("try_acquire");
        if acquired {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(poll_ms))
            .await;
    }
    false
}

/// Phase 1 anchor: replica observes main lock release,
/// promotes, then a fresh shard (the new main) loads state
/// from Postgres and processes a fill.
///
/// This is the *exact* observable contract `run_replica` →
/// `run_main` must preserve across the T3.2 refactor.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore] // testcontainer; runs under `make integration`
async fn replica_promotes_on_main_release_and_processes_fill() {
    let (_container, connstr) = setup_pg().await;

    let main_client = connect(&connstr).await;
    let replica_client = connect(&connstr).await;
    let promoted_main_client = connect(&connstr).await;

    seed_account(&main_client, 100, 1000_0000_0000).await;

    // shard_id must match `test_config().shard_id` so the
    // load_from_postgres routing predicate (`user_id %
    // shard_count == shard_id`) selects the seeded account.
    let shard_id = 0;

    // Main acquires the lock (mirrors run_main startup).
    let mut main_lease = AdvisoryLease::new(shard_id);
    main_lease
        .acquire(&main_client)
        .await
        .expect("main acquire");
    assert!(main_lease.is_acquired());

    // Replica starts polling; lock is held so it must wait.
    let mut replica_lease = AdvisoryLease::new(shard_id);
    let acquired_while_main_alive = replica_lease
        .try_acquire(&replica_client)
        .await
        .expect("try_acquire");
    assert!(
        !acquired_while_main_alive,
        "replica must not acquire while main holds lock"
    );

    // Replica buffers some fills before promotion (this is
    // what `buffer_fill_for_replica` does on the hot path).
    let mut replica_shard =
        RiskShard::new(test_config(true));
    for seq in 1..=5 {
        replica_shard.buffer_fill_for_replica(FillEvent {
            seq,
            symbol_id: 0,
            taker_user_id: 100,
            maker_user_id: 200,
            price: 50000_0000,
            qty: 10_0000,
            taker_side: 0,
            timestamp_ns: seq * 1_000,
        });
    }
    assert_eq!(replica_shard.replica_buffered_count(), 5);

    // Simulate main crash: drop its session. PG releases
    // session-scoped advisory locks automatically.
    // `main_lease` falls out of scope below; AdvisoryLease has
    // no Drop impl, so an explicit `drop` would be a no-op.
    drop(main_client);
    let _ = main_lease;

    // Replica polls and observes the release within timeout.
    let promoted = poll_until_promoted(
        &mut replica_lease,
        &replica_client,
        50,
        Duration::from_secs(5),
    )
    .await;
    assert!(promoted, "replica must promote within 5s");
    assert!(replica_lease.is_acquired());

    // Apply locally-buffered fills (what
    // `shard.promote_from_replica()` does today — the
    // refactor must keep this call).
    let applied = replica_shard.promote_from_replica();
    // No tip sync was applied; nothing should drain.
    assert_eq!(applied.len(), 0);

    // Release the replica's lock so the new main can grab it
    // via the blocking `acquire` path used in `run_main`.
    replica_lease
        .release(&replica_client)
        .await
        .expect("replica release");

    // New "main" boots: acquires lock (blocking), reloads
    // state from Postgres, processes a fresh fill.
    let mut new_main_lease = AdvisoryLease::new(shard_id);
    new_main_lease
        .acquire(&promoted_main_client)
        .await
        .expect("new main acquire");
    assert!(new_main_lease.is_acquired());

    let state = load_from_postgres(
        &promoted_main_client,
        shard_id,
        1,
        4,
    )
    .await
    .expect("load_from_postgres");
    let mut new_main_shard =
        RiskShard::new(test_config(false));
    new_main_shard.load_state(state);

    // Verify the reload picked up the seeded account.
    let acct = new_main_shard.accounts.get(&100);
    assert!(
        acct.is_some(),
        "promoted main must reload seeded accounts"
    );
    assert_eq!(acct.unwrap().collateral, 1000_0000_0000);

    // Process a fill against the new main — proves it's a
    // working main, not just a process holding a lock.
    let fill = FillEvent {
        seq: 1,
        symbol_id: 0,
        taker_user_id: 100,
        maker_user_id: 200,
        price: 50000_0000,
        qty: 10_0000,
        taker_side: 0,
        timestamp_ns: 1_000,
    };
    new_main_shard.process_fill(&fill);
    let pos = new_main_shard
        .positions
        .get(&(100u32, 0u32))
        .expect("position created");
    assert_eq!(pos.long_qty, 10_0000);
    assert_eq!(new_main_shard.tips[0], 1);
}

/// Two replicas race for promotion. Only one wins (advisory
/// lock exclusivity); the loser keeps polling. Pins Invariant
/// #10 "at most one main per shard" under contention.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore] // testcontainer
async fn split_brain_prevented_under_promotion_race() {
    let (_container, connstr) = setup_pg().await;

    let main_client = connect(&connstr).await;
    let replica_a_client = connect(&connstr).await;
    let replica_b_client = connect(&connstr).await;

    let shard_id = 8;

    let mut main_lease = AdvisoryLease::new(shard_id);
    main_lease
        .acquire(&main_client)
        .await
        .expect("main acquire");

    let mut replica_a_lease = AdvisoryLease::new(shard_id);
    let mut replica_b_lease = AdvisoryLease::new(shard_id);

    // Both replicas blocked.
    assert!(
        !replica_a_lease
            .try_acquire(&replica_a_client)
            .await
            .unwrap()
    );
    assert!(
        !replica_b_lease
            .try_acquire(&replica_b_client)
            .await
            .unwrap()
    );

    drop(main_client);
    let _ = main_lease;

    // Race them. Exactly one must win.
    let a = poll_until_promoted(
        &mut replica_a_lease,
        &replica_a_client,
        25,
        Duration::from_secs(5),
    );
    let b = poll_until_promoted(
        &mut replica_b_lease,
        &replica_b_client,
        25,
        Duration::from_secs(5),
    );
    let (a, b) = tokio::join!(a, b);
    assert!(a || b, "at least one replica must promote");
    assert!(
        !(a && b),
        "exactly one replica may promote (lock exclusivity)"
    );

    // The winner holds the lock; the loser, polling again,
    // must still see it as held.
    let winner_acquired = replica_a_lease.is_acquired();
    let loser_client = if winner_acquired {
        &replica_b_client
    } else {
        &replica_a_client
    };
    let loser_lease = if winner_acquired {
        &mut replica_b_lease
    } else {
        &mut replica_a_lease
    };
    let still_blocked = loser_lease
        .try_acquire(loser_client)
        .await
        .expect("loser try_acquire");
    assert!(
        !still_blocked,
        "loser must remain blocked while winner holds lock"
    );
}

/// Promoted main loses its lease (PG session dropped) and the
/// next-tick renew check returns `false`. Pins the contract
/// `run_main` relies on to exit-for-restart, which the
/// state-machine refactor will surface as a `Demote` /
/// `Shutdown` transition rather than an `Err` return.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore] // testcontainer
async fn promoted_main_detects_lease_loss_via_renew() {
    let (_container, connstr) = setup_pg().await;

    // Use one client we control; we'll simulate session loss
    // by dropping the client *and* the lease so the next
    // renew on a fresh client returns held=false.
    let main_client = connect(&connstr).await;
    let shard_id = 9;

    let mut lease = AdvisoryLease::new(shard_id);
    lease
        .acquire(&main_client)
        .await
        .expect("main acquire");
    assert!(
        lease.renew(&main_client).await.expect("renew"),
        "freshly-acquired lease must report held"
    );

    // Simulate session loss: drop the client. The pg backend
    // releases the advisory lock. A fresh client's `renew`
    // (querying pg_locks for *this* backend) must return
    // false.
    drop(main_client);

    let probe_client = connect(&connstr).await;
    let held = lease
        .renew(&probe_client)
        .await
        .expect("renew after session loss");
    assert!(
        !held,
        "renew on fresh session must report lease lost"
    );
}

/// In-memory parity check that survives the refactor: a
/// replica that has buffered fills + applied tips matches
/// main's positions exactly. This was already covered by
/// `replication_e2e_test.rs::replica_stays_in_sync_with_main_via_tip_sync`
/// but is re-asserted here against the promotion path
/// (post-promote, the buffered+tipped state is consistent).
#[tokio::test]
async fn promoted_replica_state_matches_main_positions() {
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
            timestamp_ns: 1_000,
        },
        FillEvent {
            seq: 2,
            symbol_id: 0,
            taker_user_id: 100,
            maker_user_id: 200,
            price: 50100_0000,
            qty: 5_0000,
            taker_side: 0,
            timestamp_ns: 2_000,
        },
    ];

    // Seed both with a funded account so process_fill creates
    // positions, not silently drops.
    main_shard
        .accounts
        .insert(100, Account::new(100, 1000_0000_0000));
    main_shard
        .accounts
        .insert(200, Account::new(200, 1000_0000_0000));
    replica_shard
        .accounts
        .insert(100, Account::new(100, 1000_0000_0000));
    replica_shard
        .accounts
        .insert(200, Account::new(200, 1000_0000_0000));

    for fill in &fills {
        main_shard.process_fill(fill);
        replica_shard.buffer_fill_for_replica(fill.clone());
    }

    replica_shard.apply_tip_from_main(0, 2);
    let drained = replica_shard.promote_from_replica();
    // tip-driven application already happened in
    // apply_tip_from_main; promote drains residual (none).
    assert_eq!(drained.len(), 0);
    assert_eq!(replica_shard.replica_buffered_count(), 0);

    // Both sides agree on the position.
    let m = main_shard
        .positions
        .get(&(100u32, 0u32))
        .expect("main pos");
    let r = replica_shard
        .positions
        .get(&(100u32, 0u32))
        .expect("replica pos");
    assert_eq!(m.long_qty, r.long_qty);
    assert_eq!(m.short_qty, r.short_qty);
    assert_eq!(m.realized_pnl, r.realized_pnl);
}
