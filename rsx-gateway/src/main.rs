use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastSender;
use rsx_gateway::cast_recv::CastEvent;
use rsx_gateway::config::load_gateway_config;
use rsx_gateway::handler::handle_connection;
use rsx_gateway::metrics::GatewayMetrics;
use rsx_gateway::route::expire_stale_orders;
use rsx_gateway::route::route_fill;
use rsx_gateway::route::route_liquidation;
use rsx_gateway::route::route_order_cancelled;
use rsx_gateway::route::route_order_done;
use rsx_gateway::route::route_order_failed;
use rsx_gateway::route::route_order_inserted;
use rsx_gateway::state::GatewayState;
use rsx_health::LoadGauges;
use rsx_types::install_panic_handler;
use rsx_types::install_shutdown_handler;
use rsx_types::time_utils::time_ns;
use std::cell::RefCell;
use std::env;
use std::io::IsTerminal;
use std::net::SocketAddr;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tracing::info;
use tracing::warn;

const PENDING_SWEEP_INTERVAL_US: u64 = 100_000;
const NS_PER_MS: u64 = 1_000_000;

/// How long the gateway keeps retrying the cast-receiver bind
/// when RSX_GW_CAST_ADDR is transiently held (AddrInUse). A fast
/// supervisor restart can race the prior instance's UDP socket
/// release; retrying rides that out instead of crashing. The
/// budget is generous enough to self-heal a slow handover (was
/// 6s — too tight, so an overlapping restart hard-crashed here,
/// got respawned, re-raced the port, and stormed) yet still
/// bounded so a genuinely permanent conflict (two gateways
/// configured on one port) fails fast rather than hanging.
const CAST_REBIND_RETRIES: u32 = 100; // 100 * 200ms = 20s
const CAST_REBIND_DELAY: std::time::Duration = std::time::Duration::from_millis(200);

/// Idle wakeup cadence for the casting loop. When no doorbell
/// ping or housekeeping tick is due, the loop still wakes this
/// often to run housekeeping (heartbeat broadcast, sender NAK
/// control, pending sweep, stale-connection reap). A ring push on
/// the dedicated cast-recv thread (see `cast_recv.rs`) wakes the
/// loop immediately via its doorbell, independent of this.
const HOUSEKEEPING_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1);

/// Capacity of the SPSC ring between the dedicated cast-recv
/// thread and this monoio thread. Sized generously (matches
/// rsx-risk's egress ring convention) — a full ring means the
/// monoio thread has fallen behind by this many risk responses,
/// which the recv thread's push-retry spins through rather than drop.
const CAST_RING_CAPACITY: usize = 8192;

fn log_effective_gateway_config(config: &rsx_gateway::config::GatewayConfig) {
    info!(
        "gateway effective config: listen={} risk_addr={} max_pending={} max_outbound={} order_timeout_ms={} heartbeat_interval_ms={} heartbeat_timeout_ms={} rl_user={} rl_ip={} rl_instance={} circuit_threshold={} circuit_cooldown_ms={} jwt_secret_set={} jwt_secret_len={}",
        config.listen_addr,
        config.risk_addr,
        config.max_pending,
        config.max_outbound,
        config.order_timeout_ms,
        config.heartbeat_interval_ms,
        config.heartbeat_timeout_ms,
        config.rate_limit_per_user,
        config.rate_limit_per_ip,
        config.rate_limit_per_instance,
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

    // ANSI only when stdout is a TTY; file logs stay clean
    // and greppable (structured latency lines).
    tracing_subscriber::fmt()
        .with_ansi(std::io::stdout().is_terminal())
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Drain hot-path latency samples out-of-band
    // (see rsx-types/src/latency.rs). 100 ms is a
    // good compromise between dashboard freshness
    // and drain-thread CPU.
    rsx_log::start_drainer(100);

    let config = load_gateway_config();
    log_effective_gateway_config(&config);

    // Health server: RSX_GW_HEALTH_ADDR=127.0.0.1:9200
    // GET /health → 200/503 (liveness)
    // GET /ready   → 200/503 (readiness; 503 when pending ≥ 90%)
    // GET /metrics → JSON load snapshot (HPA / ops)
    let gauges: Arc<LoadGauges> = LoadGauges::new();
    gauges.state_idx.store(4, Ordering::Relaxed); // "running"
    let metrics = Arc::new(GatewayMetrics::default());
    if let Ok(addr_str) = env::var("RSX_GW_HEALTH_ADDR") {
        if let Ok(addr) = addr_str.parse::<SocketAddr>() {
            let g = gauges.clone();
            let m = metrics.clone();
            let max_p = config.max_pending as u64;
            rsx_health::spawn_health_server(addr, move || m.health_snapshot(&g, max_p));
        } else {
            warn!("RSX_GW_HEALTH_ADDR: invalid addr '{addr_str}'");
        }
    }

    let risk_addr: SocketAddr = env::var("RSX_RISK_CAST_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9101".into())
        .parse()
        // SAFETY: fail-fast at startup
        .expect("invalid RSX_RISK_CAST_ADDR");
    let gw_addr: SocketAddr = env::var("RSX_GW_CAST_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9102".into())
        .parse()
        // SAFETY: fail-fast at startup
        .expect("invalid RSX_GW_CAST_ADDR");
    let wal_dir = env::var("RSX_GW_WAL_DIR").unwrap_or_else(|_| "./tmp/wal".into());

    // casting/UDP: send orders to Risk
    let cast_sender = CastSender::new(risk_addr, 0, &PathBuf::from(&wal_dir))
        // SAFETY: fail-fast at startup
        .expect("failed to create cast sender");

    // casting/UDP: receive responses from Risk. Retry on AddrInUse:
    // a fast restart (fault-injection / supervisor) races the old
    // process's socket release; panicking here would leave the
    // gateway dead instead of recovering.
    let cast_receiver = {
        let mut attempt = 0;
        loop {
            match CastReceiver::new(gw_addr, risk_addr) {
                Ok(r) => break r,
                Err(e)
                    if e.kind() == std::io::ErrorKind::AddrInUse
                        && attempt < CAST_REBIND_RETRIES =>
                {
                    attempt += 1;
                    tracing::warn!(
                        "gateway cast bind {gw_addr} in use, \
                         retry {attempt}/{CAST_REBIND_RETRIES} \
                         (prior instance still holds the socket?)"
                    );
                    std::thread::sleep(CAST_REBIND_DELAY);
                }
                Err(e) => panic!(
                    "failed to bind cast receiver on {gw_addr}: {e} \
                     (still held after {attempt} retries — another \
                     gateway on this RSX_GW_CAST_ADDR?)"
                ),
            }
        }
    };

    info!("gateway started on {}", config.listen_addr,);

    // SIGTERM/SIGINT flip this flag; both the cast-recv thread's
    // busy-spin loop and the monoio reactor loop below poll it
    // (defined here, before the recv thread, so it can share the
    // same `&'static AtomicBool`).
    let shutdown = install_shutdown_handler();

    // A std non-blocking UDP thread pushes decoded responses onto the
    // SPSC ring drained by this monoio thread.
    let cast_recv_handle =
        rsx_gateway::cast_recv::spawn(cast_receiver, wal_dir.clone(), shutdown, CAST_RING_CAPACITY)
            // SAFETY: fail-fast at startup
            .expect("failed to spawn cast-recv thread");

    let max_pending = config.max_pending;
    let order_timeout_ms = config.order_timeout_ms;
    let heartbeat_interval_ns = config.heartbeat_interval_ms * NS_PER_MS;
    let heartbeat_timeout_ns = config.heartbeat_timeout_ms * NS_PER_MS;
    let max_outbound = config.max_outbound;
    let circuit_threshold = config.circuit_threshold;
    let circuit_cooldown_ms = config.circuit_cooldown_ms;
    let jwt_secret = config.jwt_secret.clone();

    if let Ok(core_str) = env::var("RSX_GW_CORE_ID") {
        if let Ok(core_id) = core_str.parse::<usize>() {
            let setup = rsx_types::cpu::setup_hot_thread(core_id);
            tracing::info!("gateway {}", setup);
            if setup.isolated == Some(false) {
                tracing::warn!("gateway core {} not isolated — expect tail spikes", core_id);
            }
        }
    }

    let mut rt = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .enable_timer()
        .build()
        // SAFETY: fail-fast at startup
        .expect("failed to build monoio runtime");

    // The reactor loop below polls the same `shutdown` flag (it
    // wakes at least every HOUSEKEEPING_INTERVAL) and exits. The
    // gateway holds no durable buffer — pending orders live in RAM
    // and are re-driven by risk re-emitting after its own replay —
    // so a final cast_sender tick to flush in-flight egress, plus
    // joining the cast-recv thread, is the whole drain; exiting the
    // loop drops the runtime, which stops the WS accept task.
    let listen_addr = config.listen_addr.clone();
    let rl_user = config.rate_limit_per_user;
    let rl_ip = config.rate_limit_per_ip;
    let rl_instance = config.rate_limit_per_instance;
    let gauges_inner = gauges.clone();
    rt.block_on(async move {
        // Signal that the gateway is live once the monoio
        // runtime is up — the health server thread may start
        // reading gauges immediately after spawn_health_server.
        gauges_inner.live.store(true, Ordering::Relaxed);
        let state = Rc::new(RefCell::new({
            let mut s = GatewayState::new(
                max_pending,
                circuit_threshold,
                circuit_cooldown_ms,
                config.symbol_configs,
            );
            s.rate_limit_per_user = rl_user;
            s.rate_limit_per_ip = rl_ip;
            s.set_instance_rate_limit(rl_instance);
            s.metrics = metrics.clone();
            s.max_outbound = max_outbound;
            s
        }));
        let sender = Rc::new(RefCell::new(cast_sender));

        // Spawn WS accept loop
        let ws_addr = listen_addr;
        let state_accept = state.clone();
        let sender_accept = sender.clone();
        let jwt_secret_accept = jwt_secret.clone();
        monoio::spawn(async move {
            if let Err(e) = rsx_gateway::ws::ws_accept_loop(&ws_addr, move |stream, peer| {
                let st = state_accept.clone();
                let snd = sender_accept.clone();
                let secret = jwt_secret_accept.clone();
                monoio::spawn(async move {
                    handle_connection(stream, peer, st, snd, &secret).await;
                });
            })
            .await
            {
                tracing::error!("ws accept error: {e}");
            }
        });

        let mut last_pending_sweep = time_ns();
        let mut last_heartbeat_ns = time_ns();

        let mut cast_events = cast_recv_handle.consumer;
        let doorbell_std = cast_recv_handle.doorbell;

        // Duplicate the doorbell's UDP fd for io_uring readiness. Dup
        // shares the file description, so POLLIN reflects the same
        // recv queue. Wrap the dup in a monoio UdpSocket used
        // *only* for `readable()` — POLL_ADD wakeup; draining the
        // doorbell's own 1-byte pings stays on the std socket below.
        let doorbell_readiness = {
            let dup = unsafe { std::os::fd::BorrowedFd::borrow_raw(doorbell_std.as_raw_fd()) }
                .try_clone_to_owned()
                // SAFETY: fail-fast at startup
                .expect("dup doorbell fd for readiness poll");
            monoio::net::udp::UdpSocket::from_std(dup.into())
                // SAFETY: fail-fast at startup
                .expect("wrap doorbell fd in monoio UdpSocket")
        };
        doorbell_std
            .set_nonblocking(true)
            // SAFETY: fail-fast at startup
            .expect("doorbell socket must be non-blocking");

        // Main loop. Drains every ring event the cast-recv thread
        // has pushed, runs periodic housekeeping, then parks on the
        // reactor until the next doorbell ping lands (or a 1 ms
        // housekeeping timer fires — heartbeats, NAK control, and
        // connection reaping still run while the ring is idle).
        // This is NOT a spin: the ring drain below is a cheap,
        // non-blocking `pop` (no syscall), and the risk->gateway
        // recv itself already happened on the dedicated cast-recv
        // thread (see `cast_recv.rs`) — this loop only routes into
        // `GatewayState`, which stays `Rc<RefCell>`/single-thread.
        loop {
            if shutdown.load(Ordering::SeqCst) {
                info!("shutdown signal received, draining egress and exiting");
                if let Err(e) = sender.borrow_mut().tick() {
                    tracing::warn!("gateway: shutdown cast_sender tick failed: {e}");
                }
                cast_recv_handle.join.join().unwrap_or_else(|e| {
                    tracing::warn!("gateway: cast-recv thread panicked: {e:?}")
                });
                break;
            }
            // Drain the doorbell's pending pings so the next
            // `readable()` wait actually blocks instead of firing
            // immediately on already-read bytes.
            let mut doorbell_buf = [0u8; 64];
            while doorbell_std.recv(&mut doorbell_buf).is_ok() {}

            while let Ok(event) = cast_events.pop() {
                match event {
                    CastEvent::Fill(rec) => route_fill(&state, &rec),
                    CastEvent::OrderDone(rec) => route_order_done(&state, &rec),
                    CastEvent::OrderCancelled(rec) => route_order_cancelled(&state, &rec),
                    CastEvent::OrderInserted(rec) => route_order_inserted(&state, &rec),
                    CastEvent::OrderFailed(rec) => route_order_failed(&state, &rec),
                    CastEvent::Liquidation(rec) => route_liquidation(&state, &rec),
                    CastEvent::ConfigApplied {
                        symbol_id,
                        config_version,
                    } => {
                        let applied = state
                            .borrow_mut()
                            .apply_config_applied(symbol_id, config_version);
                        if !applied {
                            tracing::warn!(
                                "ignored CONFIG_APPLIED symbol={} version={}",
                                symbol_id,
                                config_version
                            );
                        }
                    }
                }
            }

            if let Err(e) = sender.borrow_mut().tick() {
                tracing::warn!("gateway: cast_sender tick failed: {e}");
            }
            sender.borrow_mut().recv_control();

            let now = time_ns();
            if now - last_pending_sweep >= PENDING_SWEEP_INTERVAL_US * 1000 {
                let cutoff = now.saturating_sub(order_timeout_ms * NS_PER_MS);
                expire_stale_orders(&state, cutoff);
                last_pending_sweep = now;
            }

            if now - last_heartbeat_ns >= heartbeat_interval_ns {
                state.borrow_mut().broadcast_heartbeat(now / NS_PER_MS);
                last_heartbeat_ns = now;
            }

            let idle_cutoff = now.saturating_sub(heartbeat_timeout_ns);
            state.borrow_mut().reap_idle_connections(idle_cutoff);

            // Publish load gauges (relaxed stores; off hot path —
            // runs once per monoio yield, not per message).
            {
                let st = state.borrow();
                gauges_inner
                    .pending_orders
                    .store(st.pending.len() as u64, Ordering::Relaxed);
                gauges_inner
                    .connections
                    .store(st.connections.len() as u64, Ordering::Relaxed);
                st.metrics.set_circuit_state(st.circuit.state());
            }

            // Park on the reactor: wake instantly on a doorbell
            // ping from the cast-recv thread (io_uring POLL_ADD via
            // `readable`), or after the housekeeping interval,
            // whichever comes first. relaxed=false so the
            // readiness is authoritative for our own recv.
            monoio::select! {
                r = doorbell_readiness.readable(false) => {
                    if let Err(e) = r {
                        tracing::warn!(
                            "gateway: doorbell readable failed: {e}"
                        );
                    }
                }
                _ = monoio::time::sleep(
                    HOUSEKEEPING_INTERVAL,
                ) => {}
            }
        }
    });
}
