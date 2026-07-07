use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecvWith;
use rsx_cast::decode_payload;
use rsx_cast::wal::extract_seq;
use rsx_health::CounterGauge;
use rsx_health::HealthSnapshot;
use rsx_health::LoadGauges;
use rsx_health::QueueGauge;
use rsx_marketdata::config::load_marketdata_config;
use rsx_marketdata::handler::handle_connection;
use rsx_marketdata::records::serialize_bbo;
use rsx_marketdata::records::serialize_l2_delta;
use rsx_marketdata::records::serialize_trade;
use rsx_marketdata::replay::run_replay_bootstrap_blocking;
use rsx_marketdata::state::MarketDataState;
use rsx_messages::FillRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_types::install_panic_handler;
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

/// Drain ME's replication stream after the marketdata's cast
/// receiver hit a sticky FAULTED or RECONNECT. Mirrors
/// `rsx_risk::main::handle_replay` and
/// `rsx_gateway::main::handle_replay`. Round-1 apply just
/// logs records — the shadow book recovers indirectly via
/// the snapshot-resend triggered by the live-stream seq-gap
/// detector after `reset_after_replay`.
fn handle_replay(
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
    // Retry while the producer's WAL hasn't flushed the gap
    // records yet (10ms flush window). Without this, replay
    // returns the old tip, the receiver resets to it, and the
    // next UDP packet FAULTs again — an infinite loop.
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
            |raw| {
                let seq = rsx_cast::wal::extract_seq(&raw.payload).unwrap_or(0);
                tracing::debug!(
                    "marketdata replay applied record_type={} seq={}",
                    raw.header.record_type,
                    seq,
                );
            },
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

    let mut state = MarketDataState::new(
        config.max_symbols,
        base_config.clone(),
        config.book_capacity,
        config.mid_price,
    );

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
                for event in result.events {
                    if let Some(rec) = event.insert {
                        state.ensure_book(rec.symbol_id, rec.price.0);
                        if let Some(book) = state.book_mut(rec.symbol_id) {
                            book.apply_insert_by_id(
                                rec.price.0,
                                rec.qty.0,
                                rec.side,
                                rec.user_id,
                                rec.ts_ns,
                                rec.order_id_hi,
                                rec.order_id_lo,
                            );
                        }
                    } else if let Some(rec) = event.cancel {
                        if let Some(book) = state.book_mut(rec.symbol_id) {
                            book.apply_cancel_by_order_id(
                                rec.order_id_hi,
                                rec.order_id_lo,
                                rec.ts_ns,
                            );
                        }
                    } else if let Some(rec) = event.fill {
                        if let Some(book) = state.book_mut(rec.symbol_id) {
                            book.apply_fill_by_order_id(
                                rec.maker_order_id_hi,
                                rec.maker_order_id_lo,
                                rec.qty.0,
                                rec.ts_ns,
                            );
                        }
                    }
                }
            }
            Err(e) => {
                info!("replay bootstrap failed: {}", e);
            }
        }
    }

    let state = Rc::new(RefCell::new(state));

    let me_addrs = rsx_marketdata::config::me_cast_addrs_from_env();

    // One CastReceiver per ME. Local bind port derived from
    // ME port: BASE_MD_CAST(9500) = BASE_ME_CAST(9100) + 400.
    let mut cast_receivers: Vec<CastReceiver> = me_addrs
        .iter()
        .map(|me_addr| {
            let md_port = me_addr.port() + 400;
            let bind_addr: SocketAddr = format!("127.0.0.1:{}", md_port)
                .parse()
                // SAFETY: fail-fast at startup
                .expect("invalid MD cast bind addr");
            CastReceiver::new(bind_addr, *me_addr)
                // SAFETY: fail-fast at startup
                .expect("failed to bind marketdata cast")
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

    // Health server: RSX_MD_HEALTH_ADDR=127.0.0.1:9203
    // GET /health → 200/503 liveness
    // GET /ready   → 200/503 readiness
    // GET /metrics → JSON (subscriber count, drop counter)
    let gauges: Arc<LoadGauges> = LoadGauges::new();
    gauges.live.store(true, Ordering::Relaxed);
    gauges.ready.store(true, Ordering::Relaxed);
    gauges.state_idx.store(4, Ordering::Relaxed); // "running"
    if let Ok(addr_str) = env::var("RSX_MD_HEALTH_ADDR") {
        if let Ok(addr) = addr_str.parse::<SocketAddr>() {
            let g = gauges.clone();
            let max_out = config.max_outbound as u64;
            rsx_health::spawn_health_server(addr, move || {
                let conns = g.connections.load(Ordering::Relaxed);
                let drops = g.drops.load(Ordering::Relaxed);
                // Use connections as a proxy for subscriber
                // pressure; saturation from drop rate.
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

    // Run monoio event loop
    let mut rt = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .enable_timer()
        .build()
        // SAFETY: fail-fast at startup
        .expect("failed to build monoio runtime");

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

        // Per-stream expected seq (one CastReceiver per ME stream).
        let mut stream_expected: Vec<u64> = vec![0; cast_receivers.len()];

        loop {
            for (ri, cast_receiver) in cast_receivers.iter_mut().enumerate() {
                loop {
                    let expected = &mut stream_expected[ri];
                    let recv = cast_receiver.try_recv_with(|hdr, payload| {
                        // Stream-level seq continuity. ME's SEQ-1 fan-out
                        // puts every record type on one monotonic
                        // per-stream seq — including ORDER_DONE /
                        // ORDER_FAILED / BBO / CONFIG_APPLIED that
                        // marketdata receives but ignores. Advance the
                        // counter on EVERY record: a per-symbol counter
                        // over only the handled types fabricated a gap for
                        // each ignored record (the delta=1 warn storm) and
                        // fired a needless snapshot resend each time. The
                        // CastReceiver delivers contiguously or FAULTs, so
                        // a gap seen here is a genuine unrecoverable loss —
                        // resend snapshots so clients rebuild.
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
                            // seq < expected: already-delivered dup, ignore
                        }

                        match hdr.record_type {
                            RECORD_ORDER_INSERTED => {
                                if let Some(rec) = decode_payload::<OrderInsertedRecord>(payload) {
                                    handle_insert(&state, &rec, config.max_outbound);
                                }
                            }
                            RECORD_ORDER_CANCELLED => {
                                if let Some(rec) = decode_payload::<OrderCancelledRecord>(payload) {
                                    handle_cancel(&state, &rec, config.max_outbound);
                                }
                            }
                            RECORD_FILL => {
                                if let Some(rec) = decode_payload::<FillRecord>(payload) {
                                    handle_fill(&state, &rec, config.max_outbound);
                                }
                            }
                            RECORD_ORDER_DONE => {
                                // Not routed to marketdata per spec.
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
            } // for cast_receiver

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

            // Publish load gauges (relaxed stores; once per
            // monoio yield — not per message).
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

fn handle_insert(
    state: &Rc<RefCell<MarketDataState>>,
    rec: &OrderInsertedRecord,
    max_outbound: usize,
) {
    let mut st = state.borrow_mut();
    st.ensure_book(rec.symbol_id, rec.price.0);
    let (side, price) = match st.book_mut(rec.symbol_id) {
        Some(book) => {
            book.apply_insert_by_id(
                rec.price.0,
                rec.qty.0,
                rec.side,
                rec.user_id,
                rec.ts_ns,
                rec.order_id_hi,
                rec.order_id_lo,
            );
            (rec.side, rec.price.0)
        }
        None => return,
    };
    broadcast_updates(&mut st, rec.symbol_id, side, price, max_outbound);
}

fn handle_cancel(
    state: &Rc<RefCell<MarketDataState>>,
    rec: &OrderCancelledRecord,
    max_outbound: usize,
) {
    let mut st = state.borrow_mut();
    let update = match st.book_mut(rec.symbol_id) {
        Some(book) => book.apply_cancel_by_order_id(rec.order_id_hi, rec.order_id_lo, rec.ts_ns),
        None => None,
    };
    if let Some((side, price)) = update {
        broadcast_updates(&mut st, rec.symbol_id, side, price, max_outbound);
    }
}

fn handle_fill(state: &Rc<RefCell<MarketDataState>>, rec: &FillRecord, max_outbound: usize) {
    let mut st = state.borrow_mut();
    let mut trade_msg: Option<Arc<str>> = None;
    let update = match st.book_mut(rec.symbol_id) {
        Some(book) => {
            let update = book.apply_fill_by_order_id(
                rec.maker_order_id_hi,
                rec.maker_order_id_lo,
                rec.qty.0,
                rec.ts_ns,
            );
            if update.is_some() {
                let trade = book.make_trade(rec.price.0, rec.qty.0, rec.taker_side, rec.ts_ns);
                trade_msg = Some(serialize_trade(&trade).into());
            }
            update
        }
        None => None,
    };
    if let Some((side, price)) = update {
        broadcast_updates(&mut st, rec.symbol_id, side, price, max_outbound);
    }
    if let Some(msg) = trade_msg {
        let clients = st.clients_for_symbol(rec.symbol_id);
        for client_id in clients {
            if st.has_trades(client_id, rec.symbol_id)
                // HEAP: fan-out clone — push_to_client takes owned String
                // (per-client VecDeque<String> outbound). One clone per
                // subscriber per trade. JSON broadcast path, acceptable
                // per spec; binary fan-out would replace this.
                && !st.push_to_client(
                    client_id,
                    msg.clone(),
                    max_outbound,
                )
            {
                let depth = st.client_depth(client_id);
                st.send_snapshot_to_client(client_id, rec.symbol_id, depth, max_outbound);
            }
        }
    }
}

fn broadcast_updates(
    st: &mut MarketDataState,
    symbol_id: u32,
    side: u8,
    price: i64,
    max_outbound: usize,
) {
    let (delta, bbo) = match st.book_mut(symbol_id) {
        Some(book) => {
            let delta = book.derive_l2_delta(side, price);
            let bbo = book.derive_bbo();
            (delta, bbo)
        }
        None => return,
    };
    let delta_msg: Arc<str> = serialize_l2_delta(&delta).into();
    let mut bbo_msg: Option<Arc<str>> = None;
    if let Some(bbo) = bbo {
        let changed = match st.last_bbo_mut(symbol_id) {
            Some(last) => {
                if last.as_ref() != Some(&bbo) {
                    *last = Some(bbo.clone());
                    true
                } else {
                    false
                }
            }
            None => true,
        };
        if changed {
            bbo_msg = Some(serialize_bbo(&bbo).into());
        }
    }

    let clients = st.clients_for_symbol(symbol_id);
    for client_id in clients {
        if st.has_depth(client_id, symbol_id)
            // HEAP: fan-out clone of L2 delta JSON per depth subscriber.
            // push_to_client requires owned String. JSON broadcast path,
            // acceptable per spec.
            && !st.push_to_client(
                client_id,
                delta_msg.clone(),
                max_outbound,
            )
        {
            st.send_snapshot_to_client(
                client_id,
                symbol_id,
                st.client_depth(client_id),
                max_outbound,
            );
        }
        if let Some(ref msg) = bbo_msg {
            if st.has_bbo(client_id, symbol_id) {
                // HEAP: fan-out clone of BBO JSON per BBO subscriber.
                // Same rationale as delta clone above.
                // SAFETY: BBO is "latest wins" state; if
                // client outbound is full the next BBO
                // update supersedes anyway.
                let _accepted = st.push_to_client(client_id, msg.clone(), max_outbound);
            }
        }
    }
}
