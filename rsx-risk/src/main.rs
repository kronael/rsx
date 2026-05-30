use rsx_cast::as_bytes;
use rsx_cast::cast::CastRecvWith;
use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastSender;
use rsx_cast::config::CastConfig;
use rsx_cast::decode_payload;
use rsx_cast::ReplicationConsumer;
use rsx_cast::RECORD_CAUGHT_UP;
use rsx_cast::wal::extract_seq;
use std::collections::HashMap;
use rsx_messages::ConfigAppliedRecord;
use rsx_messages::BboRecord;
use rsx_messages::FillRecord;
use rsx_messages::MarkPriceRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_ORDER_ACCEPTED;
use rsx_messages::RECORD_CONFIG_APPLIED;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_MARK_PRICE;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_messages::CancelRequest;
use rsx_messages::RECORD_CANCEL_REQUEST;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_matching::wire::OrderMessage;
use rsx_health::LoadGauges;
use rsx_risk::config::load_shard_config;
use rsx_risk::lease::AdvisoryLease;
use rsx_risk::persist::run_persist_worker_with_shutdown;
use rsx_risk::replay::apply_record;
use rsx_risk::replay::load_from_postgres;
use rsx_risk::schema::run_migrations;
use rsx_risk::replay::replay_from_wal;
use rsx_cast::CaughtUpRecord;
use rsx_risk::rings::ShardRings;
use rsx_risk::shard::RiskShard;
use rsx_risk::ShardConfig;
use rsx_risk::BboUpdate;
use rsx_risk::FillEvent;
use rsx_risk::OrderRequest;
use rsx_risk::OrderResponse;
use rsx_risk::RejectReason;
use rsx_risk::persist::PersistEvent;
use rsx_types::install_panic_handler;
use rsx_types::FailureReason;
use rsx_types::time_utils::time;
use std::env;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tracing::error;
use tracing::info;
use tracing::warn;

mod metrics;

/// Backoff schedule (seconds) for shard crash-restarts.
const RESTART_BACKOFF_SECS: &[u64] = &[
    5, 10, 20, 40, 60, 60,
];
/// Max consecutive crashes before the shard gives up.
const MAX_RESTARTS: usize = 8;

/// Transition signalled by `run_main` on return.
#[derive(Debug)]
enum MainTransition {
    /// Advisory lease lost; main() should loop and call
    /// `run_main` again, which re-enters WARM CATCHUP and
    /// re-tries the non-blocking lock acquire.
    Demote,
}

/// Node role in the eager warm-standby protocol. Every process
/// boots into `WarmCatchup`, consumes the main's authoritative
/// ME replication stream, and only transitions to `Live` once it
/// is caught up AND wins the (non-blocking) advisory lock. There
/// is no separate "cold main boot" — promotion is always warm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeState {
    /// Applying the ME replication stream into the shard with no
    /// persist worker, no gateway ingress, no egress, no
    /// liquidation tick. Polling the non-blocking lock once
    /// caught up.
    WarmCatchup,
    /// Sole lock holder (invariant #10). Persist worker, lease
    /// renewal, gateway ingress + egress, liquidation tick all
    /// attached. Applying ME records AND forwarding to GW.
    Live,
}

fn log_effective_risk_config(config: &ShardConfig) {
    info!(
        "risk effective config: shard_id={} shard_count={} max_symbols={} lease_poll_ms={} lease_renew_ms={} liquidation_base_delay_ns={} liquidation_base_slip_bps={} liquidation_max_rounds={}",
        config.shard_id,
        config.shard_count,
        config.max_symbols,
        config.replication_config.lease_poll_interval_ms,
        config.replication_config.lease_renew_interval_ms,
        config.liquidation_config.base_delay_ns,
        config.liquidation_config.base_slip_bps,
        config.liquidation_config.max_rounds,
    );
    for sid in 0..config.max_symbols {
        let p = &config.symbol_params[sid];
        info!(
            "risk symbol_config sid={} im_rate={} mm_rate={} max_leverage={} taker_fee_bps={} maker_fee_bps={}",
            sid,
            p.initial_margin_rate,
            p.maintenance_margin_rate,
            p.max_leverage,
            config.taker_fee_bps[sid],
            config.maker_fee_bps[sid],
        );
    }

    info!(
        "risk shard_routing rule='user_id % shard_count == shard_id' shard_id={} shard_count={}",
        config.shard_id,
        config.shard_count,
    );
    for user_id in 0u32..8 {
        let owner = user_id % config.shard_count;
        let serves = owner == config.shard_id;
        info!(
            "risk shard_routing_example user_id={} owner_shard={} served_here={}",
            user_id, owner, serves
        );
    }
}

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .init();

    // Drain hot-path latency samples out-of-band
    // (see rsx-types/src/latency.rs). 100 ms is a
    // good compromise between dashboard freshness
    // and drain-thread CPU.
    rsx_log::start_drainer(100);

    // SAFETY: fail-fast at startup
    let config = load_shard_config()
        .expect("failed to load shard config");
    let shard_id = config.shard_id;
    let shard_count = config.shard_count;
    let max_symbols = config.max_symbols;

    info!(
        "risk shard {} starting ({} shards, {} symbols)",
        shard_id, shard_count, max_symbols,
    );
    log_effective_risk_config(&config);

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

    let mut attempts: usize = 0;

    // Every process is a warm candidate main. `run_main` boots
    // into WARM CATCHUP: it loads PG state, replays boot WAL, then
    // consumes the main's authoritative ME replication stream into
    // its shard WITHOUT persisting or forwarding. Only once caught
    // up does it call the NON-BLOCKING advisory lock; the lock
    // (not catch-up) remains the sole single-main fence (invariant
    // #10). The first node catches up to an empty stream, wins the
    // free lock, and goes LIVE with the already-warm shard (no
    // full rebuild). Later nodes stay warm and retry the lock. On
    // a clean Demote (lease lost) we reset the restart budget and
    // call `run_main` again — it re-enters WARM CATCHUP and
    // re-tries the lock. `run_main` is re-enterable: it owns its
    // PG client, catchup consumer, persist worker, and lease
    // thread, and tears them all down before returning.
    loop {
        let err: Box<dyn std::error::Error> = match run_main(
            shard_id, max_symbols, gauges.clone(),
        ) {
            Ok(MainTransition::Demote) => {
                info!("lease lost; re-acquiring advisory lock");
                // Back to warm catchup state: not ready until
                // we win the lock again.
                gauges.ready.store(false, Ordering::Relaxed);
                gauges.state_idx.store(1, Ordering::Relaxed);
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
        let jitter_ms = (backoff_secs as f64
            * 200.0
            * (rand_jitter() - 0.5)) as i64;
        let sleep_ms =
            (backoff_secs * 1000) as i64 + jitter_ms;
        error!(
            "crashed ({}/{} attempts): \
             {err}; restart in {sleep_ms}ms",
            attempts, MAX_RESTARTS,
        );
        std::thread::sleep(Duration::from_millis(
            sleep_ms.max(100) as u64,
        ));
    }
}

/// Handle a `CastRecv::Faulted` or `CastRecv::Reconnect` by
/// draining the producer's replication stream from
/// `last_delivered_seq + 1`. Returns the new tip to pass into
/// `CastReceiver::reset_after_replay`.
///
/// The apply path is intentionally minimal for round 1 — each
/// record is just acknowledged so the receiver can resume.
/// Re-injecting orders/fills/marks into the live ring is a
/// follow-up (see .ship/28-REFINE-AUDIT-2/PLAN.md). The spec
/// invariant "ME never silently drops" is preserved because
/// matching has its own FAULTED handler that re-runs the
/// authoritative replay from risk's WAL.
///
/// Panics if the replay endpoint env var is unset (fail-loud
/// per the spec) or the replication consumer exhausts its
/// retry budget. Transient connection errors retry inside
/// `drain_replay`.
fn handle_replay(
    label: &str,
    env_var: &str,
    stream_id: u32,
    last_delivered_seq: u64,
    gap: Option<(u64, u64)>,
    wal_dir: &str,
) -> u64 {
    match gap {
        Some((gs, ge)) => warn!(
            "{label} FAULTED at seq={last_delivered_seq} \
             gap=[{gs}..={ge}], opening replay via {env_var}",
        ),
        None => warn!(
            "{label} RECONNECT at seq={last_delivered_seq}, \
             opening replay via {env_var}",
        ),
    }
    let replay_addr = env::var(env_var).unwrap_or_else(|_| {
        panic!(
            "{label} {} requires {env_var} pointing at the \
             producer's replication server",
            if gap.is_some() { "FAULTED" } else { "RECONNECT" },
        )
    });
    let tip_file = std::path::PathBuf::from(wal_dir).join(
        format!("risk_{label}_{stream_id}_replay_tip.bin"),
    );
    // Retry if the WAL hasn't flushed the gap records yet.
    // WAL flushes every 10ms; 5 retries × 15ms = 75ms covers
    // the window plus some slack for burst writes.
    let gap_end = gap.map(|(_, ge)| ge).unwrap_or(0);
    const MAX_TIP_RETRIES: u8 = 5;
    let mut tip_retries = 0u8;
    let new_tip = loop {
        let tip = rsx_risk::drain_replay(
            stream_id,
            replay_addr.clone(),
            last_delivered_seq,
            tip_file.clone(),
            |raw| {
                let seq = rsx_cast::wal::extract_seq(
                    &raw.payload,
                ).unwrap_or(0);
                tracing::debug!(
                    "{label} replay applied \
                     record_type={} seq={}",
                    raw.header.record_type, seq,
                );
            },
        )
        .unwrap_or_else(|e| {
            panic!(
                "{label} replay drain failed against \
                 {replay_addr}: {e}",
            )
        });
        tip_retries += 1;
        if tip < gap_end && tip_retries < MAX_TIP_RETRIES {
            warn!(
                "{label} replay tip={tip} < gap_end={gap_end}, \
                 WAL not flushed yet (attempt {tip_retries}), \
                 retrying in 15ms"
            );
            std::thread::sleep(
                Duration::from_millis(15),
            );
        } else {
            break tip;
        }
    };
    info!(
        "{label} replay drained, new_tip={new_tip}, resuming",
    );
    new_tip
}

/// Forward a record onto risk's gateway stream, renumbering it
/// with `gw`'s own contiguous seq (SEQ-1 fix). Risk's gateway
/// stream multiplexes forwarded ME records AND risk-generated
/// margin rejects; preserving ME's seq (or the reject's seq=0)
/// leaves holes the gateway reads as FAULTED, and seq=0 records
/// are dropped outright by the receiver. The gateway never
/// replays *from* risk, so renumbering is safe — the seq is
/// transport-only on this hop; the record is identified by its
/// order_id. CRC is recomputed by `send_raw` over the restamped
/// payload.
fn forward_to_gw(
    gw: &mut CastSender,
    record_type: u16,
    payload: &[u8],
) {
    let plen = payload.len();
    if plen < 8 || plen > 256 {
        warn!("risk: gw forward bad payload len={plen}");
        return;
    }
    let mut buf = [0u8; 256];
    buf[..plen].copy_from_slice(payload);
    let seq = gw.next_seq();
    buf[0..8].copy_from_slice(&seq.to_le_bytes());
    if let Err(e) = gw.send_raw(record_type, &buf[..plen]) {
        warn!("risk: forward to gw failed: {e}");
    }
    gw.advance_seq();
}

/// Drive a tokio_postgres connection to completion. tokio_postgres
/// returns the `Connection` future separately from the `Client`; it
/// must be polled on a task for the client to make progress. Named
/// (not an inline `tokio::spawn(async move {…})`) per CLAUDE.md so the
/// coroutine's lifetime is visible to the reader.
async fn drive_pg_connection(
    connection: tokio_postgres::Connection<
        tokio_postgres::Socket,
        tokio_postgres::tls::NoTlsStream,
    >,
) {
    if let Err(e) = connection.await {
        error!("pg connection error: {e}");
    }
}

/// Simple jitter in [0.0, 1.0) using subsecond nanos mod prime.
fn rand_jitter() -> f64 {
    use std::time::SystemTime;
    let ns = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(12345);
    (ns % 1_000_003) as f64 / 1_000_003.0
}

fn run_main(
    shard_id: u32,
    max_symbols: usize,
    gauges: Arc<LoadGauges>,
) -> Result<MainTransition, Box<dyn std::error::Error>> {
    let config = load_shard_config()?;
    let shard_count = config.shard_count;
    let lease_renew_interval_ms = config.replication_config.lease_renew_interval_ms;
    let lease_renew_interval_secs = (lease_renew_interval_ms / 1000).max(1);
    let lease_poll_interval_ms = config.replication_config.lease_poll_interval_ms;
    let mut shard = RiskShard::new(config);

    // SAFETY: fail-fast at startup -- risk requires
    // postgres for state persistence and advisory locks
    let db_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL required for risk");
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
        let (client, connection) =
            tokio_postgres::connect(
                &db_url,
                tokio_postgres::NoTls,
            )
            .await?;
        tokio::spawn(drive_pg_connection(connection));
        // Migrations are idempotent and concurrency-safe (each is
        // version-guarded + idempotent DDL + ON CONFLICT), so every
        // node can run them at boot with no lock. See migrations/CLAUDE.md.
        run_migrations(&client).await?;
        let state = load_from_postgres(
            &client,
            shard_id,
            shard_count,
            max_symbols,
        )
        .await?;
        shard.load_state(state);
        info!("loaded accounts from postgres (warm candidate)");
        Ok::<_, Box<dyn std::error::Error>>(client)
    })?;

    let wal_dir = env::var("RSX_RISK_WAL_DIR")
        .unwrap_or_else(|_| "./tmp/wal".into());
    let symbol_ids: Vec<u32> =
        (0..max_symbols as u32).collect();
    let replayed = replay_from_wal(
        &mut shard,
        std::path::Path::new(&wal_dir),
        &symbol_ids,
    )?;
    if replayed > 0 {
        info!("replayed {} fills from boot wal", replayed);
    }

    // Resolve ME topology up front: the warm replica must catch
    // up the SAME ME source the live main consumes. The live main
    // binds ONE CastReceiver for all MEs (single recv addr) and
    // uses the first ME's symbol_id as the replication stream_id
    // for FAULTED replay. We match that: ONE ReplicationConsumer
    // against RSX_ME_REPLICATION_ADDR with that stream_id.
    let me_addrs = rsx_risk::me_cast_addrs_from_env();
    if me_addrs.is_empty() {
        return Err("no ME cast addresses configured".into());
    }
    // SAFETY: me_addrs.is_empty() checked above
    let me_stream_id = me_addrs
        .keys()
        .next()
        .copied()
        .expect("INVARIANT: me_addrs non-empty (checked above)");
    let me_repl_addr = env::var("RSX_ME_REPLICATION_ADDR")
        .expect(
            "RSX_ME_REPLICATION_ADDR required (ME's replication \
             server — the warm-catchup + FAULTED replay source)",
        );

    // Mark stream addresses (consumed in warm AND live).
    let mark_addr: SocketAddr =
        env::var("RSX_RISK_MARK_CAST_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9105".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_RISK_MARK_CAST_ADDR");
    let mark_sender_addr: SocketAddr =
        env::var("RSX_MARK_CAST_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9106".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_MARK_CAST_ADDR");
    let mut mark_receiver = CastReceiver::new(
        mark_addr,
        mark_sender_addr,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind mark cast receiver");

    // ===== WARM CATCHUP =====
    // Consume the main's authoritative ME replication stream into
    // the shard until caught up, then win the non-blocking lock.
    // Blocks (re-trying the lock) until promotion. On error,
    // returns it to main()'s restart loop.
    let mut state = NodeState::WarmCatchup;
    gauges.state_idx.store(1, Ordering::Relaxed); // "warm_catchup"
    gauges.ready.store(false, Ordering::Relaxed);
    info!("risk shard {} state={:?}", shard_id, state);
    run_warm_catchup(
        &rt,
        &pg_client,
        &mut lease,
        &mut shard,
        &mut mark_receiver,
        me_stream_id,
        &me_repl_addr,
        &wal_dir,
        lease_poll_interval_ms,
    )?;
    // Past this point this node is the SOLE advisory-lock holder
    // (invariant #10) and the shard is warm + final-drained.
    state = NodeState::Live;
    gauges.state_idx.store(2, Ordering::Relaxed); // "live"
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

    let (persist_prod, persist_cons) =
        rtrb::RingBuffer::<PersistEvent>::new(8192);
    gauges.persist_ring_cap.store(8192, Ordering::Relaxed);
    shard.set_persist_producer(persist_prod);

    // Persist worker thread. We retain its `JoinHandle` and
    // a shutdown flag so that a demote can stop the worker
    // cleanly before returning — otherwise a demote →
    // re-acquire cycle leaks worker threads, each holding its
    // own PG connection.
    let persist_shutdown = Arc::new(AtomicBool::new(false));
    let persist_handle = {
        let url = db_url.clone();
        let sid = shard_id;
        let shutdown = persist_shutdown.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder
                ::new_current_thread()
                .enable_all()
                .build()
                // SAFETY: fail-fast at startup
                .expect("tokio rt");
            rt.block_on(async move {
                let (client, connection) =
                    tokio_postgres::connect(
                        &url,
                        tokio_postgres::NoTls,
                    )
                    .await
                    // SAFETY: fail-fast at startup
                    .expect("pg connect for persist");
                tokio::spawn(drive_pg_connection(connection));
                run_persist_worker_with_shutdown(
                    persist_cons,
                    client,
                    sid,
                    Some(shutdown),
                )
                .await;
            });
        })
    };

    if let Ok(core_str) =
        env::var("RSX_RISK_CORE_ID")
    {
        if let Ok(core_id) =
            core_str.parse::<usize>()
        {
            let setup = rsx_types::cpu::setup_hot_thread(core_id);
            info!("risk {}", setup);
            if setup.isolated == Some(false) {
                tracing::warn!(
                    "risk core {} not isolated — expect tail spikes",
                    core_id
                );
            }
        }
    }

    let risk_addr: SocketAddr =
        env::var("RSX_RISK_CAST_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9101".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_RISK_CAST_ADDR");
    let gw_addr: SocketAddr =
        env::var("RSX_GW_CAST_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9102".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_GW_CAST_ADDR");

    let mut gw_receiver = CastReceiver::new(
        risk_addr, gw_addr,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind risk cast receiver");

    // Receive fills/events from ME (separate port).
    // All MEs send to this single recv addr.
    let risk_me_recv_addr: SocketAddr =
        env::var("RSX_RISK_ME_RECV_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:28301".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_RISK_ME_RECV_ADDR");
    // Use first ME addr as the cast peer for the receiver.
    // me_stream_id (resolved before warm catchup) = symbol_id of
    // the first (primary) ME; used as stream_id in FAULTED TCP
    // replay requests.
    // SAFETY: me_addrs.is_empty() checked before warm catchup
    let first_me_addr = me_addrs
        .values()
        .next()
        .copied()
        .expect("INVARIANT: me_addrs non-empty (checked above)");
    let mut me_receiver = CastReceiver::new(
        risk_me_recv_addr,
        first_me_addr,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind ME fill receiver");

    // mark_receiver was bound + drained during warm catchup; the
    // same socket carries on into the live loop below.

    // Send validated orders to ME.
    // One CastSender per ME, keyed by symbol_id.
    let me_send_bind: Option<String> =
        env::var("RSX_RISK_ME_SEND_ADDR").ok();
    let me_sender_cfg = CastConfig {
        sender_bind_addr: me_send_bind,
        ..Default::default()
    };
    let mut me_senders: HashMap<u32, CastSender> =
        HashMap::new();
    for (&sid, &addr) in &me_addrs {
        let sender = CastSender::with_config(
            addr,
            0,
            Path::new(&wal_dir),
            &me_sender_cfg,
        )
        // SAFETY: fail-fast at startup
        .expect("failed to create ME cast sender");
        me_senders.insert(sid, sender);
    }

    // Send responses to Gateway
    let mut gw_sender = CastSender::new(
        gw_addr,
        0,
        Path::new(&wal_dir),
    )
    // SAFETY: fail-fast at startup
    .expect("failed to create GW cast sender");

    // Egress SPSC rings. Input rings removed: the recv loop
    // calls shard.process_* directly (same thread). These two
    // carry shard output back to this loop's casting senders.
    let (resp_prod, mut resp_cons) =
        rtrb::RingBuffer::<OrderResponse>::new(2048);
    let (accepted_prod, mut accepted_cons) =
        rtrb::RingBuffer::<OrderRequest>::new(2048);
    gauges.resp_ring_cap.store(2048, Ordering::Relaxed);
    gauges.accept_ring_cap.store(2048, Ordering::Relaxed);

    let mut rings = ShardRings {
        response_producer: resp_prod,
        accepted_producer: accepted_prod,
    };

    info!("risk shard {} running state={:?}", shard_id, state);

    loop {
        let now_secs = time();

        // Events from ME (fills, BBO, order lifecycle) are
        // drained BEFORE orders so an order's pre-trade margin
        // check sees margin freed by this tick's fills/releases
        // (capital efficiency; fills-before-orders invariant).
        loop {
            let recv = me_receiver.try_recv_with(|hdr, payload| {
            match hdr.record_type {
                RECORD_BBO => if let Some(rec) =
                    decode_payload::<BboRecord>(payload)
                {
                    // BBO is a "latest wins" state snapshot.
                    // Stash (coalesces per symbol); the tick
                    // drains + runs the per-BBO margin scan.
                    shard.stash_bbo(BboUpdate {
                        seq: rec.seq,
                        symbol_id: rec.symbol_id,
                        bid_px: rec.bid_px.0,
                        bid_qty: rec.bid_qty.0,
                        ask_px: rec.ask_px.0,
                        ask_qty: rec.ask_qty.0,
                    });
                    // BBO is consumed only by risk (margin scan). It is
                    // NOT forwarded to the gateway: the gateway has no BBO
                    // handler, and forward_to_gw re-sequences with gw's own
                    // counter, so dropping it leaves no seq gap. Public BBO
                    // reaches clients via the marketdata process.
                }
                RECORD_FILL => if let Some(fill) =
                    decode_payload::<FillRecord>(payload)
                {
                    // F4.3 — per-stage latency trace.
                    // Stage `risk_out` = fill received from
                    // ME and about to be forwarded to gateway.
                    // Anchor against the taker's gateway-ingress
                    // timestamp (fill.taker_ts_ns) so this delta
                    // composes with gateway_in / risk_in / me_in
                    // on the same clock origin. Fall back to
                    // ts_ns if the field is missing (legacy WAL
                    // record predating the FillRecord layout
                    // change).
                    rsx_log::latency_sample!(
                        "risk_out",
                        fill.taker_order_id_hi,
                        fill.taker_order_id_lo,
                        if fill.taker_ts_ns == 0 {
                            fill.ts_ns
                        } else {
                            fill.taker_ts_ns
                        }
                    );
                    // Fills are correctness-critical:
                    // position == sum(fills). Process
                    // directly on this thread (no input
                    // ring) — process_fill only touches
                    // shard state and the persist ring,
                    // which has its own stall path.
                    let event = FillEvent {
                        seq: fill.seq,
                        symbol_id: fill.symbol_id,
                        taker_user_id: fill
                            .taker_user_id,
                        maker_user_id: fill
                            .maker_user_id,
                        price: fill.price.0,
                        qty: fill.qty.0,
                        taker_side: fill.taker_side,
                        timestamp_ns: fill.ts_ns,
                    };
                    shard.process_fill(&event);
                    gauges.fills_processed.fetch_add(
                        1, Ordering::Relaxed,
                    );
                    // LIQUIDATOR.md §10: a fill means the
                    // symbol is accepting orders again, so any
                    // halt from a prior ORDER_FAILED is lifted.
                    shard.resume_liquidation(fill.symbol_id);
                    // Forward fill to GW
                    forward_to_gw(&mut gw_sender, RECORD_FILL, payload);
                    // Sub-stage: cast send to gateway completed.
                    // Anchor on the same taker_ts_ns used by
                    // risk_out (with the >2024 plausibility
                    // guard).
                    rsx_log::latency_sample!(
                        "risk_cast_send_done",
                        fill.taker_order_id_hi,
                        fill.taker_order_id_lo,
                        if fill.taker_ts_ns == 0 {
                            fill.ts_ns
                        } else {
                            fill.taker_ts_ns
                        }
                    );
                }
                RECORD_ORDER_DONE => if let Some(rec) =
                    decode_payload::<OrderDoneRecord>(payload)
                {
                    shard.release_frozen_for_order(
                        rec.user_id,
                        rec.order_id_hi,
                        rec.order_id_lo,
                    );
                    forward_to_gw(&mut gw_sender, RECORD_ORDER_DONE, payload);
                }
                RECORD_ORDER_CANCELLED => if let Some(rec) =
                    decode_payload::<OrderCancelledRecord>(payload)
                {
                    shard.release_frozen_for_order(
                        rec.user_id,
                        rec.order_id_hi,
                        rec.order_id_lo,
                    );
                    forward_to_gw(&mut gw_sender, RECORD_ORDER_CANCELLED, payload);
                }
                RECORD_ORDER_INSERTED => if decode_payload::<OrderInsertedRecord>(payload).is_some() {
                    forward_to_gw(&mut gw_sender, RECORD_ORDER_INSERTED, payload);
                }
                RECORD_ORDER_ACCEPTED => if let Some(rec) =
                    decode_payload::<OrderAcceptedRecord>(payload)
                {
                    // ME confirmed the order: now (and only now)
                    // write the durable freeze. Reduce-only orders
                    // reserve no margin, so nothing to persist.
                    if rec.reduce_only == 0 {
                        shard.confirm_freeze(
                            rec.user_id,
                            rec.order_id_hi,
                            rec.order_id_lo,
                            rec.symbol_id,
                        );
                    }
                }
                RECORD_ORDER_FAILED => if let Some(rec) =
                    decode_payload::<OrderFailedRecord>(payload)
                {
                    shard.release_frozen_for_order(
                        rec.user_id,
                        rec.order_id_hi,
                        rec.order_id_lo,
                    );
                    // LIQUIDATOR.md §10: order rejected (symbol
                    // halted) pauses liquidation for that symbol.
                    // OrderFailedRecord carries no symbol_id, so
                    // halt the symbols this user is being
                    // liquidated on (the failed order is one).
                    shard.halt_liquidation_for_user(rec.user_id);
                    forward_to_gw(&mut gw_sender, RECORD_ORDER_FAILED, payload);
                }
                RECORD_CONFIG_APPLIED => if let Some(rec) =
                    decode_payload::<ConfigAppliedRecord>(payload)
                {
                    shard.process_config_applied(
                        rec.symbol_id,
                        rec.config_version,
                    );
                    info!(
                        "config_applied: symbol={} v={}",
                        rec.symbol_id,
                        rec.config_version,
                    );
                    forward_to_gw(&mut gw_sender, RECORD_CONFIG_APPLIED, payload);
                }
                _ => {}
            }
            });
            match recv {
                CastRecvWith::Data => {}
                CastRecvWith::Empty => break,
                CastRecvWith::Faulted {
                    last_delivered_seq,
                    gap_start,
                    gap_end_inclusive,
                } => {
                    let new_tip = handle_replay(
                        "risk.me",
                        "RSX_ME_REPLICATION_ADDR",
                        me_stream_id,
                        last_delivered_seq,
                        Some((gap_start, gap_end_inclusive)),
                        &wal_dir,
                    );
                    me_receiver.reset_after_replay(new_tip);
                }
                CastRecvWith::Reconnect { last_delivered_seq } => {
                    let new_tip = handle_replay(
                        "risk.me",
                        "RSX_ME_REPLICATION_ADDR",
                        me_stream_id,
                        last_delivered_seq,
                        None,
                        &wal_dir,
                    );
                    me_receiver.reset_after_replay(new_tip);
                }
            }
        }

        // Orders/cancels from Gateway.
        loop {
            // Egress backpressure: if either output ring is
            // full a processed order's response/accepted push
            // would have nowhere to go. Stop draining the
            // socket this iteration — the kernel recv buffer
            // absorbs, the main loop drains the output rings
            // below, and we resume next iteration. Same end
            // state as the old order_prod stall, no drop.
            if rings.response_producer.is_full()
                || rings.accepted_producer.is_full()
            {
                break;
            }
            let recv = gw_receiver.try_recv_with(|hdr, payload| {
            match hdr.record_type {
                RECORD_ORDER_REQUEST => if let Some(order) =
                    decode_payload::<OrderRequest>(payload)
                {
                    // F4.3 — per-stage latency trace.
                    // Stage `risk_in` = order arrived from
                    // gateway. t_us measured against the
                    // gateway's submit timestamp.
                    rsx_log::latency_sample!(
                        "risk_in",
                        order.order_id_hi,
                        order.order_id_lo,
                        order.timestamp_ns
                    );
                    // Process directly on this thread (no input
                    // ring). The loop head guarantees both output
                    // rings have a free slot, so neither push can
                    // fail — a dropped response/accepted would be
                    // a silent ghost order (gateway thinks it's
                    // pending, ME never sees it). R-N2.
                    gauges.orders_processed.fetch_add(
                        1, Ordering::Relaxed,
                    );
                    let resp = shard.process_order(&order);
                    if matches!(
                        resp,
                        OrderResponse::Accepted { .. }
                    ) {
                        rings.accepted_producer
                            .push(order)
                            .expect(
                                "INVARIANT: accepted_producer \
                                 capacity checked at loop head",
                            );
                    }
                    rings.response_producer
                        .push(resp)
                        .expect(
                            "INVARIANT: response_producer \
                             capacity checked at loop head",
                        );
                }
                RECORD_CANCEL_REQUEST => if let Some(cancel) =
                    decode_payload::<CancelRequest>(payload)
                {
                    // Forward cancel to correct ME.
                    if let Some(s) = me_senders
                        .get_mut(&cancel.symbol_id)
                    {
                        if let Err(e) = s.send_raw(
                            RECORD_CANCEL_REQUEST,
                            payload,
                        ) {
                            warn!("risk: forward cancel to me failed: {e}");
                        }
                    } else {
                        warn!(
                            "cancel for unknown \
                             symbol_id={}",
                            cancel.symbol_id
                        );
                    }
                }
                _ => {}
            }
            });
            match recv {
                CastRecvWith::Data => {}
                CastRecvWith::Empty => break,
                CastRecvWith::Faulted {
                    last_delivered_seq,
                    gap_start,
                    gap_end_inclusive,
                } => {
                    let new_tip = handle_replay(
                        "risk.gw",
                        "RSX_GW_REPLICATION_ADDR",
                        shard_id,
                        last_delivered_seq,
                        Some((gap_start, gap_end_inclusive)),
                        &wal_dir,
                    );
                    gw_receiver.reset_after_replay(new_tip);
                }
                CastRecvWith::Reconnect { last_delivered_seq } => {
                    let new_tip = handle_replay(
                        "risk.gw",
                        "RSX_GW_REPLICATION_ADDR",
                        shard_id,
                        last_delivered_seq,
                        None,
                        &wal_dir,
                    );
                    gw_receiver.reset_after_replay(new_tip);
                }
            }
        }

        // Mark prices from Mark process
        loop {
            let recv = mark_receiver.try_recv_with(|preamble, payload| {
            if preamble.record_type == RECORD_MARK_PRICE {
                if let Some(rec) = decode_payload::<MarkPriceRecord>(payload) {
                    // Latest-wins state; applied directly on the
                    // tile (no ring — recv and shard share a thread).
                    shard.update_mark(rec.symbol_id, rec.mark_price.0);
                }
            }
            });
            match recv {
                CastRecvWith::Data => {}
                CastRecvWith::Empty => break,
                CastRecvWith::Faulted {
                    last_delivered_seq,
                    gap_start,
                    gap_end_inclusive,
                } => {
                    let new_tip = handle_replay(
                        "risk.mark",
                        "RSX_MARK_REPLICATION_ADDR",
                        shard_id,
                        last_delivered_seq,
                        Some((gap_start, gap_end_inclusive)),
                        &wal_dir,
                    );
                    mark_receiver.reset_after_replay(new_tip);
                }
                CastRecvWith::Reconnect { last_delivered_seq } => {
                    let new_tip = handle_replay(
                        "risk.mark",
                        "RSX_MARK_REPLICATION_ADDR",
                        shard_id,
                        last_delivered_seq,
                        None,
                        &wal_dir,
                    );
                    mark_receiver.reset_after_replay(new_tip);
                }
            }
        }

        // Periodic risk work (liquidation sweep, tip persist,
        // stashed-BBO drain) — inputs are processed directly in
        // the recv handlers above.
        shard.tick(&mut rings, now_secs);

        // Publish load gauges (relaxed stores; once per loop
        // iteration, not per message — zero hot-path syscall cost).
        gauges.resp_ring_used.store(
            resp_cons.slots() as u64,
            Ordering::Relaxed,
        );
        gauges.accept_ring_used.store(
            accepted_cons.slots() as u64,
            Ordering::Relaxed,
        );

        // Drain responses: send ORDER_FAILED to GW
        while let Ok(resp) = resp_cons.pop() {
            if let OrderResponse::Rejected {
                user_id,
                reason,
                order_id_hi,
                order_id_lo,
            } = resp
            {
                gauges.rejects.fetch_add(1, Ordering::Relaxed);
                let reason_u8 = match reason {
                    RejectReason::InsufficientMargin => {
                        FailureReason::InsufficientMargin as u8
                    }
                    RejectReason::UserInLiquidation => {
                        FailureReason::UserInLiquidation as u8
                    }
                    RejectReason::NotInShard => {
                        FailureReason::WrongShard as u8
                    }
                };
                let rec = OrderFailedRecord {
                    seq: 0,
                    ts_ns: now_secs * 1_000_000_000,
                    user_id,
                    _pad0: 0,
                    order_id_hi,
                    order_id_lo,
                    reason: reason_u8,
                    _pad: [0; 23],
                };
                // SEQ-1: was send_raw with the record's seq=0,
                // which the gateway drops (seq==0) AND which has
                // no place in the forwarded-ME seq space. Route
                // through forward_to_gw so it gets gw_sender's
                // next contiguous seq like every other gw record.
                forward_to_gw(
                    &mut gw_sender,
                    RECORD_ORDER_FAILED,
                    as_bytes(&rec),
                );
            }
        }

        // Drain accepted orders -> cast to correct ME
        while let Ok(order) = accepted_cons.pop() {
            let msg = OrderMessage {
                seq: order.seq,
                price: order.price,
                qty: order.qty,
                side: order.side,
                tif: order.tif,
                reduce_only: if order.reduce_only {
                    1
                } else {
                    0
                },
                post_only: if order.post_only {
                    1
                } else {
                    0
                },
                _pad1: [0; 4],
                user_id: order.user_id,
                _pad2: 0,
                timestamp_ns: order.timestamp_ns,
                order_id_hi: order.order_id_hi,
                order_id_lo: order.order_id_lo,
            };
            let send_failed = match me_senders
                .get_mut(&order.symbol_id)
            {
                Some(s) => {
                    if let Err(e) = s.send_raw(
                        RECORD_ORDER_REQUEST,
                        as_bytes(&msg),
                    ) {
                        warn!("risk: forward order to me failed: {e}");
                        true
                    } else {
                        false
                    }
                }
                None => {
                    warn!(
                        "order for unknown symbol_id={}",
                        order.symbol_id
                    );
                    true
                }
            };
            // The pre-trade gate already froze margin in-memory for
            // this accepted order. If ME never receives it, no
            // RECORD_ORDER_ACCEPTED returns → the freeze (and the
            // gateway's pending order) would leak forever. Release
            // the in-memory freeze and tell the client it failed.
            // (The durable PG freeze is only written by confirm_freeze
            // on OrderAccepted, so there is nothing durable to undo.)
            if send_failed {
                shard.release_frozen_for_order(
                    order.user_id,
                    order.order_id_hi,
                    order.order_id_lo,
                );
                let rec = OrderFailedRecord {
                    seq: 0,
                    ts_ns: now_secs * 1_000_000_000,
                    user_id: order.user_id,
                    _pad0: 0,
                    order_id_hi: order.order_id_hi,
                    order_id_lo: order.order_id_lo,
                    reason: FailureReason::NetworkError as u8,
                    _pad: [0; 23],
                };
                forward_to_gw(
                    &mut gw_sender,
                    RECORD_ORDER_FAILED,
                    as_bytes(&rec),
                );
            }
        }

        // Cast housekeeping
        for s in me_senders.values_mut() {
            if let Err(e) = s.tick() {
                warn!("risk: me_sender tick failed: {e}");
            }
            s.recv_control();
        }
        if let Err(e) = gw_sender.tick() {
            warn!("risk: gw_sender tick failed: {e}");
        }
        gw_sender.recv_control();

        // Check lease health (non-blocking — lease thread updates atomically).
        // On loss we tear down the persist worker + lease thread (each owns a
        // PG connection) and return so main()'s loop re-enters run_main, which
        // re-blocks on the advisory lock — this process becomes a standby.
        if !lease_held.load(Ordering::Relaxed) {
            stop_persist_worker(&persist_shutdown, persist_handle);
            stop_lease_thread(&lease_stop, lease_thread);
            if lease_error.load(Ordering::Relaxed) {
                return Err(
                    "lease check failed after 3 consecutive errors".into()
                );
            } else {
                warn!("lease lost, re-acquiring advisory lock");
                return Ok(MainTransition::Demote);
            }
        }
    }
}

/// WARM CATCHUP (NodeState::WarmCatchup).
///
/// Consume the live main's authoritative ME WAL replication
/// stream (the SAME source `handle_replay` uses for FAULTED
/// recovery — no separate risk WAL) into the already-PG-loaded
/// shard, applying each record via the shared `apply_record`
/// path. Also drain the mark stream into `update_mark`. NO
/// persist worker, NO gateway ingress/egress, NO liquidation
/// tick — this node is a passive follower.
///
/// CAUGHT-UP detection: the replication server emits
/// RECORD_CAUGHT_UP { live_seq } after draining its current WAL.
/// `caught_up` ⟺ we have seen that record AND `applied_seq >=
/// live_seq`. We open the consumer with a per-node tip file so a
/// reconnect resumes from the persisted tip+1 (CAUGHT_UP itself
/// carries no seq, so it never advances the tip).
///
/// PROMOTE: only when caught up do we call the NON-BLOCKING
/// `pg_try_advisory_lock`. If it fails another node holds the
/// lock — stay warm and retry after `lease_poll_interval_ms`. If
/// it succeeds this node is the sole holder (invariant #10); we
/// do a FINAL DRAIN of any records past the last CAUGHT_UP and
/// return Ok — the caller transitions to LIVE with the warm
/// shard (no rebuild). The advisory lock is the SOLE single-main
/// fence; catch-up only gates WHEN `try_acquire` is called.
///
/// ME topology: the live main binds ONE CastReceiver for all MEs
/// and replays a single stream_id, so this is ONE
/// ReplicationConsumer — matching that topology.
#[allow(clippy::too_many_arguments)]
fn run_warm_catchup(
    rt: &tokio::runtime::Runtime,
    pg_client: &tokio_postgres::Client,
    lease: &mut AdvisoryLease,
    shard: &mut RiskShard,
    mark_receiver: &mut CastReceiver,
    me_stream_id: u32,
    me_repl_addr: &str,
    wal_dir: &str,
    lease_poll_interval_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let tip_file = std::path::PathBuf::from(wal_dir).join(
        format!("risk_warm_me_{me_stream_id}_tip.bin"),
    );
    let mut consumer = ReplicationConsumer::new(
        me_stream_id,
        vec![me_repl_addr.to_owned()],
        tip_file,
        None,
    )?;
    // Resume the ME stream from the shard's persisted per-symbol
    // tip so we don't re-request records already folded into the
    // PG snapshot (process_fill still dedups on tip — invariant
    // #5 — but skipping the re-request is cheaper).
    if (me_stream_id as usize) < shard.tips.len() {
        consumer.tip = shard.tips[me_stream_id as usize];
    }

    info!(
        "warm catchup: consuming ME replication \
         stream_id={me_stream_id} from {me_repl_addr} tip={}",
        consumer.tip,
    );

    let mut applied_seq: u64 = consumer.tip;
    let poll = Duration::from_millis(lease_poll_interval_ms.max(1));

    loop {
        // Drain mark prices (latest-wins state) each iteration so
        // the warm shard's margin view stays fresh; mark gaps are
        // recoverable (latest-wins) so we ignore FAULTED here.
        drain_mark_warm(mark_receiver, shard);

        // Stream ME records until CAUGHT_UP (callback returns
        // false) or the connection ends. `caught_live_seq` is set
        // by the callback when it sees RECORD_CAUGHT_UP.
        let mut caught_live_seq: Option<u64> = None;
        let stream = rt.block_on(consumer.run_once(|raw| {
            if raw.header.record_type == RECORD_CAUGHT_UP {
                if let Some(rec) =
                    decode_payload::<CaughtUpRecord>(&raw.payload)
                {
                    caught_live_seq = Some(rec.live_seq);
                }
                // Stop the stream so the outer loop can poll the
                // lock; reconnect resumes from tip+1.
                return false;
            }
            let seq = extract_seq(&raw.payload).unwrap_or(0);
            if seq > applied_seq {
                applied_seq = seq;
            }
            apply_record(
                shard,
                raw.header.record_type,
                &raw.payload,
            );
            true
        }));

        if let Err(e) = stream {
            // RECORD_REPLICATION_NOT_AVAILABLE maps to NotFound.
            // When consumer.tip > 0 this means ME cannot serve our
            // current tip+1, most likely because ME just restarted
            // with an empty WAL (my_highest=0). Our PG snapshot
            // already covers everything up to consumer.tip, so we
            // are ahead of ME — treat as caught up and proceed to
            // lock acquisition.
            if e.kind() == std::io::ErrorKind::NotFound
                && consumer.tip > 0
            {
                info!(
                    "warm catchup: ME WAL behind our tip={} \
                     (fresh boot or GC'd tail); \
                     PG snapshot is current — treating as caught up",
                    consumer.tip,
                );
                // Synthesize the caught-up condition so the lock
                // attempt below fires normally.
                caught_live_seq = Some(0);
                applied_seq = consumer.tip;
            } else {
                // Disconnect/error clears caught_up implicitly (we
                // re-derive it next iteration). Back off then retry;
                // the consumer reconnects from its persisted tip+1.
                warn!(
                    "warm catchup: ME stream error: {e}; \
                     retry in {}ms",
                    poll.as_millis(),
                );
                std::thread::sleep(poll);
                continue;
            }
        }

        let caught_up = match caught_live_seq {
            Some(live_seq) => applied_seq >= live_seq,
            None => false,
        };

        if !caught_up {
            // Connection ended without CAUGHT_UP, or we are
            // behind the reported live_seq. Loop to re-stream
            // (resumes from tip+1) — no lock attempt.
            continue;
        }

        // Caught up: attempt the NON-BLOCKING lock. This is the
        // ONLY place try_acquire is called; the lock — not
        // catch-up — is the single-main fence (invariant #10).
        let acquired = rt
            .block_on(lease.try_acquire(pg_client))?;
        if !acquired {
            // Another node is main. Stay warm; keep applying.
            std::thread::sleep(poll);
            continue;
        }

        info!(
            "warm catchup: caught up (applied_seq={applied_seq}) \
             AND won advisory lock — final drain then go LIVE",
        );

        // FINAL DRAIN: between the last CAUGHT_UP and winning the
        // lock the main may have written more records. Apply
        // everything up to the current WAL tip so the live loop
        // starts with no gap. One more run_once: stream to the
        // next CAUGHT_UP and stop.
        let final_drain = rt.block_on(consumer.run_once(|raw| {
            if raw.header.record_type == RECORD_CAUGHT_UP {
                return false;
            }
            let seq = extract_seq(&raw.payload).unwrap_or(0);
            if seq > applied_seq {
                applied_seq = seq;
            }
            apply_record(
                shard,
                raw.header.record_type,
                &raw.payload,
            );
            true
        }));
        if let Err(e) = final_drain {
            warn!(
                "warm catchup: final drain stream error: {e} \
                 (applied_seq={applied_seq}); proceeding — the \
                 live ME receiver re-syncs via FAULTED replay",
            );
        }
        drain_mark_warm(mark_receiver, shard);
        return Ok(());
    }
}

/// Drain the mark CastReceiver into the shard during warm
/// catchup. Mark is latest-wins state; FAULTED/RECONNECT are
/// ignored (the next live mark supersedes any gap).
fn drain_mark_warm(
    mark_receiver: &mut CastReceiver,
    shard: &mut RiskShard,
) {
    loop {
        let recv = mark_receiver.try_recv_with(|preamble, payload| {
            if preamble.record_type == RECORD_MARK_PRICE {
                if let Some(rec) =
                    decode_payload::<MarkPriceRecord>(payload)
                {
                    shard.update_mark(
                        rec.symbol_id,
                        rec.mark_price.0,
                    );
                }
            }
        });
        match recv {
            CastRecvWith::Data => {}
            _ => break,
        }
    }
}

/// Signal the persist worker to shut down and wait for the
/// thread to exit. Called from `run_main` before any
/// successful transition out of the Main role so that the
/// next promote spawns a fresh worker without doubling up
/// on PG connections.
fn stop_persist_worker(
    shutdown: &Arc<AtomicBool>,
    handle: std::thread::JoinHandle<()>,
) {
    shutdown.store(true, Ordering::Relaxed);
    // Bounded wait so a stuck worker can't hang the demote. Poll
    // the handle directly (no watchdog thread). The worker drains
    // pending then returns; the typical exit window is
    // FLUSH_INTERVAL (10ms) + one final flush_batch. We give it 5s
    // — well past the worst-case exponential backoff between failed
    // flushes; past that we abandon the thread (cleaned up on
    // process exit).
    let start = std::time::Instant::now();
    loop {
        if handle.is_finished() {
            if let Err(e) = handle.join() {
                warn!(
                    "persist worker thread panicked: {:?}",
                    e,
                );
            }
            info!("persist worker stopped");
            return;
        }
        if start.elapsed() > Duration::from_secs(5) {
            warn!(
                "persist worker did not exit within 5s; \
                 abandoning thread (will be cleaned up on \
                 process exit)",
            );
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn spawn_lease_thread(
    rt: tokio::runtime::Runtime,
    pg_client: tokio_postgres::Client,
    mut lease: rsx_risk::lease::AdvisoryLease,
    renew_interval_secs: u64,
    lease_held: Arc<AtomicBool>,
    lease_error: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        rt.block_on(async move {
            let interval = Duration::from_secs(renew_interval_secs.max(1));
            let mut consec_errors: u32 = 0;
            loop {
                tokio::time::sleep(interval).await;
                if stop.load(Ordering::Relaxed) {
                    if let Err(e) = lease.release(&pg_client).await {
                        warn!("lease release on stop failed: {e}");
                    }
                    return;
                }
                match lease.renew(&pg_client).await {
                    Ok(true) => { consec_errors = 0; }
                    Ok(false) => {
                        warn!("lease lost (shard {})", lease.shard_id());
                        lease_held.store(false, Ordering::Release);
                        return;
                    }
                    Err(e) => {
                        consec_errors += 1;
                        warn!("lease renew error ({}/3): {e}", consec_errors);
                        if consec_errors >= 3 {
                            lease_error.store(true, Ordering::Release);
                            lease_held.store(false, Ordering::Release);
                            return;
                        }
                    }
                }
            }
        });
    })
}

fn stop_lease_thread(
    stop: &Arc<AtomicBool>,
    handle: std::thread::JoinHandle<()>,
) {
    stop.store(true, Ordering::Relaxed);
    if let Err(e) = handle.join() {
        warn!("lease thread panicked on join: {:?}", e);
    }
}
