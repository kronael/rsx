use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::cmp::CmpSender;
use rsx_dxs::records::ConfigAppliedRecord;
use rsx_dxs::records::FillRecord;
use rsx_dxs::records::LiquidationRecord;
use rsx_dxs::records::OrderCancelledRecord;
use rsx_dxs::records::OrderDoneRecord;
use rsx_dxs::records::OrderFailedRecord;
use rsx_dxs::records::OrderInsertedRecord;
use rsx_dxs::records::RECORD_CONFIG_APPLIED;
use rsx_dxs::records::RECORD_FILL;
use rsx_dxs::records::RECORD_ORDER_CANCELLED;
use rsx_dxs::records::RECORD_ORDER_DONE;
use rsx_dxs::records::RECORD_LIQUIDATION;
use rsx_dxs::records::RECORD_ORDER_FAILED;
use rsx_dxs::records::RECORD_ORDER_INSERTED;
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
use rsx_types::time::time_ns;
use std::cell::RefCell;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
use tracing::info;

const PENDING_SWEEP_INTERVAL_US: u64 = 100_000;
const NS_PER_MS: u64 = 1_000_000;

fn log_effective_gateway_config(
    config: &rsx_gateway::config::GatewayConfig,
) {
    info!(
        "gateway effective config: listen={} risk_addr={} max_pending={} order_timeout_ms={} heartbeat_interval_ms={} heartbeat_timeout_ms={} rl_user={} rl_ip={} rl_instance={} circuit_threshold={} circuit_cooldown_ms={} jwt_secret_set={} jwt_secret_len={}",
        config.listen_addr,
        config.risk_addr,
        config.max_pending,
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

    tracing_subscriber::fmt::init();

    let config = load_gateway_config();
    log_effective_gateway_config(&config);

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
    let wal_dir = env::var("RSX_GW_WAL_DIR")
        .unwrap_or_else(|_| "./tmp/wal".into());

    // CMP/UDP: send orders to Risk
    let cmp_sender = CmpSender::new(
        risk_addr,
        0,
        &PathBuf::from(&wal_dir),
    )
    // SAFETY: fail-fast at startup
    .expect("failed to create CMP sender");

    // CMP/UDP: receive responses from Risk
    let mut cmp_receiver = CmpReceiver::new(
        gw_addr, risk_addr, 0,
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

    let mut rt =
        monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
            .build()
            // SAFETY: fail-fast at startup
            .expect("failed to build monoio runtime");

    let listen_addr = config.listen_addr.clone();
    rt.block_on(async move {
        let state = Rc::new(RefCell::new(
            GatewayState::new(
                max_pending,
                circuit_threshold,
                circuit_cooldown_ms,
                config.symbol_configs,
            ),
        ));
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
            while let Some((hdr, payload)) =
                cmp_receiver.try_recv()
            {
                match hdr.record_type {
                    RECORD_FILL
                        if payload.len()
                            >= std::mem::size_of::<
                                FillRecord,
                            >() =>
                    {
                        let rec = unsafe {
                            std::ptr::read_unaligned(
                                payload.as_ptr()
                                    as *const
                                        FillRecord,
                            )
                        };
                        route_fill(&state, &rec);
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
                        route_order_done(
                            &state, &rec,
                        );
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
                        route_order_cancelled(
                            &state, &rec,
                        );
                    }
                    RECORD_ORDER_INSERTED
                        if payload.len()
                            >= std::mem::size_of::<
                                OrderInsertedRecord,
                            >() =>
                    {
                        let rec = unsafe {
                            std::ptr::read_unaligned(
                                payload.as_ptr()
                                    as *const
                                        OrderInsertedRecord,
                            )
                        };
                        route_order_inserted(
                            &state, &rec,
                        );
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
                        route_order_failed(
                            &state, &rec,
                        );
                    }
                    RECORD_LIQUIDATION
                        if payload.len()
                            >= std::mem::size_of::<
                                LiquidationRecord,
                            >() =>
                    {
                        let rec = unsafe {
                            std::ptr::read_unaligned(
                                payload.as_ptr()
                                    as *const
                                        LiquidationRecord,
                            )
                        };
                        route_liquidation(
                            &state, &rec,
                        );
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

            let _ = sender.borrow_mut().tick();
            cmp_receiver.tick();
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

            // Yield to monoio scheduler
            monoio::time::sleep(
                std::time::Duration::from_micros(100),
            )
            .await;
        }
    });
}
