use rsx_cast::cast::CastRecv;
use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastSender;
use rsx_cast::decode_payload;
use rsx_messages::ConfigAppliedRecord;
use rsx_messages::FillRecord;
use rsx_messages::LiquidationRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_CONFIG_APPLIED;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_LIQUIDATION;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_gateway::config::load_gateway_config;
use rsx_gateway::handler::handle_connection;
use rsx_gateway::route::route_fill;
use rsx_gateway::route::route_liquidation;
use rsx_gateway::route::route_order_cancelled;
use rsx_gateway::route::route_order_done;
use rsx_gateway::route::route_order_failed;
use rsx_gateway::route::route_order_inserted;
use rsx_gateway::state::GatewayState;
use rsx_types::install_panic_handler;
use rsx_types::time_utils::time_ns;
use std::cell::RefCell;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
use tracing::info;
use tracing::warn;

const PENDING_SWEEP_INTERVAL_US: u64 = 100_000;
const NS_PER_MS: u64 = 1_000_000;

/// Drain the risk producer's replication stream after the
/// gateway's CMP receiver hit a sticky FAULTED or RECONNECT.
/// Mirrors `rsx_risk::main::handle_replay`. Round-1 apply
/// just logs records; the gateway's outbound state (per-user
/// pending map, position cache) recovers indirectly via
/// risk re-emitting after its own replay completes.
fn handle_replay(
    last_delivered_seq: u64,
    gap: Option<(u64, u64)>,
    wal_dir: &str,
) -> u64 {
    match gap {
        Some((gs, ge)) => warn!(
            "gateway cmp_receiver FAULTED at \
             seq={last_delivered_seq} gap=[{gs}..={ge}], \
             opening replay via RSX_RISK_REPLICATION_ADDR",
        ),
        None => warn!(
            "gateway cmp_receiver RECONNECT at \
             seq={last_delivered_seq}, opening replay via \
             RSX_RISK_REPLICATION_ADDR",
        ),
    }
    let replay_addr = match env::var("RSX_RISK_REPLICATION_ADDR") {
        Ok(a) => a,
        Err(_) => {
            let skip_to = gap.map(|(_, ge)| ge).unwrap_or(last_delivered_seq);
            warn!(
                "RSX_RISK_REPLICATION_ADDR not set; \
                 skipping gap to seq={skip_to} (in-flight fills lost)"
            );
            return skip_to;
        }
    };
    // Gateway sees a merged stream from risk (response side);
    // stream_id 0 matches `CastSender::new(.., 0, ..)` on the
    // risk gw_sender.
    let tip_file = PathBuf::from(wal_dir)
        .join("gateway_replay_tip.bin");
    let new_tip = rsx_gateway::drain_replay(
        0,
        replay_addr.clone(),
        last_delivered_seq,
        tip_file,
        |raw| {
            let seq = rsx_cast::wal::extract_seq(&raw.payload)
                .unwrap_or(0);
            tracing::debug!(
                "gateway replay applied record_type={} seq={}",
                raw.header.record_type, seq,
            );
        },
    )
    .unwrap_or_else(|e| {
        panic!(
            "gateway replay drain failed against \
             {replay_addr}: {e}",
        )
    });
    info!(
        "gateway replay drained, new_tip={new_tip}, resuming",
    );
    new_tip
}

fn log_effective_gateway_config(
    config: &rsx_gateway::config::GatewayConfig,
) {
    info!(
        "gateway effective config: listen={} risk_addr={} max_pending={} order_timeout_ms={} heartbeat_interval_ms={} heartbeat_timeout_ms={} rl_user={} rl_ip={} circuit_threshold={} circuit_cooldown_ms={} jwt_secret_set={} jwt_secret_len={}",
        config.listen_addr,
        config.risk_addr,
        config.max_pending,
        config.order_timeout_ms,
        config.heartbeat_interval_ms,
        config.heartbeat_timeout_ms,
        config.rate_limit_per_user,
        config.rate_limit_per_ip,
        config.circuit_threshold,
        config.circuit_cooldown_ms,
        !config.jwt_secret.is_empty(),
        config.jwt_secret.len(),
    );
    for cfg in &config.symbol_configs {
        info!(
            "gateway symbol_config sid={} tick_size={} lot_size={} price_decimals={} qty_decimals={}",
            cfg.symbol_id,
            cfg.tick_size,
            cfg.lot_size,
            cfg.price_decimals,
            cfg.qty_decimals,
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

    let config = load_gateway_config();
    log_effective_gateway_config(&config);

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
    let wal_dir = env::var("RSX_GW_WAL_DIR")
        .unwrap_or_else(|_| "./tmp/wal".into());

    // CMP/UDP: send orders to Risk
    let cmp_sender = CastSender::new(
        risk_addr,
        0,
        &PathBuf::from(&wal_dir),
    )
    // SAFETY: fail-fast at startup
    .expect("failed to create CMP sender");

    // CMP/UDP: receive responses from Risk
    let mut cmp_receiver = CastReceiver::new(
        gw_addr, risk_addr,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind CMP receiver");

    info!(
        "gateway started on {}",
        config.listen_addr,
    );

    let max_pending = config.max_pending;
    let order_timeout_ms = config.order_timeout_ms;
    let heartbeat_interval_ns =
        config.heartbeat_interval_ms * NS_PER_MS;
    let heartbeat_timeout_ns =
        config.heartbeat_timeout_ms * NS_PER_MS;
    let circuit_threshold = config.circuit_threshold;
    let circuit_cooldown_ms = config.circuit_cooldown_ms;
    let jwt_secret = config.jwt_secret.clone();
    let hb_interval = config.heartbeat_interval_ms;
    let hb_timeout = config.heartbeat_timeout_ms;

    if let Ok(core_str) = env::var("RSX_GW_CORE_ID") {
        if let Ok(core_id) = core_str.parse::<usize>() {
            let ids = core_affinity::get_core_ids()
                .unwrap_or_default();
            if let Some(id) = ids.get(core_id) {
                core_affinity::set_for_current(*id);
                tracing::info!("gateway pinned to core {}", core_id);
            }
        }
    }

    let mut rt =
        monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
            .enable_timer()
            .build()
            // SAFETY: fail-fast at startup
            .expect("failed to build monoio runtime");

    let listen_addr = config.listen_addr.clone();
    let rl_user = config.rate_limit_per_user;
    let rl_ip = config.rate_limit_per_ip;
    rt.block_on(async move {
        let state = Rc::new(RefCell::new({
            let mut s = GatewayState::new(
                max_pending,
                circuit_threshold,
                circuit_cooldown_ms,
                config.symbol_configs,
            );
            s.rate_limit_per_user = rl_user;
            s.rate_limit_per_ip = rl_ip;
            s
        }));
        let sender =
            Rc::new(RefCell::new(cmp_sender));

        // Spawn WS accept loop
        let ws_addr = listen_addr;
        let state_accept = state.clone();
        let sender_accept = sender.clone();
        let jwt_secret_accept = jwt_secret.clone();
        monoio::spawn(async move {
            if let Err(e) =
                rsx_gateway::ws::ws_accept_loop(
                    &ws_addr,
                    move |stream, peer| {
                        let st = state_accept.clone();
                        let snd =
                            sender_accept.clone();
                        let secret =
                            jwt_secret_accept.clone();
                        monoio::spawn(async move {
                            handle_connection(
                                stream, peer, st, snd,
                                &secret,
                                hb_interval,
                                hb_timeout,
                            )
                            .await;
                        });
                    },
                )
                .await
            {
                tracing::error!(
                    "ws accept error: {e}"
                );
            }
        });

        let mut last_pending_sweep = time_ns();
        let mut last_heartbeat_ns = time_ns();

        // CMP polling loop (yields to monoio)
        loop {
            loop {
                let (hdr, payload) = match cmp_receiver
                    .try_recv()
                {
                    CastRecv::Data(h, p) => (h, p),
                    CastRecv::Empty => break,
                    CastRecv::Faulted {
                        last_delivered_seq,
                        gap_start,
                        gap_end_inclusive,
                    } => {
                        let new_tip = handle_replay(
                            last_delivered_seq,
                            Some((gap_start, gap_end_inclusive)),
                            &wal_dir,
                        );
                        cmp_receiver.reset_after_replay(new_tip);
                        continue;
                    }
                    CastRecv::Reconnect { last_delivered_seq } => {
                        let new_tip = handle_replay(
                            last_delivered_seq,
                            None,
                            &wal_dir,
                        );
                        cmp_receiver.reset_after_replay(new_tip);
                        continue;
                    }
                };
                {
                match hdr.record_type {
                    RECORD_FILL => if let Some(rec) = decode_payload::<FillRecord>(&payload) {
                        // Sub-stage: fill record arrived at
                        // gateway's CMP recv loop, about to
                        // route. Anchor on taker_ts_ns (with
                        // the >2024 plausibility guard).
                        {
                            let now_ns = std::time::SystemTime::now()
                                .duration_since(
                                    std::time::UNIX_EPOCH,
                                )
                                .map(|d| d.as_nanos() as u64)
                                .unwrap_or(0);
                            let anchor_ns = if rec.taker_ts_ns
                                > 1_700_000_000_000_000_000
                            {
                                rec.taker_ts_ns
                            } else {
                                rec.ts_ns
                            };
                            let t_us = now_ns
                                .saturating_sub(anchor_ns)
                                / 1000;
                            rsx_log::latency::sample("gateway_cmp_recv", rec.taker_order_id_hi, rec.taker_order_id_lo, t_us, anchor_ns);
                        }
                        route_fill(&state, &rec);
                    }
                    RECORD_ORDER_DONE => if let Some(rec) = decode_payload::<OrderDoneRecord>(&payload) {
                        route_order_done(
                            &state, &rec,
                        );
                    }
                    RECORD_ORDER_CANCELLED => if let Some(rec) = decode_payload::<OrderCancelledRecord>(&payload) {
                        route_order_cancelled(
                            &state, &rec,
                        );
                    }
                    RECORD_ORDER_INSERTED => if let Some(rec) = decode_payload::<OrderInsertedRecord>(&payload) {
                        route_order_inserted(
                            &state, &rec,
                        );
                    }
                    RECORD_ORDER_FAILED => if let Some(rec) = decode_payload::<OrderFailedRecord>(&payload) {
                        route_order_failed(
                            &state, &rec,
                        );
                    }
                    RECORD_LIQUIDATION => if let Some(rec) = decode_payload::<LiquidationRecord>(&payload) {
                        route_liquidation(
                            &state, &rec,
                        );
                    }
                    RECORD_CONFIG_APPLIED => if let Some(rec) = decode_payload::<ConfigAppliedRecord>(&payload) {
                        let applied = state
                            .borrow_mut()
                            .apply_config_applied(
                                rec.symbol_id,
                                rec.config_version,
                            );
                        if !applied {
                            tracing::warn!(
                                "ignored CONFIG_APPLIED symbol={} version={}",
                                rec.symbol_id,
                                rec.config_version
                            );
                        }
                    }
                    _ => {}
                }
            }

            if let Err(e) = sender.borrow_mut().tick() {
                tracing::warn!("gateway: cmp_sender tick failed: {e}");
            }
            sender.borrow_mut().recv_control();

            let now = time_ns();
            if now - last_pending_sweep
                >= PENDING_SWEEP_INTERVAL_US * 1000
            {
                let cutoff = now.saturating_sub(
                    order_timeout_ms * NS_PER_MS,
                );
                let _ =
                    state.borrow_mut().pending.remove_stale(cutoff);
                last_pending_sweep = now;
            }

            // Server heartbeat broadcast
            if now - last_heartbeat_ns
                >= heartbeat_interval_ns
            {
                let ts_ms = now / NS_PER_MS;
                state
                    .borrow_mut()
                    .broadcast_heartbeat(ts_ms);
                last_heartbeat_ns = now;
            }

            // Reap stale connections
            {
                let cutoff = now.saturating_sub(
                    heartbeat_timeout_ns,
                );
                let stale: Vec<u64> = state
                    .borrow()
                    .stale_connections(cutoff);
                if !stale.is_empty() {
                    let mut st =
                        state.borrow_mut();
                    for id in &stale {
                        if let Some(c) =
                            st.connections.get(id)
                        {
                            info!(
                                "closing idle connection user_id={}",
                                c.user_id
                            );
                        }
                        st.remove_connection(*id);
                    }
                }
                }
            }

            monoio::time::sleep(std::time::Duration::ZERO).await;
        }
    });
}
