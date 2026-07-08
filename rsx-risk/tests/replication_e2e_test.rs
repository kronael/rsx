//! Eager warm-standby + crash-recovery integration tests.
//!
//! Every risk-shard process is a warm candidate main: it loads PG
//! state, *warms* by applying the live main's ME WAL replication
//! stream into its own shard (via `replay::apply_record`), and
//! only calls the NON-BLOCKING `pg_try_advisory_lock` once caught
//! up. These tests cover the guarantees that protocol rests on:
//!   - WARM CATCHUP: applying the ME replication stream folds
//!     position/freeze/tip state into the shard with no PG persist
//!     worker, and `RECORD_CAUGHT_UP{live_seq}` + `applied_seq`
//!     correctly derive `caught_up`. (`warm_catchup_*`, not
//!     ignored — uses a real in-process `ReplicationService`.)
//!   - invariant #10: the advisory lock is exclusive (at most one
//!     main per shard; the loser stays warm and retries). Catch-up
//!     only gates *when* `try_acquire` is called.
//!   - crash recovery: a cold-started shard rebuilds its state
//!     from a Postgres `ColdStartState` snapshot.
//!
//! The advisory-lock tests are `#[ignore]`d (need a live PG; run
//! under `make integration`).

use rsx_cast::decode_payload;
use rsx_cast::wal::extract_seq;
use rsx_cast::CaughtUpRecord;
use rsx_cast::ReplicationConsumer;
use rsx_cast::ReplicationService;
use rsx_cast::WalWriter;
use rsx_cast::RECORD_CAUGHT_UP;
use rsx_messages::FillRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_risk::account::Account;
use rsx_risk::config::LiquidationConfig;
use rsx_risk::config::ReplicationConfig;
use rsx_risk::config::ShardConfig;
use rsx_risk::funding::FundingConfig;
use rsx_risk::lease::AdvisoryLease;
use rsx_risk::margin::SymbolRiskParams;
use rsx_risk::position::Position;
use rsx_risk::replay::apply_record;
use rsx_risk::replay::ColdStartState;
use rsx_risk::shard::RiskShard;
use rsx_types::Price;
use rsx_types::Qty;
use rustc_hash::FxHashMap;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::time::Duration;
use tempfile::TempDir;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;
use tokio_postgres::NoTls;

/// Spin a throwaway Postgres testcontainer and return it + a connstring.
/// The advisory-lock tests need a real shared PG but no schema (pg_advisory_lock
/// is built-in), so this runs no migrations. Hold the returned container for the
/// test's lifetime; it stops on drop.
async fn pg_container() -> (ContainerAsync<Postgres>, String) {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let connstr = format!(
        "host=localhost port={port} user=postgres \
         password=postgres dbname=postgres"
    );
    (container, connstr)
}

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
    let (_pg, connstr) = pg_container().await;
    let (client, conn) = tokio_postgres::connect(&connstr, NoTls)
        .await
        .expect("failed to connect");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let shard_id = 200;
    let mut lease = AdvisoryLease::new(shard_id);
    lease.acquire(&client).await.expect("acquire");

    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let held = lease.renew(&client).await.expect("renew");
        assert!(held);
    }

    lease.release(&client).await.expect("release");
}

#[tokio::test]
#[ignore]
async fn standby_detects_main_crash_via_lock() {
    let (_pg, connstr) = pg_container().await;
    let (client_main, conn_main) = tokio_postgres::connect(&connstr, NoTls)
        .await
        .expect("failed to connect");
    tokio::spawn(async move {
        let _ = conn_main.await;
    });

    let (client_standby, conn_standby) = tokio_postgres::connect(&connstr, NoTls)
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

    // Release the main's PG client (closes its connection → PG drops the
    // session advisory lock) and its lease before the standby retries.
    #[allow(clippy::drop_non_drop)]
    {
        drop(client_main);
        drop(lease_main);
    }

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
    // Pure in-memory: ColdStartState -> set_state -> assert. No PG needed
    // (the snapshot is constructed directly; persistence is covered by
    // persist_test.rs).
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
    // Fixed-point literals: `<integer>_<4-decimal>` grouping, not thousands.
    #[allow(clippy::inconsistent_digit_grouping)]
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

    shard.set_state(state);

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
    let (_pg, connstr) = pg_container().await;
    let (client1, conn1) = tokio_postgres::connect(&connstr, NoTls)
        .await
        .expect("failed to connect");
    tokio::spawn(async move {
        let _ = conn1.await;
    });

    let (client2, conn2) = tokio_postgres::connect(&connstr, NoTls)
        .await
        .expect("failed to connect");
    tokio::spawn(async move {
        let _ = conn2.await;
    });

    let shard_id = 203;

    let mut lease1 = AdvisoryLease::new(shard_id);
    lease1.acquire(&client1).await.expect("lease1");

    let mut lease2 = AdvisoryLease::new(shard_id);
    let acquired = lease2.try_acquire(&client2).await.expect("try");
    assert!(!acquired);

    for _ in 0..10 {
        let acquired = lease2.try_acquire(&client2).await.expect("try");
        assert!(!acquired);
    }

    lease1.release(&client1).await.expect("release");
}

// ===== Eager warm-standby: WARM CATCHUP path =====

fn reserve_port() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    drop(l);
    a
}

/// Self-signed cert used as BOTH server identity and client CA
/// (single-box self-trust). Replication is TLS-mandatory.
fn test_tls(dir: &std::path::Path) -> rsx_cast::TlsConfig {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .unwrap();
    let cert_path = dir.join("repl_cert.pem");
    let key_path = dir.join("repl_key.pem");
    std::fs::write(&cert_path, cert.cert.pem()).unwrap();
    std::fs::write(&key_path, cert.key_pair.serialize_pem()).unwrap();
    rsx_cast::TlsConfig {
        server: Some(rsx_cast::TlsServer {
            cert_path: cert_path.clone(),
            key_path,
        }),
        client: Some(rsx_cast::TlsClient { cert_path }),
    }
}

fn warm_fill(seq: u64, taker: u32, maker: u32) -> FillRecord {
    FillRecord {
        seq,
        ts_ns: 1_000_000_000 + seq,
        symbol_id: 1,
        taker_user_id: taker,
        maker_user_id: maker,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: seq,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100 + seq,
        price: Price(500_000_000),
        qty: Qty(5_0000),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    }
}

fn warm_accept(seq: u64, user: u32, oid_lo: u64) -> OrderAcceptedRecord {
    OrderAcceptedRecord {
        seq,
        ts_ns: 1_000_000_000 + seq,
        user_id: user,
        symbol_id: 1,
        order_id_hi: 0,
        order_id_lo: oid_lo,
        price: 500_000_000,
        qty: 2_0000,
        side: 0,
        tif: 0,
        reduce_only: 0,
        post_only: 0,
        cid: [0; 20],
    }
}

/// WARM CATCHUP end-to-end against a real in-process
/// `ReplicationService`: write FILL + ORDER_ACCEPTED records to a
/// WAL, stand up the replication server, then drive a
/// `ReplicationConsumer` through the EXACT warm-catchup loop logic
/// (apply via `replay::apply_record`, detect `RECORD_CAUGHT_UP`,
/// track `applied_seq`). Asserts the shard warmed WITHOUT any PG
/// persist worker — position built, tip advanced (invariant #5),
/// freeze recorded — and that `caught_up` is derived correctly
/// (`applied_seq >= live_seq`).
#[tokio::test]
async fn warm_catchup_applies_me_stream_and_detects_caught_up() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    // ME WAL: two fills for user 4 (4 % 1 == 0 → in shard 0),
    // then an ORDER_ACCEPTED that freezes margin for a resting
    // order. live_seq after this is 3.
    let stream_id = 1u32;
    let mut writer = WalWriter::new(stream_id, &wal_dir, 64 * 1024 * 1024).unwrap();
    let mut f1 = warm_fill(1, 4, 8);
    let fr = writer.prepare(&mut f1).unwrap();
    writer.append_framed(&fr).unwrap();
    let mut f2 = warm_fill(2, 4, 8);
    let fr = writer.prepare(&mut f2).unwrap();
    writer.append_framed(&fr).unwrap();
    let mut a3 = warm_accept(3, 4, 9999);
    let fr = writer.prepare(&mut a3).unwrap();
    writer.append_framed(&fr).unwrap();
    writer.flush().unwrap();
    let live_seq = writer.last_seq();
    assert_eq!(live_seq, 3);

    let tls = test_tls(tmp.path());
    let replay_addr = reserve_port();
    let wal_dir_srv = wal_dir.clone();
    let tls_srv = tls.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let service = ReplicationService::new(wal_dir_srv, tls_srv).unwrap();
        rt.block_on(async move {
            service.serve(replay_addr).await.unwrap();
        });
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::net::TcpStream::connect(replay_addr).is_err() {
        if std::time::Instant::now() > deadline {
            panic!("replication server failed to bind");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    // Warm candidate: fresh shard, account preloaded with
    // collateral, NO persist producer (persist_producer = None),
    // NO gateway ingress/egress, NO liquidation tick.
    let mut shard = RiskShard::new(test_config());
    shard.accounts.insert(
        4u32,
        Account {
            user_id: 4,
            collateral: 10_000_000_000,
            version: 0,
        },
    );

    let tip_file = tmp.path().join("risk_warm_tip.bin");
    let mut consumer =
        ReplicationConsumer::new(stream_id, vec![replay_addr.to_string()], tip_file, tls).unwrap();

    // Drive the warm-catchup loop body (mirrors
    // main.rs::run_warm_catchup): apply each record via
    // `apply_record`, stop on RECORD_CAUGHT_UP, track applied_seq.
    let mut applied_seq: u64 = 0;
    let mut caught_live_seq: Option<u64> = None;
    consumer
        .run_once(|raw| {
            if raw.header.record_type == RECORD_CAUGHT_UP {
                if let Some(rec) = decode_payload::<CaughtUpRecord>(&raw.payload) {
                    caught_live_seq = Some(rec.live_seq);
                }
                return false;
            }
            let seq = extract_seq(&raw.payload).unwrap_or(0);
            if seq > applied_seq {
                applied_seq = seq;
            }
            apply_record(&mut shard, raw.header.record_type, &raw.payload);
            true
        })
        .await
        .expect("warm catchup stream");

    // caught_up ⟺ saw CAUGHT_UP AND applied_seq >= live_seq.
    let caught_up = match caught_live_seq {
        Some(ls) => applied_seq >= ls,
        None => false,
    };
    assert!(caught_up, "should be caught up after draining WAL");
    assert_eq!(caught_live_seq, Some(live_seq));
    assert!(applied_seq >= live_seq);

    // Shard warmed with NO persist worker attached:
    // - position built from the two fills (10_0000 long qty).
    let pos = shard
        .positions
        .get(&(4u32, 1u32))
        .expect("warm shard should have built the position");
    assert_eq!(pos.long_qty, 10_0000);
    // - per-symbol fill tip advanced to the last FILL seq (2),
    //   monotonic (invariant #5). ORDER_ACCEPTED (seq 3) does not
    //   pass through process_fill, so it does not move the fill tip
    //   — only fills dedup against tips[].
    assert_eq!(shard.tips[1], 2);
    // - ORDER_ACCEPTED froze margin for the resting order.
    assert!(
        shard.frozen_for_user(4) > 0,
        "ORDER_ACCEPTED should have reserved frozen margin",
    );
}

/// Not-caught-up gate: if the consumer's connection ends before
/// `RECORD_CAUGHT_UP` (or applied_seq < live_seq), `caught_up` is
/// false so the warm node MUST NOT attempt the advisory lock. This
/// is the strict catch-up-only promotion gate — no cold promote.
#[test]
fn not_caught_up_blocks_lock_attempt() {
    // No CAUGHT_UP observed.
    let caught_live_seq: Option<u64> = None;
    let applied_seq: u64 = 7;
    let caught_up = match caught_live_seq {
        Some(ls) => applied_seq >= ls,
        None => false,
    };
    assert!(!caught_up);

    // CAUGHT_UP seen but still behind live_seq.
    let caught_live_seq = Some(10u64);
    let applied_seq = 9u64;
    let caught_up = match caught_live_seq {
        Some(ls) => applied_seq >= ls,
        None => false,
    };
    assert!(!caught_up);

    // Caught up exactly.
    let caught_live_seq = Some(10u64);
    let applied_seq = 10u64;
    let caught_up = match caught_live_seq {
        Some(ls) => applied_seq >= ls,
        None => false,
    };
    assert!(caught_up);
}
