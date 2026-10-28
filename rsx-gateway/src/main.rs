use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastSender;
use rsx_gateway::cast_recv::CastEvent;
use rsx_gateway::config::core_id_from_env;
use rsx_gateway::config::load_gateway_config;
use rsx_gateway::config::GatewayConfig;
use rsx_gateway::handler::handle_connection;
use rsx_gateway::metrics::GatewayMetrics;
use rsx_gateway::route::expire_stale_orders;
use rsx_gateway::route::route_fill;
use rsx_gateway::route::route_liquidation;
use rsx_gateway::route::route_order_done;
use rsx_gateway::route::route_order_failed;
use rsx_gateway::route::route_order_inserted;
use rsx_gateway::state::GatewayState;
use rsx_gateway::symbols::boot_load;
use rsx_gateway::symbols::log_changes;
use rsx_gateway::symbols::spawn_sidecar;
use rsx_gateway::symbols::SymbolTable;
use rsx_health::DaemonState;
use rsx_health::LoadGauges;
use rsx_types::install_panic_handler;
use rsx_types::install_shutdown_handler;
use rsx_types::time_utils::time_ns;
use std::cell::RefCell;
use std::env;
use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tracing::error;
use tracing::info;
use tracing::warn;

const PENDING_SWEEP_INTERVAL_NS: u64 = 100_000_000;
const NS_PER_MS: u64 = 1_000_000;

/// Longest boot waits for risk to create and fill `symbol_config_schedule`.
const SCHEMA_WAIT: Duration = Duration::from_secs(30);

/// A fast restart races the prior instance's UDP socket release, so a
/// busy `RSX_GW_CAST_ADDR` is retried for 20 s; a permanent conflict (two
/// gateways on one port) still fails.
const CAST_REBIND_RETRIES: u32 = 100;
const CAST_REBIND_DELAY: Duration = Duration::from_millis(200);

/// Longest the reactor loop sleeps without a doorbell ping before it runs
/// housekeeping (heartbeats, NAK control, pending sweep, idle reap).
const HOUSEKEEPING_INTERVAL: Duration = Duration::from_millis(1);

/// Capacity of the SPSC ring between the cast-recv thread and the reactor.
/// A full ring stalls the recv thread rather than dropping a response.
const CAST_RING_CAPACITY: usize = 8192;

fn log_effective_gateway_config(config: &GatewayConfig) {
    info!(
        "gateway effective config: listen={} max_pending={} max_outbound={} order_timeout_ms={} heartbeat_interval_ms={} heartbeat_timeout_ms={} rl_user={} rl_ip={} rl_instance={} circuit_threshold={} circuit_cooldown_ms={} jwt_secret_set={} jwt_secret_len={}",
        config.listen_addr,
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
}

fn env_addr(key: &str, default: &str) -> SocketAddr {
    env::var(key)
        .unwrap_or_else(|_| default.into())
        .parse()
        .unwrap_or_else(|e| panic!("invalid {key}: {e}"))
}

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt()
        .with_ansi(std::io::stdout().is_terminal())
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    rsx_log::start_drainer(100);

    let config = load_gateway_config();
    log_effective_gateway_config(&config);

    let symbol_source = boot_load(&config.database_url, SCHEMA_WAIT)
        .unwrap_or_else(|e| panic!("failed to load symbol config from postgres: {e}"));
    log_changes(&SymbolTable::default(), &symbol_source.table);

    let gauges: Arc<LoadGauges> = LoadGauges::new();
    gauges.set_state(DaemonState::Running);
    let metrics = Arc::new(GatewayMetrics::default());
    if let Ok(addr_str) = env::var("RSX_GW_HEALTH_ADDR") {
        if let Ok(addr) = addr_str.parse::<SocketAddr>() {
            let gauges = gauges.clone();
            let metrics = metrics.clone();
            let max_pending = config.max_pending as u64;
            rsx_health::spawn_health_server(addr, move || {
                metrics.health_snapshot(&gauges, max_pending)
            });
        } else {
            warn!("RSX_GW_HEALTH_ADDR: invalid addr '{addr_str}'");
        }
    }

    let risk_addr = env_addr("RSX_RISK_CAST_ADDR", "127.0.0.1:9101");
    let gw_addr = env_addr("RSX_GW_CAST_ADDR", "127.0.0.1:9102");
    let wal_dir = env::var("RSX_GW_WAL_DIR").unwrap_or_else(|_| "./tmp/wal".into());

    let cast_sender = CastSender::new(risk_addr, 0, &PathBuf::from(&wal_dir))
        .expect("failed to create cast sender");

    let cast_receiver = {
        let mut attempt = 0;
        loop {
            match CastReceiver::new(gw_addr, risk_addr) {
                Ok(receiver) => break receiver,
                Err(e)
                    if e.kind() == std::io::ErrorKind::AddrInUse
                        && attempt < CAST_REBIND_RETRIES =>
                {
                    attempt += 1;
                    warn!(
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

    info!("gateway started on {}", config.listen_addr);

    let shutdown = install_shutdown_handler();

    let core_id = core_id_from_env("RSX_GW_CORE_ID");
    let cast_core_id = core_id_from_env("RSX_GW_CAST_CORE_ID");
    if core_id.is_some() && core_id == cast_core_id {
        warn!("RSX_GW_CORE_ID and RSX_GW_CAST_CORE_ID name the same core: the reactor and gw-cast-recv will share it");
    }

    let cast_recv_handle = rsx_gateway::cast_recv::spawn(
        cast_receiver,
        wal_dir,
        config.risk_replication_addr,
        shutdown,
        CAST_RING_CAPACITY,
        cast_core_id,
    )
    .expect("failed to spawn cast-recv thread");

    let initial_symbols = symbol_source.table.clone();
    let mut symbol_sidecar = spawn_sidecar(symbol_source, config.symbol_poll_interval)
        .expect("failed to spawn symbols thread");

    // Pin last: a thread inherits its creator's affinity, so every sidecar
    // spawned above (log drainer, health server, cast-recv, symbols) must exist
    // before the reactor thread is confined to its core.
    if let Some(core_id) = core_id {
        let setup = rsx_types::cpu::setup_hot_thread(core_id);
        info!("gateway {}", setup);
        if setup.isolated == Some(false) {
            warn!("gateway core {} not isolated — expect tail spikes", core_id);
        }
    }

    let mut rt = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .enable_timer()
        .build()
        .expect("failed to build monoio runtime");

    rt.block_on(async move {
        gauges.live.store(true, Ordering::Relaxed);
        let heartbeat_interval_ns = config.heartbeat_interval_ms * NS_PER_MS;
        let heartbeat_timeout_ns = config.heartbeat_timeout_ms * NS_PER_MS;
        let order_timeout_ns = config.order_timeout_ms * NS_PER_MS;
        let jwt_secret = config.jwt_secret;
        let listen_addr = config.listen_addr;
        let state = Rc::new(RefCell::new({
            let mut state = GatewayState::new(
                config.max_pending,
                config.circuit_threshold,
                config.circuit_cooldown_ms,
                initial_symbols,
            );
            state.rate_limit_per_user = config.rate_limit_per_user;
            state.rate_limit_per_ip = config.rate_limit_per_ip;
            state.set_instance_rate_limit(config.rate_limit_per_instance);
            state.metrics = metrics;
            state.max_outbound = config.max_outbound;
            state
        }));
        let sender = Rc::new(RefCell::new(cast_sender));

        monoio::spawn(accept_loop(
            listen_addr,
            state.clone(),
            sender.clone(),
            jwt_secret,
        ));

        let mut last_pending_sweep = time_ns();
        let mut last_heartbeat_ns = time_ns();

        let mut cast_events = cast_recv_handle.consumer;
        let doorbell_std = cast_recv_handle.doorbell;

        // A dup of the doorbell fd shares its file description, so POLLIN
        // reflects the same recv queue. The monoio wrapper is used only for
        // `readable()`; the 1-byte pings are drained on the std socket.
        let doorbell_readiness = monoio::net::udp::UdpSocket::from_std(
            doorbell_std
                .try_clone()
                .expect("dup doorbell fd for readiness poll"),
        )
        .expect("wrap doorbell fd in monoio UdpSocket");

        loop {
            if shutdown.load(Ordering::SeqCst) {
                info!("shutdown signal received, draining egress and exiting");
                if let Err(e) = sender.borrow_mut().tick() {
                    warn!("gateway: shutdown cast_sender tick failed: {e}");
                }
                cast_recv_handle
                    .join
                    .join()
                    .unwrap_or_else(|e| warn!("gateway: cast-recv thread panicked: {e:?}"));
                break;
            }
            // Drain the pings so the next `readable()` blocks.
            let mut doorbell_buf = [0u8; 64];
            while doorbell_std.recv(&mut doorbell_buf).is_ok() {}

            while let Ok(event) = cast_events.pop() {
                match event {
                    CastEvent::Fill(rec) => route_fill(&state, &rec),
                    CastEvent::OrderDone(rec) => route_order_done(&state, &rec),
                    CastEvent::OrderInserted(rec) => route_order_inserted(&state, &rec),
                    CastEvent::OrderFailed(rec) => route_order_failed(&state, &rec),
                    CastEvent::Liquidation(rec) => route_liquidation(&state, &rec),
                    CastEvent::ConfigApplied {
                        symbol_id,
                        config_version,
                    } => {
                        if state.borrow().is_config_newer(symbol_id, config_version) {
                            symbol_sidecar.reload.notify_one();
                        }
                    }
                }
            }

            while let Ok(table) = symbol_sidecar.updates.pop() {
                let mut state = state.borrow_mut();
                log_changes(&state.symbols, &table);
                state.symbols = table;
            }

            if let Err(e) = sender.borrow_mut().tick() {
                warn!("gateway: cast_sender tick failed: {e}");
            }
            sender.borrow_mut().recv_control();

            let now = time_ns();
            if now - last_pending_sweep >= PENDING_SWEEP_INTERVAL_NS {
                expire_stale_orders(&state, now.saturating_sub(order_timeout_ns));
                state.borrow_mut().client_ids.expire(now);
                last_pending_sweep = now;
            }

            if now - last_heartbeat_ns >= heartbeat_interval_ns {
                state.borrow_mut().broadcast_heartbeat(now / NS_PER_MS);
                last_heartbeat_ns = now;
            }

            state
                .borrow_mut()
                .reap_idle_connections(now.saturating_sub(heartbeat_timeout_ns));

            {
                let st = state.borrow();
                gauges
                    .pending_orders
                    .store(st.pending.in_flight() as u64, Ordering::Relaxed);
                st.metrics.resting_orders.set(st.pending.resting() as u64);
                gauges
                    .connections
                    .store(st.connections.len() as u64, Ordering::Relaxed);
                st.metrics.set_circuit_state(st.circuit.state());
            }

            monoio::select! {
                r = doorbell_readiness.readable(false) => {
                    if let Err(e) = r {
                        warn!("gateway: doorbell readable failed: {e}");
                    }
                }
                _ = monoio::time::sleep(HOUSEKEEPING_INTERVAL) => {}
            }
        }
    });
}

async fn accept_loop(
    listen_addr: String,
    state: Rc<RefCell<GatewayState>>,
    sender: Rc<RefCell<CastSender>>,
    jwt_secret: String,
) {
    let handler = move |stream, peer| {
        let state = state.clone();
        let sender = sender.clone();
        let jwt_secret = jwt_secret.clone();
        monoio::spawn(async move {
            handle_connection(stream, peer, state, sender, &jwt_secret).await;
        });
    };
    if let Err(e) = rsx_gateway::ws::ws_accept_loop(&listen_addr, handler).await {
        error!("ws accept error: {e}");
    }
}
