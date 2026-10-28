//! Composition root for the risk process: config, health server,
//! the restart loop, and one node lifecycle per `run_node` call —
//! Postgres load, warm catch-up, promotion, the live loop, teardown.

use crate::cast_loop;
use crate::cast_loop::LiveExit;
use crate::metrics;
use rsx_cast::cast::CastReceiver;
use rsx_health::DaemonState;
use rsx_health::LoadGauges;
use rsx_risk::config::load_shard_config;
use rsx_risk::config::load_symbol_configs;
use rsx_risk::config::log_shard_config;
use rsx_risk::config::RuntimeConfig;
use rsx_risk::config::SymbolRiskConfig;
use rsx_risk::db;
use rsx_risk::lease::spawn_lease_thread;
use rsx_risk::lease::stop_lease_thread;
use rsx_risk::lease::AdvisoryLease;
use rsx_risk::persist::spawn_persist_worker;
use rsx_risk::persist::stop_persist_worker;
use rsx_risk::persist::PersistEvent;
use rsx_risk::promotion::run_warm_catchup;
use rsx_risk::promotion::NodeState;
use rsx_risk::replay::load_from_postgres;
use rsx_risk::risk_utils::rand_jitter;
use rsx_risk::risk_utils::sleep_unless_shutdown;
use rsx_risk::schema::run_migrations;
use rsx_risk::shard::RiskShard;
use rsx_types::install_shutdown_handler;
use rsx_types::time_utils::time_ns;
use std::env;
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tracing::error;
use tracing::info;
use tracing::warn;

/// Backoff schedule (seconds) for shard crash-restarts.
const RESTART_BACKOFF_SECS: &[u64] = &[5, 10, 20, 40, 60, 60];
/// Max consecutive crashes before the shard gives up.
const MAX_RESTARTS: usize = 8;

/// Process entry after the panic handler and tracing are installed.
/// Never returns: exits the process on shutdown or an exhausted
/// restart budget.
pub fn run() -> ! {
    // Drain hot-path latency samples out-of-band
    // (see rsx-types/src/latency.rs). 100 ms is a
    // good compromise between dashboard freshness
    // and drain-thread CPU.
    rsx_log::start_drainer(100);

    let config = load_shard_config().expect("failed to load shard config");
    let shard_id = config.shard_id;
    let shard_count = config.shard_count;
    let max_symbols = config.max_symbols;

    info!(
        "risk shard {} starting ({} shards, {} symbols)",
        shard_id, shard_count, max_symbols,
    );
    log_shard_config(&config);

    // Health server: RSX_RISK_HEALTH_ADDR=127.0.0.1:9201
    // GET /health → 200/503 liveness
    // GET /ready   → 200/503 readiness (only when Live)
    // GET /metrics → JSON (ring occupancy, counters)
    let gauges: Arc<LoadGauges> = LoadGauges::new();
    gauges.live.store(true, Ordering::Relaxed);
    // ready=false until we reach NodeState::Live
    if let Ok(addr_str) = env::var("RSX_RISK_HEALTH_ADDR") {
        if let Ok(addr) = addr_str.parse::<SocketAddr>() {
            let g = gauges.clone();
            rsx_health::spawn_health_server(addr, move || metrics::health_snapshot(&g));
        } else {
            warn!("RSX_RISK_HEALTH_ADDR: invalid addr '{addr_str}'");
        }
    }

    // SIGTERM/SIGINT flip this flag. The live loop observes it,
    // `run_node` drains the persist ring to Postgres, stops the
    // lease thread, and exits the process directly (so it never
    // re-enters warm catchup). This top-level loop also checks it
    // before each (re-)entry so a signal that lands during warm
    // catchup or restart backoff exits instead of re-entering.
    let shutdown = install_shutdown_handler();

    let mut attempts: usize = 0;

    // Every process is a warm candidate main. `run_node` boots
    // into WARM CATCHUP: it loads PG state, then consumes the
    // main's authoritative ME replication stream into its shard
    // WITHOUT persisting or forwarding. Only once caught up does
    // it call the NON-BLOCKING advisory lock; the lock (not
    // catch-up) remains the sole single-main fence (invariant
    // #10). The first node catches up to an empty stream, wins the
    // free lock, and goes LIVE with the already-warm shard (no
    // full rebuild). Later nodes stay warm and retry the lock. On
    // a clean Demote (lease lost) we reset the restart budget and
    // call `run_node` again — it re-enters WARM CATCHUP and
    // re-tries the lock. `run_node` is re-enterable: it owns its
    // PG client, catchup consumer, persist worker, and lease
    // thread, and tears them all down before returning.
    loop {
        if shutdown.load(Ordering::SeqCst) {
            info!("shutdown signal received; exiting without re-entry");
            std::process::exit(0);
        }
        let err: Box<dyn std::error::Error> =
            match run_node_thread(shard_id, max_symbols, gauges.clone(), shutdown) {
                // Ok(()) means demoted (lease lost); re-enter warm
                // catchup and re-try the non-blocking advisory lock.
                Ok(()) => {
                    info!("lease lost; re-acquiring advisory lock");
                    // Back to warm catchup state: not ready until
                    // we win the lock again.
                    gauges.ready.store(false, Ordering::Relaxed);
                    gauges.set_state(DaemonState::WarmCatchup);
                    attempts = 0;
                    continue;
                }
                Err(e) => e,
            };

        attempts += 1;
        if attempts > MAX_RESTARTS {
            error!(
                "FATAL: shard {} restart budget \
                 exhausted ({} attempts); last error: {err}",
                shard_id, attempts,
            );
            std::process::exit(1);
        }
        let backoff_secs = RESTART_BACKOFF_SECS[attempts
            .saturating_sub(1)
            .min(RESTART_BACKOFF_SECS.len() - 1)];
        // ±20% jitter
        let jitter_ms = (backoff_secs as f64 * 200.0 * (rand_jitter() - 0.5)) as i64;
        let sleep_ms = (backoff_secs * 1000) as i64 + jitter_ms;
        error!(
            "crashed ({}/{} attempts): \
             {err}; restart in {sleep_ms}ms",
            attempts, MAX_RESTARTS,
        );
        sleep_unless_shutdown(Duration::from_millis(sleep_ms.max(100) as u64), shutdown);
    }
}

/// One `run_node` cycle on its own thread: the hot pin dies with the
/// cycle, so the next cycle's side-threads inherit the open mask.
fn run_node_thread(
    shard_id: u32,
    max_symbols: usize,
    gauges: Arc<LoadGauges>,
    shutdown: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| run_node(shard_id, max_symbols, gauges, shutdown).map_err(|e| e.to_string()))
            .join()
            .expect("run_node thread panicked; the panic hook exits first")
    })
    .map_err(Into::into)
}

/// Install every symbol's active `symbol_config_schedule` row.
async fn apply_symbol_configs(
    client: &tokio_postgres::Client,
    shard: &mut RiskShard,
    max_symbols: usize,
) -> std::io::Result<()> {
    let now_ms = time_ns() / 1_000_000;
    let configs = load_symbol_configs(client, max_symbols, now_ms).await?;
    for config in &configs {
        shard.apply_symbol_config(config);
    }
    info!(
        "symbol config loaded: {} of {} symbols configured",
        configs.len(),
        max_symbols
    );
    Ok(())
}

/// One node lifecycle: load Postgres, warm-catch-up, win the lock,
/// run live, tear down. `Ok(())` is a clean demote (lease lost) and
/// the caller re-enters; `Err` counts against the restart budget.
fn run_node(
    shard_id: u32,
    max_symbols: usize,
    gauges: Arc<LoadGauges>,
    shutdown: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = load_shard_config()?;
    let shard_count = config.shard_count;
    let lease_renew_interval_ms = config.replication_config.lease_renew_interval_ms;
    let lease_renew_interval_secs = (lease_renew_interval_ms / 1000).max(1);
    let lease_poll_interval_ms = config.replication_config.lease_poll_interval_ms;
    let mut shard = RiskShard::new(config);

    // All process I/O topology parsed once, fail-fast on
    // malformed required values (DATABASE_URL, ME repl addr,
    // cast addrs). See RuntimeConfig for defaults.
    let cfg = RuntimeConfig::from_env();
    // `wal_dir` is passed as &str to handle_replay /
    // run_warm_catchup; derive it once from the PathBuf.
    let wal_dir: &str = cfg
        .wal_dir
        .to_str()
        .expect("RSX_RISK_WAL_DIR must be valid UTF-8");
    // Replication is TLS-mandatory: read the cert paths once at
    // startup so warm-catchup + every FAULTED replay reuse them.
    let tls = rsx_cast::TlsConfig::from_env()?;
    let db_url = &cfg.db_url;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    // Load PG state WITHOUT acquiring the advisory lock. Every
    // node loads + warms; the lock is only attempted after
    // catch-up (see WARM CATCHUP below). The connection is kept
    // alive by a background task; the same `rt` + `pg_client` are
    // later moved into the lease thread on promotion.
    let mut lease = AdvisoryLease::new(shard_id);
    let pg_client = rt.block_on(async {
        let client = db::connect(db_url).await?;
        // Migrations are idempotent and concurrency-safe (each is
        // version-guarded + idempotent DDL + ON CONFLICT), so every
        // node can run them at boot with no lock. See migrations/CLAUDE.md.
        run_migrations(&client).await?;
        let state = load_from_postgres(&client, shard_id, shard_count, max_symbols).await?;
        shard.set_state(state);
        info!("loaded accounts from postgres (warm candidate)");
        apply_symbol_configs(&client, &mut shard, max_symbols).await?;
        Ok::<_, Box<dyn std::error::Error>>(client)
    })?;

    let warm_streams: Vec<(u32, String)> = cfg
        .me_streams
        .iter()
        .map(|stream| (stream.symbol_id, stream.replay_addr.clone()))
        .collect();

    // Mark stream addresses (consumed in warm AND live).
    let mut mark_receiver = CastReceiver::new(cfg.mark_addr, cfg.mark_sender_addr)
        .expect("failed to bind mark cast receiver");

    // ===== WARM CATCHUP =====
    // Consume the main's authoritative ME replication stream into
    // the shard until caught up, then win the non-blocking lock.
    // Blocks (re-trying the lock) until promotion. On error,
    // returns it to the restart loop.
    let mut state = NodeState::WarmCatchup;
    gauges.set_state(DaemonState::WarmCatchup);
    gauges.ready.store(false, Ordering::Relaxed);
    info!("risk shard {} state={:?}", shard_id, state);
    run_warm_catchup(
        &rt,
        &pg_client,
        &mut lease,
        &mut shard,
        &mut mark_receiver,
        &warm_streams,
        wal_dir,
        lease_poll_interval_ms,
        &tls,
        shutdown,
    )?;
    // Warm catchup also returns on SIGTERM, where the lock was
    // never acquired -- promoting then would make a shutting-down
    // node the writer.
    if shutdown.load(Ordering::Relaxed) {
        info!("risk shard {} exiting before promotion", shard_id);
        return Ok(());
    }
    // Past this point this node is the SOLE advisory-lock holder
    // (invariant #10) and the shard is warm + final-drained. Symbol
    // config that changed while this node was warm (no persist
    // worker, so CONFIG_APPLIED could not trigger a reload) is
    // reloaded here, before the first live order.
    rt.block_on(apply_symbol_configs(&pg_client, &mut shard, max_symbols))?;
    rt.block_on(shard.follow_funding(&pg_client))?;
    state = NodeState::Live;
    gauges.set_state(DaemonState::Live);
    gauges.ready.store(true, Ordering::Relaxed);
    info!("risk shard {} state={:?} (promoted)", shard_id, state);

    let lease_held = Arc::new(AtomicBool::new(true));
    let lease_error = Arc::new(AtomicBool::new(false));
    let lease_stop = Arc::new(AtomicBool::new(false));
    let lease_thread = spawn_lease_thread(
        rt,
        pg_client,
        lease,
        lease_renew_interval_secs,
        lease_held.clone(),
        lease_error.clone(),
        lease_stop.clone(),
    );

    let (persist_prod, persist_cons) = rtrb::RingBuffer::<PersistEvent>::new(8192);
    gauges.persist_ring_cap.store(8192, Ordering::Relaxed);
    shard.set_persist_producer(persist_prod);
    let (config_prod, config_cons) = rtrb::RingBuffer::<SymbolRiskConfig>::new(64);
    shard.set_config_consumer(config_cons);

    // Persist worker thread. We retain its `JoinHandle` and
    // a shutdown flag so that a demote can stop the worker
    // cleanly before returning — otherwise a demote →
    // re-acquire cycle leaks worker threads, each holding its
    // own PG connection.
    let persist_shutdown = Arc::new(AtomicBool::new(false));
    let persist_handle = spawn_persist_worker(
        db_url.clone(),
        shard_id,
        persist_cons,
        config_prod,
        persist_shutdown.clone(),
    );

    if let Some(core_id) = cfg.core_id {
        let setup = rsx_types::cpu::setup_hot_thread(core_id);
        info!("risk {}", setup);
        if setup.isolated == Some(false) {
            warn!("risk core {} not isolated — expect tail spikes", core_id);
        }
    }

    let mut io = cast_loop::bind(&cfg, mark_receiver, wal_dir);

    info!("risk shard {} running state={:?}", shard_id, state);

    let exit = cast_loop::run(
        &mut shard,
        &mut io,
        &cfg,
        &tls,
        &gauges,
        shutdown,
        &lease_held,
        &lease_error,
        &persist_handle,
        shard_id,
    );

    // Every exit tears down both sidecar threads (each owns a PG
    // connection) so the next promote spawns fresh ones.
    stop_persist_worker(&persist_shutdown, persist_handle);
    stop_lease_thread(&lease_stop, lease_thread);
    match exit {
        LiveExit::Shutdown => {
            info!("risk shard {} shutdown complete", shard_id);
            std::process::exit(0);
        }
        LiveExit::PersistWorkerDied => Err("persist worker died (circuit open); demoting".into()),
        LiveExit::LeaseError => Err("lease check failed after 3 consecutive errors".into()),
        LiveExit::LeaseLost => {
            warn!("lease lost, re-acquiring advisory lock");
            Ok(())
        }
    }
}
