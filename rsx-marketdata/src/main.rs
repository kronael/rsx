use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecvWith;
use rsx_cast::wal::extract_seq;
use rsx_health::CounterGauge;
use rsx_health::HealthSnapshot;
use rsx_health::LoadGauges;
use rsx_health::QueueGauge;
use rsx_marketdata::apply::apply_record;
use rsx_marketdata::config::load_marketdata_config;
use rsx_marketdata::handler::handle_connection;
use rsx_marketdata::replay::run_replay_bootstrap_blocking;
use rsx_marketdata::state::MarketDataState;
use rsx_types::install_panic_handler;
use rsx_types::install_shutdown_handler;
use rsx_types::time_utils::time_ms;
use rsx_types::time_utils::time_ns;
use rsx_types::SymbolConfig;
use std::cell::RefCell;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;
use tracing::warn;

fn handle_replay(
    state: &Rc<RefCell<MarketDataState>>,
    max_outbound: usize,
    stream_id: u32,
    last_delivered_seq: u64,
    gap: Option<(u64, u64)>,
    replay_addr: Option<&str>,
    tip_file_path: &str,
) -> u64 {
    match gap {
        Some((gs, ge)) => warn!(
            "marketdata cast_receiver FAULTED at \
             seq={last_delivered_seq} gap=[{gs}..={ge}], \
             opening replay via RSX_MD_REPLAY_ADDR",
        ),
        None => warn!(
            "marketdata cast_receiver RECONNECT at \
             seq={last_delivered_seq}, opening replay via \
             RSX_MD_REPLAY_ADDR",
        ),
    }
    let addr = replay_addr.unwrap_or_else(|| {
        panic!(
            "marketdata {} requires RSX_MD_REPLAY_ADDR \
             pointing at ME's replication server",
            if gap.is_some() {
                "FAULTED"
            } else {
                "RECONNECT"
            },
        )
    });
    let tip_file = PathBuf::from(format!("{tip_file_path}.replay_{stream_id}",));
    let gap_end = gap.map(|(_, ge)| ge).unwrap_or(0);
    const MAX_TIP_RETRIES: u8 = 5;
    let mut tip_retries = 0u8;
    let tls = rsx_cast::TlsConfig::from_env()
        .unwrap_or_else(|e| panic!("marketdata replay requires TLS: {e}"));
    let new_tip = loop {
        let tip = rsx_marketdata::replay::drain_replay(
            stream_id,
            addr.to_string(),
            last_delivered_seq,
            tip_file.clone(),
            tls.clone(),
            |raw| apply_record(state, raw.header.record_type, &raw.payload, max_outbound),
        )
        .unwrap_or_else(|e| {
            panic!(
                "marketdata replay drain failed against \
                 {addr}: {e}",
            )
        });
        tip_retries += 1;
        if tip < gap_end && tip_retries < MAX_TIP_RETRIES {
            warn!(
                "marketdata replay tip={tip} < gap_end={gap_end}, \
                 WAL not flushed (attempt {tip_retries}), retry 15ms"
            );
            std::thread::sleep(Duration::from_millis(15));
        } else {
            break tip;
        }
    };
    info!(
        "marketdata replay drained, new_tip={new_tip}, \
         resuming",
    );
    new_tip
}

fn log_effective_marketdata_config(config: &rsx_marketdata::config::MarketDataConfig) {
    info!(
        "marketdata effective config: listen={} max_symbols={} snapshot_depth={} book_capacity={} mid_price={} tick_size={} lot_size={} price_decimals={} qty_decimals={} max_outbound={} replay_addr={} stream_id={} tip_file={} heartbeat_interval_ms={} heartbeat_timeout_ms={}",
        config.listen_addr,
        config.max_symbols,
        config.snapshot_depth,
        config.book_capacity,
        config.mid_price,
        config.tick_size,
        config.lot_size,
        config.price_decimals,
        config.qty_decimals,
        config.max_outbound,
        config.replay_addr.as_deref().unwrap_or(""),
        config.stream_id,
        config.tip_file,
        config.heartbeat_interval_ms,
        config.heartbeat_timeout_ms,
    );
}

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt::init();

    let config = load_marketdata_config();
    log_effective_marketdata_config(&config);

    let base_config = SymbolConfig {
        symbol_id: 0,
        price_decimals: config.price_decimals,
        qty_decimals: config.qty_decimals,
        tick_size: config.tick_size,
        lot_size: config.lot_size,
    };

    let state = Rc::new(RefCell::new(MarketDataState::new(
        config.max_symbols,
        base_config,
        config.book_capacity,
        config.mid_price,
    )));

    if let Some(ref replay_addr) = config.replay_addr {
        info!("starting replay bootstrap from {}", replay_addr);
        let tip_file = PathBuf::from(&config.tip_file);
        let tls = rsx_cast::TlsConfig::from_env().expect(
            "marketdata replay requires TLS \
             (run scripts/gen-snakeoil-certs.sh)",
        );
        match run_replay_bootstrap_blocking(config.stream_id, replay_addr.clone(), tip_file, tls) {
            Ok(result) => {
                info!(
                    "replay bootstrap complete: {} events, \
                     caught_up={}, last_seq={}",
                    result.events.len(),
                    result.caught_up,
                    result.last_seq
                );
                for raw in result.events {
                    apply_record(
                        &state,
                        raw.header.record_type,
                        &raw.payload,
                        config.max_outbound,
                    );
                }
            }
            Err(e) => {
                info!("replay bootstrap failed: {}", e);
            }
        }
    }

    let me_addrs = rsx_marketdata::config::me_cast_addrs_from_env();

    let mut cast_receivers: Vec<CastReceiver> = me_addrs
        .iter()
        .map(|me_addr| {
            let md_port = me_addr.port() + 400;
            let bind_addr: SocketAddr = format!("127.0.0.1:{}", md_port)
                .parse()
                .expect("invalid MD cast bind addr");
            CastReceiver::new(bind_addr, *me_addr).expect("failed to bind marketdata cast")
        })
        .collect();

    info!(
        "marketdata started on {} subscribing to {} ME(s): {}",
        config.listen_addr,
        cast_receivers.len(),
        me_addrs
            .iter()
            .map(|a| a.to_string())
            .collect::<Vec<_>>()
            .join(","),
    );

    if let Ok(core_str) = std::env::var("RSX_MD_CORE_ID") {
        if let Ok(core_id) = core_str.parse::<usize>() {
            let setup = rsx_types::cpu::setup_hot_thread(core_id);
            tracing::info!("marketdata {}", setup);
            if setup.isolated == Some(false) {
                tracing::warn!(
                    "marketdata core {} not isolated — expect tail spikes",
                    core_id
                );
            }
        }
    }

    let gauges: Arc<LoadGauges> = LoadGauges::new();
    gauges.live.store(true, Ordering::Relaxed);
    gauges.ready.store(true, Ordering::Relaxed);
    gauges.state_idx.store(4, Ordering::Relaxed);
    if let Ok(addr_str) = env::var("RSX_MD_HEALTH_ADDR") {
        if let Ok(addr) = addr_str.parse::<SocketAddr>() {
            let g = gauges.clone();
            let max_out = config.max_outbound as u64;
            rsx_health::spawn_health_server(addr, move || {
                let conns = g.connections.load(Ordering::Relaxed);
                let drops = g.drops.load(Ordering::Relaxed);
                let saturation = if max_out > 0 {
                    (drops as f64 / (conns.max(1) as f64 * max_out as f64)).min(1.0)
                } else {
                    0.0
                };
                HealthSnapshot {
                    live: g.live.load(Ordering::Relaxed),
                    ready: g.ready.load(Ordering::Relaxed),
                    saturation,
                    queues: vec![QueueGauge {
                        name: "subscribers",
                        used: conns,
                        cap: 65536,
                    }],
                    counters: vec![
                        CounterGauge {
                            name: "drops",
                            value: drops,
                        },
                        CounterGauge {
                            name: "publishes",
                            value: g.publishes.load(Ordering::Relaxed),
                        },
                    ],
                    state: g.state_label(),
                }
            });
        } else {
            warn!("RSX_MD_HEALTH_ADDR: invalid addr '{addr_str}'");
        }
    }

    let mut rt = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .enable_timer()
        .build()
        .expect("failed to build monoio runtime");

    let shutdown = install_shutdown_handler();

    let gauges_inner = gauges.clone();
    rt.block_on(async move {
        let ws_addr = config.listen_addr.clone();
        let state_accept = state.clone();
        let max_outbound = config.max_outbound;
        let snapshot_depth = config.snapshot_depth;
        monoio::spawn(async move {
            if let Err(e) = rsx_marketdata::ws::ws_accept_loop(&ws_addr, move |stream| {
                let st = state_accept.clone();
                monoio::spawn(async move {
                    handle_connection(stream, st, max_outbound, snapshot_depth).await;
                });
            })
            .await
            {
                tracing::error!("ws accept error: {e}");
            }
        });

        const NS_PER_MS: u64 = 1_000_000;
        let heartbeat_interval_ns = config.heartbeat_interval_ms * NS_PER_MS;
        let heartbeat_timeout_ns = config.heartbeat_timeout_ms * NS_PER_MS;
        let mut last_heartbeat_ns = time_ns();
        let mut last_timeout_check_ns = time_ns();
        let mut last_evict_ns = time_ns();
        const BOOK_TTL_NS: u64 = 60_000_000_000;

        let mut stream_expected: Vec<u64> = vec![0; cast_receivers.len()];

        loop {
            if shutdown.load(Ordering::SeqCst) {
                info!("shutdown signal received, exiting");
                break;
            }
            for (ri, cast_receiver) in cast_receivers.iter_mut().enumerate() {
                loop {
                    let expected = &mut stream_expected[ri];
                    let recv = cast_receiver.try_recv_with(|hdr, payload| {
                        if let Some(seq) = extract_seq(payload) {
                            if *expected != 0 && seq > *expected {
                                tracing::warn!(
                                    "marketdata stream seq gap \
                                 expected={} got={}",
                                    *expected,
                                    seq,
                                );
                                let mut st = state.borrow_mut();
                                st.note_gap();
                                st.resend_all_snapshots(config.snapshot_depth, config.max_outbound);
                            }
                            if seq >= *expected {
                                *expected = seq + 1;
                            }
                        }

                        apply_record(&state, hdr.record_type, payload, config.max_outbound);
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
                                &state,
                                config.max_outbound,
                                config.stream_id,
                                last_delivered_seq,
                                Some((gap_start, gap_end_inclusive)),
                                config.replay_addr.as_deref(),
                                &config.tip_file,
                            );
                            cast_receiver.reset_after_replay(new_tip);
                            continue;
                        }
                        CastRecvWith::Reconnect { last_delivered_seq } => {
                            let new_tip = handle_replay(
                                &state,
                                config.max_outbound,
                                config.stream_id,
                                last_delivered_seq,
                                None,
                                config.replay_addr.as_deref(),
                                &config.tip_file,
                            );
                            cast_receiver.reset_after_replay(new_tip);
                            continue;
                        }
                    }
                }
            }

            let now = time_ns();
            if now.saturating_sub(last_heartbeat_ns) >= heartbeat_interval_ns {
                let ts_ms = time_ms();
                state.borrow_mut().broadcast_heartbeat(ts_ms);
                last_heartbeat_ns = now;
            }

            if now.saturating_sub(last_timeout_check_ns) >= heartbeat_timeout_ns {
                let timed_out = state.borrow_mut().check_timeouts(heartbeat_timeout_ns);
                for conn_id in timed_out {
                    info!("conn {} timed out (no heartbeat)", conn_id);
                }
                last_timeout_check_ns = now;
            }

            if now.saturating_sub(last_evict_ns) >= BOOK_TTL_NS {
                state.borrow_mut().evict_stale_books(BOOK_TTL_NS);
                last_evict_ns = now;
            }

            {
                let st = state.borrow();
                gauges_inner
                    .connections
                    .store(st.connection_count() as u64, Ordering::Relaxed);
                gauges_inner.drops.store(st.gap_count(), Ordering::Relaxed);
            }

            monoio::time::sleep(Duration::ZERO).await;
        }
    });
}
