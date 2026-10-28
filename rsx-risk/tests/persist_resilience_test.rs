//! Persist worker resilience under Postgres outages.
//!
//! Gap 1: PG disconnect mid-stream — worker must NOT drop
//! events or panic during the outage; pending events stay in
//! `pending` while the worker keeps retrying via exponential
//! backoff. Recovery from a full PG restart requires a worker
//! restart with a fresh `Client`: `run_persist_worker` does
//! NOT reconnect (see `rsx-risk/src/persist.rs` :380-446 — it
//! retries `flush_batch` against the same `Client`, and
//! `tokio-postgres` does not auto-reconnect after the
//! connection task dies).
//!
//! Gap 2: Circuit-open after CIRCUIT_AT consecutive failures —
//! when the DB stays unreachable, the worker eventually gives
//! up and terminates instead of retrying forever.
//!
//! Gap 3: Shutdown signal causes a clean worker exit — used by
//! `run_main` to stop the worker on a demote (lease loss) so
//! that the demote → re-acquire cycle does not leak worker
//! threads.
//!
//! All three tests need Docker; gated with #[ignore], run
//! under `make integration`.

mod common;

use crate::common::connect;
use crate::common::PostgresNoLogWait;
use rsx_risk::lease::AdvisoryLease;
use rsx_risk::persist::flush_batch;
use rsx_risk::persist::run_persist_worker;
use rsx_risk::persist::run_persist_worker_with_shutdown;
use rsx_risk::persist::PersistEvent;
use rsx_risk::persist::StopMode;
use rsx_risk::schema::run_migrations;
use rtrb::RingBuffer;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use testcontainers::ContainerAsync;
use tokio_postgres::Client;

/// Boot a Postgres container, run migrations, return both the
/// handle (for stop/start control) and a connected client.
async fn pg_setup() -> (ContainerAsync<PostgresNoLogWait>, u16, Client) {
    let (container, port) = common::start_postgres().await;
    let client = connect(port).await;
    run_migrations(&client).await.unwrap();
    (container, port, client)
}

/// Gap 1 — worker does not drop events or panic during a PG
/// outage. Drives the worker through a normal flush, stops
/// the container, pushes more events while the DB is down,
/// and asserts the worker survives (does not panic) for the
/// duration of the outage window. The buffered events stay
/// in the worker's `pending` Vec across retries — they are
/// neither flushed (because the client is dead) nor dropped.
///
/// NB: this test does NOT assert that buffered events reach
/// Postgres after restart. `run_persist_worker` does not
/// reconnect; tokio-postgres's `Client` becomes unusable
/// once its connection task dies. Recovery from a full PG
/// restart requires a worker restart with a fresh `Client`.
/// See module docs and `rsx-risk/src/persist.rs` :380-446.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn persist_survives_pg_outage_without_dropping() {
    let _ = tracing_subscriber::fmt::try_init();

    let (container, port, verify_client) = pg_setup().await;

    // The worker owns its own client.
    let worker_client = connect(port).await;

    let (mut producer, consumer) = RingBuffer::<PersistEvent>::new(1024);

    // Spawn the persist worker.
    let worker = tokio::spawn(async move {
        run_persist_worker(consumer, worker_client, 0).await;
    });

    // Phase 1: normal write while DB is up.
    producer
        .push(PersistEvent::Tip {
            symbol_id: 0,
            seq: 1,
        })
        .unwrap();
    wait_for_tip(&verify_client, 0, 1).await;

    // Phase 2: take the DB down and push more events. The
    // worker should retain them in `pending` and keep
    // retrying. CIRCUIT_AT is 8; we stay below it so the
    // worker remains alive throughout this phase.
    container.stop().await.unwrap();
    for seq in 2..=6u64 {
        producer
            .push(PersistEvent::Tip { symbol_id: 0, seq })
            .unwrap();
    }

    // Give the worker time to attempt several failed flushes
    // (FLUSH_INTERVAL_MS=10, BACKOFF_INIT_MS=100). Worker
    // must still be running (not panicked, not exited).
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !worker.is_finished(),
        "worker exited during outage (CIRCUIT_AT=8 not yet \
         reached); should still be retrying with backoff",
    );

    // Cleanup: drop the producer so the consumer side ends,
    // then let the worker exit via circuit-open. We do NOT
    // assert that the buffered tips (2..=6) reach PG — the
    // worker has no reconnection logic; see module docs.
    drop(producer);
    let _ = tokio::time::timeout(Duration::from_secs(60), worker).await;
}

/// Gap 2 — circuit opens after CIRCUIT_AT consecutive failures.
///
/// Strategy: connect the worker to a Postgres that we then
/// stop and never restart. After ~8 failed flushes the worker
/// should log "persist circuit open" and the JoinHandle
/// completes (the loop breaks).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn persist_circuit_opens_on_sustained_failure() {
    let _ = tracing_subscriber::fmt::try_init();

    let (container, port, _verify) = pg_setup().await;
    let worker_client = connect(port).await;
    let (mut producer, consumer) = RingBuffer::<PersistEvent>::new(1024);

    let worker = tokio::spawn(async move {
        run_persist_worker(consumer, worker_client, 0).await;
    });

    // Stop PG immediately so every flush fails.
    container.stop().await.unwrap();

    // Push CIRCUIT_AT + 1 events. The persist worker batches
    // pending events per flush cycle, so one big batch counts
    // as one failure — push them spaced out so each lands in
    // its own batch. FLUSH_INTERVAL_MS = 10ms.
    for seq in 1..=12u64 {
        producer
            .push(PersistEvent::Tip { symbol_id: 0, seq })
            .unwrap();
        // Sleep > flush interval so the worker picks each
        // event into a separate flush batch.
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    // The exponential backoff means 8 failures span
    // ~100+200+400+800+1600+3200+6400+12800 ms = ~25s worst
    // case. Allow generous wait — but each retry doubles, so
    // commonly the loop exits well before the wall clock.
    let result = tokio::time::timeout(Duration::from_secs(60), worker).await;

    assert!(
        result.is_ok(),
        "worker did not exit after sustained PG failure \
         (circuit should have opened)",
    );
    result.unwrap().unwrap();

    drop(producer);
}

/// Gap 3 — shutdown signal causes the worker to exit cleanly,
/// proving that a demote → re-acquire cycle no longer leaks a
/// worker thread. We write one tip, flip the shutdown flag, and
/// assert the worker handle resolves within a generous window.
/// Without `run_main`'s shutdown plumbing each re-acquire would
/// spawn another worker, each holding its own PG connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn persist_worker_exits_on_shutdown_signal() {
    let _ = tracing_subscriber::fmt::try_init();

    let (_container, port, verify_client) = pg_setup().await;
    let worker_client = connect(port).await;
    let (mut producer, consumer) = RingBuffer::<PersistEvent>::new(64);
    let shutdown = Arc::new(AtomicU8::new(StopMode::Run as u8));

    let worker = {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            run_persist_worker_with_shutdown(
                consumer,
                worker_client,
                0,
                Some(shutdown),
                None,
                None,
            )
            .await;
        })
    };

    // Prove the worker is alive and flushing.
    producer
        .push(PersistEvent::Tip {
            symbol_id: 0,
            seq: 1,
        })
        .unwrap();
    wait_for_tip(&verify_client, 0, 1).await;

    // Signal shutdown and assert the worker exits within the
    // demote budget used by `stop_persist_worker` (5s).
    shutdown.store(StopMode::FlushAndStop as u8, Ordering::Relaxed);
    let result = tokio::time::timeout(Duration::from_secs(5), worker).await;
    assert!(
        result.is_ok(),
        "worker did not exit within 5s of shutdown signal; \
         demote would leak the thread on every cycle",
    );
    result.unwrap().unwrap();

    drop(producer);
}

/// Poll the `tips` table until a given seq appears.
/// Bounded wait so a hang in the worker doesn't hang the test.
async fn wait_for_tip(client: &Client, symbol_id: u32, expected_seq: u64) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if std::time::Instant::now() > deadline {
            panic!(
                "tip seq>={expected_seq} for sym {symbol_id} \
                 not observed within deadline",
            );
        }
        let rows = client
            .query(
                "SELECT last_seq FROM tips \
                 WHERE instance_id = 0 AND symbol_id = $1",
                &[&(symbol_id as i32)],
            )
            .await;
        if let Ok(rows) = rows {
            if let Some(row) = rows.first() {
                let seq: i64 = row.get(0);
                if seq as u64 >= expected_seq {
                    return;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Spawn a worker whose stop mode is already set, push one tip,
/// and return whether the tip reached Postgres once the worker
/// exited.
async fn tip_flushed_after_stop(mode: StopMode) -> bool {
    let (_container, port, verify_client) = pg_setup().await;
    let worker_client = connect(port).await;
    let (mut producer, consumer) = RingBuffer::<PersistEvent>::new(64);
    let shutdown = Arc::new(AtomicU8::new(mode as u8));
    producer
        .push(PersistEvent::Tip {
            symbol_id: 0,
            seq: 1,
        })
        .unwrap();
    let worker = tokio::spawn(run_persist_worker_with_shutdown(
        consumer,
        worker_client,
        0,
        Some(shutdown),
        None,
        None,
    ));
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .expect("worker exits")
        .unwrap();
    let rows = verify_client
        .query("SELECT 1 FROM tips WHERE instance_id = 0", &[])
        .await
        .unwrap();
    !rows.is_empty()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn persist_worker_flushes_pending_on_flush_and_stop() {
    assert!(tip_flushed_after_stop(StopMode::FlushAndStop).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn persist_worker_drops_pending_on_drop_and_stop() {
    assert!(!tip_flushed_after_stop(StopMode::DropAndStop).await);
}

/// A flush fenced on a session that does not hold the shard's
/// advisory lock aborts and writes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires docker; pg testcontainer"]
async fn flush_batch_aborts_when_lock_session_lost_the_lock() {
    let (_container, port, verify_client) = pg_setup().await;
    let holder = connect(port).await;
    let mut lease = AdvisoryLease::new(7);
    assert!(lease.try_acquire(&holder).await.unwrap());
    let tip = [PersistEvent::Tip {
        symbol_id: 0,
        seq: 1,
    }];

    let mut writer = connect(port).await;
    flush_batch(&mut writer, 7, &tip, Some(lease.backend_pid))
        .await
        .expect("holder session holds the lock");

    let other = connect(port).await;
    let other_pid: i32 = other
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let tip2 = [PersistEvent::Tip {
        symbol_id: 0,
        seq: 2,
    }];
    assert!(flush_batch(&mut writer, 7, &tip2, Some(other_pid))
        .await
        .is_err());

    lease.release(&holder).await.unwrap();
    assert!(flush_batch(&mut writer, 7, &tip2, Some(lease.backend_pid))
        .await
        .is_err());
    let row = verify_client
        .query_one("SELECT last_seq FROM tips WHERE instance_id = 7", &[])
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 1);
}
