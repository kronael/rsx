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

use rsx_risk::persist::run_persist_worker;
use rsx_risk::persist::run_persist_worker_with_shutdown;
use rsx_risk::persist::PersistEvent;
use rsx_risk::schema::run_migrations;
use rtrb::RingBuffer;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;
use tokio_postgres::Client;
use tokio_postgres::NoTls;
use tracing::warn;

/// Boot a Postgres container, run migrations, return both the
/// handle (for stop/start control) and a connected client.
async fn pg_setup() -> (ContainerAsync<Postgres>, u16, Client) {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let client = connect(port).await;
    run_migrations(&client).await.unwrap();
    (container, port, client)
}

/// Open a fresh tokio-postgres connection to the given port.
async fn connect(port: u16) -> Client {
    let connstr = format!(
        "host=localhost port={port} user=postgres \
         password=postgres dbname=postgres"
    );
    let (client, conn) = tokio_postgres::connect(&connstr, NoTls).await.unwrap();
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            warn!("pg conn closed: {e}");
        }
    });
    client
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
    let shutdown = Arc::new(AtomicBool::new(false));

    let worker = {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            run_persist_worker_with_shutdown(consumer, worker_client, 0, Some(shutdown)).await;
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
    shutdown.store(true, Ordering::Relaxed);
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
