//! Persist worker resilience under Postgres outages.
//!
//! Gap 1: PG disconnect mid-flush — worker must survive the
//! outage, buffer writes during the failure window, and flush
//! them once the database recovers.
//!
//! Gap 2: Circuit-open after CIRCUIT_AT consecutive failures —
//! when the DB stays unreachable, the worker eventually gives
//! up and terminates instead of retrying forever.
//!
//! Both tests need Docker; gated with #[ignore], run under
//! `make integration`.

use rsx_risk::persist::PersistEvent;
use rsx_risk::persist::run_persist_worker;
use rsx_risk::schema::run_migrations;
use rtrb::RingBuffer;
use std::time::Duration;
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
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
    let (client, conn) =
        tokio_postgres::connect(&connstr, NoTls)
            .await
            .unwrap();
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            warn!("pg conn closed: {e}");
        }
    });
    client
}

/// Gap 1 — survive a Postgres restart mid-stream.
///
/// Drives the worker through a normal flush, stops the
/// container, pushes more events during the outage (the
/// worker keeps them in `pending` and retries with backoff),
/// then restarts the container. After recovery the buffered
/// events must reach Postgres.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn persist_survives_pg_restart() {
    let _ = tracing_subscriber::fmt::try_init();

    let (container, port, verify_client) = pg_setup().await;

    // The worker owns its own client.
    let worker_client = connect(port).await;

    let (mut producer, consumer) =
        RingBuffer::<PersistEvent>::new(1024);

    // Spawn the persist worker.
    let worker = tokio::spawn(async move {
        run_persist_worker(consumer, worker_client, 0).await;
    });

    // Phase 1: normal write while DB is up.
    producer
        .push(PersistEvent::Tip { symbol_id: 0, seq: 1 })
        .unwrap();
    wait_for_tip(&verify_client, 0, 1).await;

    // Phase 2: take the DB down.
    container.stop().await.unwrap();

    // Push enough events to confirm the worker keeps buffering
    // while flushes fail. CIRCUIT_AT is 8; we stay below it.
    for seq in 2..=6u64 {
        producer
            .push(PersistEvent::Tip { symbol_id: 0, seq })
            .unwrap();
    }

    // Give the worker time to attempt at least one failed
    // flush (FLUSH_INTERVAL_MS=10, BACKOFF_INIT_MS=100).
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Phase 3: bring the DB back. Container restart preserves
    // its host port mapping in testcontainers ≥0.23.
    container.start().await.unwrap();

    // Worker reconnects through retry/backoff; existing
    // `worker_client` was opened against the same port so the
    // OS-level socket reconnects when PG comes back.

    // The verify client may have been killed by the restart;
    // open a fresh one.
    let verify_client = connect(port).await;
    wait_for_tip(&verify_client, 0, 6).await;

    // Cleanup.
    drop(producer);
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        worker,
    )
    .await;
}

/// Gap 2 — circuit opens after CIRCUIT_AT consecutive failures.
///
/// Strategy: connect the worker to a Postgres that we then
/// stop and never restart. After ~8 failed flushes the worker
/// should log "persist circuit open" and the JoinHandle
/// completes (the loop breaks).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn persist_circuit_opens_on_sustained_failure() {
    let _ = tracing_subscriber::fmt::try_init();

    let (container, port, _verify) = pg_setup().await;
    let worker_client = connect(port).await;
    let (mut producer, consumer) =
        RingBuffer::<PersistEvent>::new(1024);

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
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        worker,
    )
    .await;

    assert!(
        result.is_ok(),
        "worker did not exit after sustained PG failure \
         (circuit should have opened)",
    );
    result.unwrap().unwrap();

    drop(producer);
}

/// Poll the `tips` table until a given seq appears.
/// Bounded wait so a hang in the worker doesn't hang the test.
async fn wait_for_tip(
    client: &Client,
    symbol_id: u32,
    expected_seq: u64,
) {
    let deadline = std::time::Instant::now()
        + Duration::from_secs(30);
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
