use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::records::FillRecord;
use rsx_dxs::records::OrderCancelledRecord;
use rsx_dxs::records::OrderInsertedRecord;
use rsx_dxs::records::RECORD_FILL;
use rsx_dxs::records::RECORD_ORDER_CANCELLED;
use rsx_dxs::records::RECORD_ORDER_DONE;
use rsx_dxs::records::RECORD_ORDER_INSERTED;
use rsx_marketdata::config::load_marketdata_config;
use rsx_marketdata::handler::handle_connection;
use rsx_marketdata::protocol::serialize_bbo;
use rsx_marketdata::protocol::serialize_l2_delta;
use rsx_marketdata::protocol::serialize_trade;
use rsx_marketdata::state::MarketDataState;
use rsx_types::SymbolConfig;
use rsx_types::install_panic_handler;
use std::cell::RefCell;
use std::env;
use std::net::SocketAddr;
use std::rc::Rc;
use tracing::info;

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt::init();

    let config = load_marketdata_config();

    let mkt_addr: SocketAddr =
        env::var("RSX_MKT_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9103".into())
            .parse()
            .expect("invalid RSX_MKT_CMP_ADDR");
    let me_addr: SocketAddr =
        env::var("RSX_ME_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9100".into())
            .parse()
            .expect("invalid RSX_ME_CMP_ADDR");

    // CMP/UDP: receive events from ME
    let mut cmp_receiver = CmpReceiver::new(
        mkt_addr, me_addr, 0,
    )
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
    .expect("failed to build monoio runtime");

    rt.block_on(async move {
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

        loop {
            while let Some((hdr, payload)) = cmp_receiver.try_recv()
            {
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

            monoio::time::sleep(
                std::time::Duration::from_micros(100),
            )
            .await;
        }
    });
}

fn handle_insert(
    state: &Rc<RefCell<MarketDataState>>,
    rec: &OrderInsertedRecord,
    max_outbound: usize,
) {
    let mut st = state.borrow_mut();
    st.ensure_book(rec.symbol_id, rec.price);
    let (side, price) = match st.book_mut(rec.symbol_id) {
        Some(book) => {
            book.apply_insert_by_id(
                rec.price,
                rec.qty,
                rec.side,
                rec.user_id,
                rec.ts_ns,
                rec.order_id_hi,
                rec.order_id_lo,
            );
            (rec.side, rec.price)
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
                rec.qty,
                rec.ts_ns,
            );
            if update.is_some() {
                let trade = book.make_trade(
                    rec.price,
                    rec.qty,
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
            if st.has_depth(client_id, rec.symbol_id) {
                st.push_to_client(client_id, msg.clone(), max_outbound);
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
            st.push_to_client(client_id, delta_msg.clone(), max_outbound);
        }
        if let Some(ref msg) = bbo_msg {
            if st.has_bbo(client_id, symbol_id) {
                st.push_to_client(client_id, msg.clone(), max_outbound);
            }
        }
    }

}
