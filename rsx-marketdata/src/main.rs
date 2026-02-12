use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::records::FillRecord;
use rsx_dxs::records::OrderCancelledRecord;
use rsx_dxs::records::OrderInsertedRecord;
use rsx_dxs::records::RECORD_FILL;
use rsx_dxs::records::RECORD_ORDER_CANCELLED;
use rsx_dxs::records::RECORD_ORDER_DONE;
use rsx_dxs::records::RECORD_ORDER_INSERTED;
use rsx_dxs::wal::extract_seq;
use rsx_marketdata::config::load_marketdata_config;
use rsx_marketdata::handler::handle_connection;
use rsx_marketdata::protocol::serialize_bbo;
use rsx_marketdata::protocol::serialize_l2_delta;
use rsx_marketdata::protocol::serialize_trade;
use rsx_marketdata::replay::run_replay_bootstrap_blocking;
use rsx_marketdata::state::MarketDataState;
use rsx_types::SymbolConfig;
use rsx_types::install_panic_handler;
use rsx_types::time::time_ms;
use rsx_types::time::time_ns;
use std::cell::RefCell;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
use tracing::info;

fn log_effective_marketdata_config(
    config: &rsx_marketdata::config::MarketDataConfig,
) {
    info!(
        "marketdata effective config: listen={} max_symbols={} snapshot_depth={} ring_size={} book_capacity={} mid_price={} tick_size={} lot_size={} price_decimals={} qty_decimals={} max_outbound={} replay_addr={} stream_id={} tip_file={} heartbeat_interval_ms={} heartbeat_timeout_ms={}",
        config.listen_addr,
        config.max_symbols,
        config.snapshot_depth,
        config.spsc_ring_size,
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
        info!(
            "starting replay bootstrap from {}",
            replay_addr
        );
        let tip_file = PathBuf::from(&config.tip_file);
        match run_replay_bootstrap_blocking(
            config.stream_id,
            replay_addr.clone(),
            tip_file,
        ) {
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
                        if let Some(book) =
                            state.book_mut(rec.symbol_id)
                        {
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
                        if let Some(book) =
                            state.book_mut(rec.symbol_id)
                        {
                            book.apply_cancel_by_order_id(
                                rec.order_id_hi,
                                rec.order_id_lo,
                                rec.ts_ns,
                            );
                        }
                    } else if let Some(rec) = event.fill {
                        if let Some(book) =
                            state.book_mut(rec.symbol_id)
                        {
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

    let mkt_addr: SocketAddr =
        env::var("RSX_MKT_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9103".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_MKT_CMP_ADDR");
    let me_addr: SocketAddr =
        env::var("RSX_ME_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9100".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_ME_CMP_ADDR");

    // CMP/UDP: receive events from ME
    let mut cmp_receiver = CmpReceiver::new(
        mkt_addr, me_addr, 0,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind marketdata CMP");

    info!(
        "marketdata started on {}",
        config.listen_addr,
    );

    // Run monoio event loop
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .build()
    // SAFETY: fail-fast at startup
    .expect("failed to build monoio runtime");

    rt.block_on(async move {

        let ws_addr = config.listen_addr.clone();
        let state_accept = state.clone();
        let max_outbound = config.max_outbound;
        let snapshot_depth = config.snapshot_depth;
        monoio::spawn(async move {
            if let Err(e) = rsx_marketdata::ws::ws_accept_loop(
                &ws_addr,
                move |stream| {
                    let st = state_accept.clone();
                    monoio::spawn(async move {
                        handle_connection(
                            stream,
                            st,
                            max_outbound,
                            snapshot_depth,
                        )
                        .await;
                    });
                },
            )
            .await
            {
                tracing::error!("ws accept error: {e}");
            }
        });

        const NS_PER_MS: u64 = 1_000_000;
        let heartbeat_interval_ns =
            config.heartbeat_interval_ms * NS_PER_MS;
        let heartbeat_timeout_ns =
            config.heartbeat_timeout_ms * NS_PER_MS;
        let mut last_heartbeat_ns = time_ns();
        let mut last_timeout_check_ns = time_ns();

        loop {
            while let Some((hdr, payload)) = cmp_receiver.try_recv()
            {
                // Seq gap detection: extract seq from payload
                // and check for gaps. On gap, resend snapshot.
                if let Some(seq) = extract_seq(&payload) {
                    let symbol_id = extract_symbol_id(
                        hdr.record_type,
                        &payload,
                    );
                    if let Some(sid) = symbol_id {
                        let gap = state
                            .borrow_mut()
                            .check_seq(sid, seq);
                        if gap {
                            tracing::warn!(
                                "seq gap symbol={} seq={}",
                                sid,
                                seq,
                            );
                            state.borrow_mut().resend_snapshot(
                                sid,
                                config.snapshot_depth,
                                config.max_outbound,
                            );
                        }
                    }
                }

                match hdr.record_type {
                    RECORD_ORDER_INSERTED => {
                        if payload.len()
                            >= std::mem::size_of::<OrderInsertedRecord>()
                        {
                            let rec = unsafe {
                                std::ptr::read_unaligned(
                                    payload.as_ptr()
                                        as *const OrderInsertedRecord,
                                )
                            };
                            handle_insert(
                                &state,
                                &rec,
                                config.max_outbound,
                            );
                        }
                    }
                    RECORD_ORDER_CANCELLED => {
                        if payload.len()
                            >= std::mem::size_of::<OrderCancelledRecord>()
                        {
                            let rec = unsafe {
                                std::ptr::read_unaligned(
                                    payload.as_ptr()
                                        as *const OrderCancelledRecord,
                                )
                            };
                            handle_cancel(
                                &state,
                                &rec,
                                config.max_outbound,
                            );
                        }
                    }
                    RECORD_FILL => {
                        if payload.len()
                            >= std::mem::size_of::<FillRecord>()
                        {
                            let rec = unsafe {
                                std::ptr::read_unaligned(
                                    payload.as_ptr()
                                        as *const FillRecord,
                                )
                            };
                            handle_fill(
                                &state,
                                &rec,
                                config.max_outbound,
                            );
                        }
                    }
                    RECORD_ORDER_DONE => {
                        // Not routed to marketdata per spec.
                    }
                    _ => {}
                }
            }

            cmp_receiver.tick();

            let now = time_ns();
            if now - last_heartbeat_ns >= heartbeat_interval_ns {
                let ts_ms = time_ms();
                state.borrow_mut().broadcast_heartbeat(ts_ms);
                last_heartbeat_ns = now;
            }

            if now - last_timeout_check_ns >= heartbeat_timeout_ns {
                let timed_out = state.borrow_mut().check_timeouts(heartbeat_timeout_ns);
                for conn_id in timed_out {
                    info!("conn {} timed out (no heartbeat)", conn_id);
                }
                last_timeout_check_ns = now;
            }

            monoio::time::sleep(
                std::time::Duration::from_micros(100),
            )
            .await;
        }
    });
}

/// Extract symbol_id from CMP record payload.
/// All records have seq(8) + ts_ns(8) + symbol_id(4) layout.
fn extract_symbol_id(
    record_type: u16,
    payload: &[u8],
) -> Option<u32> {
    match record_type {
        RECORD_ORDER_INSERTED
        | RECORD_ORDER_CANCELLED
        | RECORD_FILL => {
            if payload.len() >= 20 {
                Some(u32::from_le_bytes([
                    payload[16],
                    payload[17],
                    payload[18],
                    payload[19],
                ]))
            } else {
                None
            }
        }
        _ => None,
    }
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
    broadcast_updates(
        &mut st,
        rec.symbol_id,
        side,
        price,
        max_outbound,
    );
}

fn handle_cancel(
    state: &Rc<RefCell<MarketDataState>>,
    rec: &OrderCancelledRecord,
    max_outbound: usize,
) {
    let mut st = state.borrow_mut();
    let update = match st.book_mut(rec.symbol_id) {
        Some(book) => book.apply_cancel_by_order_id(
            rec.order_id_hi,
            rec.order_id_lo,
            rec.ts_ns,
        ),
        None => None,
    };
    if let Some((side, price)) = update {
        broadcast_updates(
            &mut st,
            rec.symbol_id,
            side,
            price,
            max_outbound,
        );
    }
}

fn handle_fill(
    state: &Rc<RefCell<MarketDataState>>,
    rec: &FillRecord,
    max_outbound: usize,
) {
    let mut st = state.borrow_mut();
    let mut trade_msg = None;
    let update = match st.book_mut(rec.symbol_id) {
        Some(book) => {
            let update = book.apply_fill_by_order_id(
                rec.maker_order_id_hi,
                rec.maker_order_id_lo,
                rec.qty.0,
                rec.ts_ns,
            );
            if update.is_some() {
                let trade = book.make_trade(
                    rec.price.0,
                    rec.qty.0,
                    rec.taker_side,
                    rec.ts_ns,
                );
                trade_msg = Some(serialize_trade(&trade));
            }
            update
        }
        None => None,
    };
    if let Some((side, price)) = update {
        broadcast_updates(
            &mut st,
            rec.symbol_id,
            side,
            price,
            max_outbound,
        );
    }
    if let Some(msg) = trade_msg {
        let clients = st.clients_for_symbol(rec.symbol_id);
        for client_id in clients {
            if st.has_trades(client_id, rec.symbol_id) {
                if !st.push_to_client(
                    client_id,
                    msg.clone(),
                    max_outbound,
                ) {
                    let depth = st.client_depth(client_id);
                    st.send_snapshot_to_client(
                        client_id,
                        rec.symbol_id,
                        depth,
                        max_outbound,
                    );
                }
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
    let delta_msg = serialize_l2_delta(&delta);
    let mut bbo_msg = None;
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
            bbo_msg = Some(serialize_bbo(&bbo));
        }
    }

    let clients = st.clients_for_symbol(symbol_id);
    for client_id in clients {
        if st.has_depth(client_id, symbol_id) {
            if !st.push_to_client(
                client_id,
                delta_msg.clone(),
                max_outbound,
            ) {
                st.send_snapshot_to_client(
                    client_id,
                    symbol_id,
                    st.client_depth(client_id),
                    max_outbound,
                );
            }
        }
        if let Some(ref msg) = bbo_msg {
            if st.has_bbo(client_id, symbol_id) {
                let _ = st.push_to_client(
                    client_id,
                    msg.clone(),
                    max_outbound,
                );
            }
        }
    }

}
