use rsx_cast::cast::CastRecv;
use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastSender;
use rsx_cast::config::CastConfig;
use std::collections::HashMap;
use rsx_messages::ConfigAppliedRecord;
use rsx_messages::BboRecord;
use rsx_messages::FillRecord;
use rsx_messages::MarkPriceRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_BBO;
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
use rsx_risk::config::load_shard_config;
use rsx_risk::lease::AdvisoryLease;
use rsx_risk::persist::run_persist_worker_with_shutdown;
use rsx_risk::replay::load_from_postgres;
use rsx_risk::schema::run_migrations;
use rsx_risk::replay::replay_from_wal;
use rsx_risk::rings::MarkPriceUpdate;
use rsx_risk::rings::ShardRings;
use rsx_risk::shard::RiskShard;
use rsx_risk::BboUpdate;
use rsx_risk::FillEvent;
use rsx_risk::OrderRequest;
use rsx_risk::OrderResponse;
use rsx_risk::persist::PersistEvent;
use rsx_types::install_panic_handler;
use rsx_types::FailureReason;
use rsx_types::time::time;
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


/// Backoff schedule (seconds) for shard crash-restarts.
const RESTART_BACKOFF_SECS: &[u64] = &[
    5, 10, 20, 40, 60, 60,
];
/// Max consecutive crashes before the shard gives up.
const MAX_RESTARTS: usize = 8;

/// Role the shard process is currently playing. Driven by
/// `main()`'s state-machine loop; `run_replica` returns when
/// it has acquired the advisory lock (→ Main), `run_main`
/// returns when it has lost the lease (→ Replica) or been
/// asked to shut down. No recursion, no env mutation.
#[derive(Clone, Copy, Debug)]
enum Role {
    Replica,
    Main,
}

/// Transition signalled by `run_replica` on return.
#[derive(Debug)]
enum ReplicaTransition {
    /// Advisory lock acquired; main() should switch to Main.
    Promote,
}

/// Transition signalled by `run_main` on return.
#[derive(Debug)]
enum MainTransition {
    /// Advisory lease lost; main() should switch to Replica
    /// and resume polling.
    Demote,
}

fn log_effective_risk_config(
    config: &rsx_risk::ShardConfig,
) {
    info!(
        "risk effective config: shard_id={} shard_count={} max_symbols={} replica={} lease_poll_ms={} lease_renew_ms={} liquidation_base_delay_ns={} liquidation_base_slip_bps={} liquidation_max_rounds={}",
        config.shard_id,
        config.shard_count,
        config.max_symbols,
        config.replication_config.is_replica,
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

    tracing_subscriber::fmt::init();

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
    let initial_is_replica =
        config.replication_config.is_replica;

    info!(
        "risk shard {} starting ({} shards, {} symbols, replica={})",
        shard_id, shard_count, max_symbols, initial_is_replica,
    );
    log_effective_risk_config(&config);

    let mut role = if initial_is_replica {
        Role::Replica
    } else {
        Role::Main
    };
    let mut attempts: usize = 0;

    // State-machine loop. Replaces the prior set_var +
    // recursive run_main pattern (see .ship/13-A16Z-FIXES
    // T3.2). On clean transitions we reset the restart
    // budget — a successful promote/demote isn't a crash.
    loop {
        let err: Box<dyn std::error::Error> = match role {
            Role::Replica => match run_replica(
                shard_id, max_symbols,
            ) {
                Ok(ReplicaTransition::Promote) => {
                    info!("transition: Replica → Main");
                    role = Role::Main;
                    attempts = 0;
                    continue;
                }
                Err(e) => e,
            },
            Role::Main => match run_main(
                shard_id, max_symbols,
            ) {
                Ok(MainTransition::Demote) => {
                    info!("transition: Main → Replica");
                    role = Role::Replica;
                    attempts = 0;
                    continue;
                }
                Err(e) => e,
            },
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
            "crashed in role {:?} ({}/{} attempts): \
             {err}; restart in {sleep_ms}ms",
            role, attempts, MAX_RESTARTS,
        );
        std::thread::sleep(Duration::from_millis(
            sleep_ms.max(100) as u64,
        ));
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
) -> Result<MainTransition, Box<dyn std::error::Error>> {
    let config = load_shard_config()?;
    let shard_count = config.shard_count;
    let lease_renew_interval_ms = config.replication_config.lease_renew_interval_ms;
    let lease_renew_interval_secs = (lease_renew_interval_ms / 1000).max(1);
    let mut shard = RiskShard::new(config);

    // SAFETY: fail-fast at startup -- risk requires
    // postgres for state persistence and advisory locks
    let db_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL required for risk");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let mut lease = AdvisoryLease::new(shard_id);
    let pg_client = rt.block_on(async {
        let (client, connection) =
            tokio_postgres::connect(
                &db_url,
                tokio_postgres::NoTls,
            )
            .await?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                error!("pg connection error: {e}");
            }
        });
        run_migrations(&client).await?;
        lease.acquire(&client).await?;
        let state = load_from_postgres(
            &client,
            shard_id,
            shard_count,
            max_symbols,
        )
        .await?;
        shard.load_state(state);
        info!("cold start loaded from postgres");
        Ok::<_, Box<dyn std::error::Error>>(client)
    })?;

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
        info!("replayed {} fills from wal", replayed);
    }

    let (persist_prod, persist_cons) =
        rtrb::RingBuffer::<PersistEvent>::new(8192);
    shard.set_persist_producer(persist_prod);

    // Tip sync channel for replica (if replica is running)
    let replica_addr: Option<SocketAddr> =
        env::var("RSX_RISK_REPLICA_ADDR")
            .ok()
            .and_then(|s| s.parse().ok());
    let mut tip_sender = replica_addr.map(|addr| {
        CastSender::new(
            addr,
            0,
            Path::new(&wal_dir),
        )
        // SAFETY: fail-fast at startup
        .expect("failed to create replica tip sender")
    });

    // Persist worker thread. We retain its `JoinHandle` and
    // a shutdown flag so that a demote can stop the worker
    // cleanly before returning — otherwise a Main → Replica
    // → Main cycle leaks worker threads, each holding its
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
                tokio::spawn(async move {
                    if let Err(e) = connection.await {
                        error!(
                            "persist pg error: {e}"
                        );
                    }
                });
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
            let ids = core_affinity::get_core_ids()
                .unwrap_or_default();
            if let Some(id) = ids.get(core_id) {
                core_affinity::set_for_current(*id);
                info!("pinned to core {}", core_id);
            }
        }
    }

    let risk_addr: SocketAddr =
        env::var("RSX_RISK_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9101".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_RISK_CMP_ADDR");
    let gw_addr: SocketAddr =
        env::var("RSX_GW_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9102".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_GW_CMP_ADDR");
    let me_addrs = rsx_risk::me_cmp_addrs_from_env();
    if me_addrs.is_empty() {
        stop_persist_worker(
            &persist_shutdown,
            persist_handle,
        );
        stop_lease_thread(&lease_stop, lease_thread);
        return Err("no ME CMP addresses configured".into());
    }

    let mut gw_receiver = CastReceiver::new(
        risk_addr, gw_addr, 0,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind risk CMP receiver");

    // Receive fills/events from ME (separate port).
    // All MEs send to this single recv addr.
    let risk_me_recv_addr: SocketAddr =
        env::var("RSX_RISK_ME_RECV_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:28301".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_RISK_ME_RECV_ADDR");
    // Use first ME addr as the CMP peer for the receiver
    // SAFETY: me_addrs.is_empty() checked above
    let first_me_addr = *me_addrs.values().next()
        .expect("INVARIANT: me_addrs non-empty (checked above)");
    let mut me_receiver = CastReceiver::new(
        risk_me_recv_addr,
        first_me_addr,
        0,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind ME fill receiver");

    // Receive mark prices from Mark process
    let mark_addr: SocketAddr =
        env::var("RSX_RISK_MARK_CMP_ADDR")
            .unwrap_or_else(|_| {
                "127.0.0.1:9105".into()
            })
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_RISK_MARK_CMP_ADDR");
    let mark_sender_addr: SocketAddr =
        env::var("RSX_MARK_CMP_ADDR")
            .unwrap_or_else(|_| {
                "127.0.0.1:9106".into()
            })
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_MARK_CMP_ADDR");
    let mut mark_receiver = CastReceiver::new(
        mark_addr,
        mark_sender_addr,
        0,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind mark CMP receiver");

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
        .expect("failed to create ME CMP sender");
        me_senders.insert(sid, sender);
    }

    // Send responses to Gateway
    let mut gw_sender = CastSender::new(
        gw_addr,
        0,
        Path::new(&wal_dir),
    )
    // SAFETY: fail-fast at startup
    .expect("failed to create GW CMP sender");

    // SPSC rings for run_once (internal)
    let (mut fill_prod, fill_cons) =
        rtrb::RingBuffer::<FillEvent>::new(4096);
    let (mut order_prod, order_cons) =
        rtrb::RingBuffer::<OrderRequest>::new(2048);
    let (mut mark_prod, mark_cons) =
        rtrb::RingBuffer::<MarkPriceUpdate>::new(256);
    let (mut bbo_prod, bbo_cons) =
        rtrb::RingBuffer::<BboUpdate>::new(256);
    let (resp_prod, mut resp_cons) =
        rtrb::RingBuffer::<OrderResponse>::new(2048);
    let (accepted_prod, mut accepted_cons) =
        rtrb::RingBuffer::<OrderRequest>::new(2048);

    let mut rings = ShardRings {
        fill_consumers: vec![fill_cons],
        order_consumer: order_cons,
        mark_consumer: mark_cons,
        bbo_consumers: vec![bbo_cons],
        response_producer: resp_prod,
        accepted_producer: accepted_prod,
    };

    info!("risk shard {} running", shard_id);

    // Backpressure counters: spin-stall on full producer ring
    // and surface a WARN each time we yield to shard.run_once
    // so this never silently drops correctness-critical events.
    let mut fill_stalls: u64 = 0;
    let mut order_stalls: u64 = 0;
    let mut bbo_drops: u64 = 0;
    let mut mark_drops: u64 = 0;

    loop {
        let now_secs = time();

        // Orders/cancels from Gateway.
        loop {
            let (hdr, payload) = match gw_receiver.try_recv() {
                CastRecv::Data(h, p) => (h, p),
                CastRecv::Empty => break,
                CastRecv::Faulted {
                    last_delivered_seq,
                    gap_start,
                    gap_end_inclusive,
                } => panic!(
                    "FAULTED: DXS replay path not yet \
                     wired here; see rsx-matching for the \
                     POC reference impl \
                     (last_delivered={} gap=[{}..={}])",
                    last_delivered_seq,
                    gap_start,
                    gap_end_inclusive,
                ),
            };
            {
            match hdr.record_type {
                RECORD_ORDER_REQUEST
                    if payload.len()
                        >= std::mem::size_of::<
                            OrderRequest,
                        >() =>
                {
                    let order = unsafe {
                        std::ptr::read_unaligned(
                            payload.as_ptr()
                                as *const OrderRequest,
                        )
                    };
                    // F4.3 — per-stage latency trace.
                    // Stage `risk_in` = order arrived from
                    // gateway. t_us measured against the
                    // gateway's submit timestamp.
                    {
                        let now_ns = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_nanos() as u64)
                            .unwrap_or(0);
                        let t_us = now_ns
                            .saturating_sub(order.timestamp_ns)
                            / 1000;
                        rsx_log::latency::sample("risk_in", order.order_id_hi, order.order_id_lo, t_us, order.timestamp_ns);
                    }
                    // Stall on full ring rather than dropping —
                    // dropping turns into a silent ghost order
                    // (gateway thinks it's pending, ME never
                    // sees it). Mirror the fill_prod pattern.
                    // R-N2.
                    let now_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let mut pending = order;
                    loop {
                        match order_prod.push(pending) {
                            Ok(()) => break,
                            Err(rtrb::PushError::Full(o)) => {
                                pending = o;
                                order_stalls = order_stalls
                                    .wrapping_add(1);
                                if order_stalls.is_power_of_two() {
                                    warn!(
                                        "order_prod full, \
                                         stalling (count={})",
                                        order_stalls,
                                    );
                                }
                                shard.run_once(
                                    &mut rings,
                                    now_secs,
                                );
                            }
                        }
                    }
                }
                RECORD_CANCEL_REQUEST
                    if payload.len()
                        >= std::mem::size_of::<
                            CancelRequest,
                        >() =>
                {
                    // Forward cancel to correct ME.
                    let cancel = unsafe {
                        std::ptr::read_unaligned(
                            payload.as_ptr()
                                as *const CancelRequest,
                        )
                    };
                    if let Some(s) = me_senders
                        .get_mut(&cancel.symbol_id)
                    {
                        if let Err(e) = s.send_raw(
                            RECORD_CANCEL_REQUEST,
                            &payload,
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
            }
        }

        // Events from ME (fills, BBO, order lifecycle).
        loop {
            let (hdr, payload) = match me_receiver.try_recv() {
                CastRecv::Data(h, p) => (h, p),
                CastRecv::Empty => break,
                CastRecv::Faulted {
                    last_delivered_seq,
                    gap_start,
                    gap_end_inclusive,
                } => panic!(
                    "FAULTED: DXS replay path not yet \
                     wired here; see rsx-matching for the \
                     POC reference impl \
                     (last_delivered={} gap=[{}..={}])",
                    last_delivered_seq,
                    gap_start,
                    gap_end_inclusive,
                ),
            };
            {
            match hdr.record_type {
                RECORD_BBO
                    if payload.len()
                        >= std::mem::size_of::<
                            BboRecord,
                        >() =>
                {
                    let rec = unsafe {
                        std::ptr::read_unaligned(
                            payload.as_ptr()
                                as *const BboRecord,
                        )
                    };
                    // BBO is a "latest wins" state snapshot;
                    // drops are safe but counted so this is
                    // never silent.
                    if bbo_prod.push(BboUpdate {
                        seq: rec.seq,
                        symbol_id: rec.symbol_id,
                        bid_px: rec.bid_px.0,
                        bid_qty: rec.bid_qty.0,
                        ask_px: rec.ask_px.0,
                        ask_qty: rec.ask_qty.0,
                    }).is_err() {
                        bbo_drops =
                            bbo_drops.wrapping_add(1);
                        if bbo_drops.is_power_of_two() {
                            warn!(
                                "bbo_prod ring full, drops={}",
                                bbo_drops,
                            );
                        }
                    }
                    // Forward to GW to maintain CMP seq
                    // continuity (GW ignores BBO content).
                    if let Err(e) = gw_sender.send_raw(
                        RECORD_BBO,
                        &payload,
                    ) {
                        warn!("risk: forward bbo to gw failed: {e}");
                    }
                }
                RECORD_FILL
                    if payload.len()
                        >= std::mem::size_of::<
                            FillRecord,
                        >() =>
                {
                    let fill = unsafe {
                        std::ptr::read_unaligned(
                            payload.as_ptr()
                                as *const FillRecord,
                        )
                    };
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
                    {
                        let now_ns =
                            std::time::SystemTime::now()
                                .duration_since(
                                    std::time::UNIX_EPOCH,
                                )
                                .map(|d| d.as_nanos() as u64)
                                .unwrap_or(0);
                        let anchor_ns =
                            if fill.taker_ts_ns == 0 {
                                fill.ts_ns
                            } else {
                                fill.taker_ts_ns
                            };
                        let t_us = now_ns
                            .saturating_sub(anchor_ns)
                            / 1000;
                        rsx_log::latency::sample("risk_out", fill.taker_order_id_hi, fill.taker_order_id_lo, t_us, anchor_ns);
                    }
                    // Fills are correctness-critical:
                    // position == sum(fills). Stall and
                    // drain via shard.run_once rather than
                    // drop. Bounded retry: SPSC consumer
                    // is in-process so this resolves in a
                    // few iterations.
                    let mut event = FillEvent {
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
                    loop {
                        match fill_prod.push(event) {
                            Ok(()) => break,
                            Err(rtrb::PushError::Full(
                                ev,
                            )) => {
                                event = ev;
                                fill_stalls = fill_stalls
                                    .wrapping_add(1);
                                if fill_stalls
                                    .is_power_of_two()
                                {
                                    warn!(
                                        "fill_prod full, \
                                         stalling (count={})",
                                        fill_stalls,
                                    );
                                }
                                shard.run_once(
                                    &mut rings,
                                    now_secs,
                                );
                            }
                        }
                    }
                    // Forward fill to GW
                    if let Err(e) = gw_sender.send_raw(
                        RECORD_FILL,
                        &payload,
                    ) {
                        warn!("risk: forward fill to gw failed: {e}");
                    }
                    // Sub-stage: CMP send to gateway completed.
                    // Anchor on the same taker_ts_ns used by
                    // risk_out (with the >2024 plausibility
                    // guard).
                    {
                        let now_ns =
                            std::time::SystemTime::now()
                                .duration_since(
                                    std::time::UNIX_EPOCH,
                                )
                                .map(|d| d.as_nanos() as u64)
                                .unwrap_or(0);
                        let anchor_ns =
                            if fill.taker_ts_ns == 0 {
                                fill.ts_ns
                            } else {
                                fill.taker_ts_ns
                            };
                        let t_us = now_ns
                            .saturating_sub(anchor_ns)
                            / 1000;
                        rsx_log::latency::sample("risk_cmp_send_done", fill.taker_order_id_hi, fill.taker_order_id_lo, t_us, anchor_ns);
                    }
                }
                RECORD_ORDER_DONE
                    if payload.len()
                        >= std::mem::size_of::<
                            OrderDoneRecord,
                        >() =>
                {
                    let rec = unsafe {
                        std::ptr::read_unaligned(
                            payload.as_ptr()
                                as *const
                                    OrderDoneRecord,
                        )
                    };
                    shard.release_frozen_for_order(
                        rec.user_id,
                        rec.order_id_hi,
                        rec.order_id_lo,
                    );
                    if let Err(e) = gw_sender.send_raw(
                        RECORD_ORDER_DONE,
                        &payload,
                    ) {
                        warn!("risk: forward order_done to gw failed: {e}");
                    }
                }
                RECORD_ORDER_CANCELLED
                    if payload.len()
                        >= std::mem::size_of::<
                            OrderCancelledRecord,
                        >() =>
                {
                    let rec = unsafe {
                        std::ptr::read_unaligned(
                            payload.as_ptr()
                                as *const
                                    OrderCancelledRecord,
                        )
                    };
                    shard.release_frozen_for_order(
                        rec.user_id,
                        rec.order_id_hi,
                        rec.order_id_lo,
                    );
                    if let Err(e) = gw_sender.send_raw(
                        RECORD_ORDER_CANCELLED,
                        &payload,
                    ) {
                        warn!("risk: forward order_cancelled to gw failed: {e}");
                    }
                }
                RECORD_ORDER_INSERTED
                    if payload.len()
                        >= std::mem::size_of::<
                            OrderInsertedRecord,
                        >() =>
                {
                    if let Err(e) = gw_sender.send_raw(
                        RECORD_ORDER_INSERTED,
                        &payload,
                    ) {
                        warn!("risk: forward order_inserted to gw failed: {e}");
                    }
                }
                RECORD_ORDER_FAILED
                    if payload.len()
                        >= std::mem::size_of::<
                            OrderFailedRecord,
                        >() =>
                {
                    let rec = unsafe {
                        std::ptr::read_unaligned(
                            payload.as_ptr()
                                as *const
                                    OrderFailedRecord,
                        )
                    };
                    shard.release_frozen_for_order(
                        rec.user_id,
                        rec.order_id_hi,
                        rec.order_id_lo,
                    );
                    if let Err(e) = gw_sender.send_raw(
                        RECORD_ORDER_FAILED,
                        &payload,
                    ) {
                        warn!("risk: forward order_failed to gw failed: {e}");
                    }
                }
                RECORD_CONFIG_APPLIED
                    if payload.len()
                        >= std::mem::size_of::<
                            ConfigAppliedRecord,
                        >() =>
                {
                    let rec = unsafe {
                        std::ptr::read_unaligned(
                            payload.as_ptr()
                                as *const
                                    ConfigAppliedRecord,
                        )
                    };
                    shard.process_config_applied(
                        rec.symbol_id,
                        rec.config_version,
                    );
                    info!(
                        "config_applied: symbol={} v={}",
                        rec.symbol_id,
                        rec.config_version,
                    );
                    if let Err(e) = gw_sender.send_raw(
                        RECORD_CONFIG_APPLIED,
                        &payload,
                    ) {
                        warn!("risk: forward config_applied to gw failed: {e}");
                    }
                }
                _ => {}
            }
            }
        }

        // Mark prices from Mark process
        loop {
            let (preamble, payload) = match mark_receiver
                .try_recv()
            {
                CastRecv::Data(h, p) => (h, p),
                CastRecv::Empty => break,
                CastRecv::Faulted {
                    last_delivered_seq,
                    gap_start,
                    gap_end_inclusive,
                } => panic!(
                    "FAULTED: DXS replay path not yet \
                     wired here; see rsx-matching for the \
                     POC reference impl \
                     (last_delivered={} gap=[{}..={}])",
                    last_delivered_seq,
                    gap_start,
                    gap_end_inclusive,
                ),
            };
            {
            if preamble.record_type == RECORD_MARK_PRICE
                && payload.len()
                    >= std::mem::size_of::<
                        MarkPriceRecord,
                    >()
            {
                let rec = unsafe {
                    std::ptr::read_unaligned(
                        payload.as_ptr()
                            as *const MarkPriceRecord,
                    )
                };
                // Mark price is "latest wins" state;
                // drops are safe but counted so this is
                // never silent.
                if mark_prod.push(MarkPriceUpdate {
                    seq: rec.seq,
                    symbol_id: rec.symbol_id,
                    price: rec.mark_price.0,
                }).is_err() {
                    mark_drops =
                        mark_drops.wrapping_add(1);
                    if mark_drops.is_power_of_two() {
                        warn!(
                            "mark_prod ring full, drops={}",
                            mark_drops,
                        );
                    }
                }
            }
            }
        }

        // Run risk engine
        shard.run_once(&mut rings, now_secs);

        // Drain responses: send ORDER_FAILED to GW
        while let Ok(resp) = resp_cons.pop() {
            if let OrderResponse::Rejected {
                user_id,
                reason,
                order_id_hi,
                order_id_lo,
            } = resp
            {
                let reason_u8 = match reason {
                    rsx_risk::RejectReason::InsufficientMargin => {
                        FailureReason::InsufficientMargin
                            as u8
                    }
                    rsx_risk::RejectReason::UserInLiquidation => {
                        FailureReason::UserInLiquidation as u8
                    }
                    rsx_risk::RejectReason::NotInShard => {
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
                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        &rec as *const OrderFailedRecord
                            as *const u8,
                        std::mem::size_of::<
                            OrderFailedRecord,
                        >(),
                    )
                };
                if let Err(e) = gw_sender.send_raw(
                    RECORD_ORDER_FAILED,
                    bytes,
                ) {
                    warn!("risk: send order_failed to gw failed: {e}");
                }
            }
        }

        // Drain accepted orders -> CMP to correct ME
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
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    &msg as *const OrderMessage
                        as *const u8,
                    std::mem::size_of::<
                        OrderMessage,
                    >(),
                )
            };
            if let Some(s) = me_senders
                .get_mut(&order.symbol_id)
            {
                if let Err(e) = s.send_raw(
                    RECORD_ORDER_REQUEST,
                    bytes,
                ) {
                    warn!("risk: forward order to me failed: {e}");
                }
            } else {
                warn!(
                    "order for unknown symbol_id={}",
                    order.symbol_id
                );
            }
        }

        // CMP housekeeping
        for s in me_senders.values_mut() {
            if let Err(e) = s.tick() {
                warn!("risk: me_sender tick failed: {e}");
            }
            s.recv_control();
        }
        if let Err(e) = gw_sender.tick() {
            warn!("risk: gw_sender tick failed: {e}");
        }
        gw_receiver.tick();
        me_receiver.tick();
        mark_receiver.tick();
        gw_sender.recv_control();

        // Send tips to replica if configured
        if let Some(ref mut sender) = tip_sender {
            for (symbol_id, &tip) in
                shard.tips.iter().enumerate()
            {
                // Send tip update to replica
                let tip_msg = TipSyncMessage {
                    symbol_id: symbol_id as u32,
                    tip,
                    _pad: [0; 48],
                };
                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        &tip_msg as *const TipSyncMessage
                            as *const u8,
                        std::mem::size_of::<
                            TipSyncMessage,
                        >(),
                    )
                };
                if let Err(e) = sender.send_raw(0x20, bytes) {
                    warn!("risk: tip send to replica failed: {e}");
                }
            }
            if let Err(e) = sender.tick() {
                warn!("risk: tip_sender tick failed: {e}");
            }
        }

        // Check lease health (non-blocking — lease thread updates atomically)
        if !lease_held.load(Ordering::Relaxed) {
            stop_persist_worker(&persist_shutdown, persist_handle);
            stop_lease_thread(&lease_stop, lease_thread);
            if lease_error.load(Ordering::Relaxed) {
                return Err(
                    "lease check failed after 3 consecutive errors".into()
                );
            } else {
                warn!("lease lost, demoting to replica");
                return Ok(MainTransition::Demote);
            }
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
    // Bounded wait via a watchdog thread so a stuck worker
    // can't hang the demote. The worker drains pending then
    // returns; the typical exit window is FLUSH_INTERVAL_MS
    // (10ms) + one final flush_batch. We give it 5s — well
    // past the worst-case exponential backoff between
    // failed flushes.
    let watch = std::thread::spawn(move || handle.join());
    let start = std::time::Instant::now();
    loop {
        if watch.is_finished() {
            // SAFETY: outer JoinHandle's payload is the
            // inner thread's join Result; we already
            // requested shutdown and only want to know
            // the watchdog itself terminated. Any panic
            // in the watchdog is logged via panic_handler.
            if let Err(e) = watch.join() {
                warn!(
                    "persist watchdog thread panicked: {:?}",
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
                    let _ = lease.release(&pg_client).await;
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
    let _ = handle.join();
}

#[repr(C, align(64))]
struct TipSyncMessage {
    symbol_id: u32,
    tip: u64,
    _pad: [u8; 48],
}

fn run_replica(
    shard_id: u32,
    max_symbols: usize,
) -> Result<ReplicaTransition, Box<dyn std::error::Error>> {
    let config = load_shard_config()?;
    let shard_count = config.shard_count;
    let mut shard = RiskShard::new(config);

    // SAFETY: fail-fast at startup
    let db_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL required for replica");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let mut lease = AdvisoryLease::new(shard_id);
    let client = rt.block_on(async {
        let (client, connection) =
            tokio_postgres::connect(
                &db_url,
                tokio_postgres::NoTls,
            )
            .await?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                error!("pg connection error: {e}");
            }
        });

        // Try to acquire lock (should fail, main holds it)
        match lease.try_acquire(&client).await {
            Ok(true) => {
                warn!(
                    "replica acquired lock immediately, \
                     main not running?"
                );
            }
            Ok(false) => {
                info!(
                    "replica starting, main holds lock"
                );
            }
            Err(e) => {
                return Err(Box::new(e)
                    as Box<dyn std::error::Error>);
            }
        }

        // Load baseline state from Postgres
        let state = load_from_postgres(
            &client,
            shard_id,
            shard_count,
            max_symbols,
        )
        .await?;
        shard.load_state(state);
        info!("replica loaded baseline from postgres");
        Ok::<_, Box<dyn std::error::Error>>(client)
    })?;

    // Set up CMP receiver from MEs (same as main).
    // Use first ME addr as CMP peer for the receiver.
    let me_addrs = rsx_risk::me_cmp_addrs_from_env();
    if me_addrs.is_empty() {
        return Err(
            "no ME CMP addresses configured".into()
        );
    }
    // SAFETY: me_addrs.is_empty() checked above
    let first_me_addr =
        *me_addrs.values().next().expect("INVARIANT: me_addrs non-empty (checked above)");
    let mut me_receiver = CastReceiver::new(
        // SAFETY: literal addr is always valid
        "127.0.0.1:0".parse().expect("valid addr"),
        first_me_addr,
        0,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind replica ME receiver");

    // Replica receives tip sync from main
    let replica_addr: SocketAddr =
        env::var("RSX_RISK_REPLICA_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9111".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_RISK_REPLICA_ADDR");
    let main_addr: SocketAddr =
        env::var("RSX_RISK_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9101".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_RISK_CMP_ADDR");
    let mut tip_receiver = CastReceiver::new(
        replica_addr, main_addr, 0,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind replica tip receiver");

    let lease_poll_interval_ms = env::var(
        "RSX_RISK_LEASE_POLL_MS",
    )
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(500u64);

    let mut last_poll_ms = 0u64;
    let promoted = Arc::new(AtomicBool::new(false));

    info!(
        "replica {} running, polling for promotion",
        shard_id
    );

    loop {
        let now_secs = time();
        let now_ms = now_secs * 1000;

        // Buffer fills from ME
        loop {
            let (preamble, payload) = match me_receiver
                .try_recv()
            {
                CastRecv::Data(h, p) => (h, p),
                CastRecv::Empty => break,
                CastRecv::Faulted {
                    last_delivered_seq,
                    gap_start,
                    gap_end_inclusive,
                } => panic!(
                    "FAULTED: DXS replay path not yet \
                     wired here; see rsx-matching for the \
                     POC reference impl \
                     (last_delivered={} gap=[{}..={}])",
                    last_delivered_seq,
                    gap_start,
                    gap_end_inclusive,
                ),
            };
            if preamble.record_type == RECORD_FILL
                && payload.len()
                    >= std::mem::size_of::<FillRecord>()
            {
                let fill = unsafe {
                    std::ptr::read_unaligned(
                        payload.as_ptr()
                            as *const FillRecord,
                    )
                };
                let fill_event = FillEvent {
                    seq: fill.seq,
                    symbol_id: fill.symbol_id,
                    taker_user_id: fill.taker_user_id,
                    maker_user_id: fill.maker_user_id,
                    price: fill.price.0,
                    qty: fill.qty.0,
                    taker_side: fill.taker_side,
                    timestamp_ns: fill.ts_ns,
                };
                shard.buffer_fill_for_replica(fill_event);
            }
        }

        // Receive tip sync from main
        loop {
            let (preamble, payload) = match tip_receiver
                .try_recv()
            {
                CastRecv::Data(h, p) => (h, p),
                CastRecv::Empty => break,
                CastRecv::Faulted {
                    last_delivered_seq,
                    gap_start,
                    gap_end_inclusive,
                } => panic!(
                    "FAULTED: DXS replay path not yet \
                     wired here; see rsx-matching for the \
                     POC reference impl \
                     (last_delivered={} gap=[{}..={}])",
                    last_delivered_seq,
                    gap_start,
                    gap_end_inclusive,
                ),
            };
            if preamble.record_type == 0x20
                && payload.len()
                    >= std::mem::size_of::<
                        TipSyncMessage,
                    >()
            {
                let msg = unsafe {
                    std::ptr::read_unaligned(
                        payload.as_ptr()
                            as *const TipSyncMessage,
                    )
                };
                shard.apply_tip_from_main(
                    msg.symbol_id,
                    msg.tip,
                );
            }
        }

        // Poll for advisory lock
        if now_ms - last_poll_ms
            >= lease_poll_interval_ms
        {
            last_poll_ms = now_ms;
            let acquired = rt.block_on(async {
                lease.try_acquire(&client).await
            })?;
            if acquired {
                info!(
                    "replica acquired lock, promoting"
                );
                promoted.store(true, Ordering::Release);
                break;
            }
        }

        me_receiver.tick();
        tip_receiver.tick();
    }

    // Promotion: apply buffered fills up to last tips. The
    // resulting shard state is discarded — run_main will
    // rebuild from Postgres + WAL on the next state-machine
    // tick. promote_from_replica is kept for its logging /
    // future use; the buffered-fills drain it performs is
    // belt-and-suspenders against persist-worker lag.
    info!(
        "promoting replica to main, buffered={}",
        shard.replica_buffered_count()
    );
    let fills = shard.promote_from_replica();
    info!(
        "promotion applied {} fills, returning to main \
         state-machine for re-entry as main",
        fills.len()
    );

    // Release the replica's PG session (and thus its
    // advisory lock) so run_main's blocking acquire on the
    // next tick can re-grab it cleanly.
    drop(client);

    Ok(ReplicaTransition::Promote)
}
