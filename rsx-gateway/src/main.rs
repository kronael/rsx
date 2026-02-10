use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::cmp::CmpSender;
use rsx_dxs::records::FillRecord;
use rsx_dxs::records::OrderCancelledRecord;
use rsx_dxs::records::OrderDoneRecord;
use rsx_dxs::records::RECORD_FILL;
use rsx_dxs::records::RECORD_ORDER_CANCELLED;
use rsx_dxs::records::RECORD_ORDER_DONE;
use rsx_dxs::records::RECORD_ORDER_RESPONSE;
use rsx_gateway::config::load_gateway_config;
use rsx_gateway::handler::handle_connection;
use rsx_gateway::order_id::order_id_to_hex;
use rsx_gateway::protocol::serialize;
use rsx_gateway::protocol::WsFrame;
use rsx_gateway::state::GatewayState;
use rsx_risk::rings::OrderResponse;
use rsx_types::install_panic_handler;
use std::cell::RefCell;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
use tracing::info;

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt::init();

    let config = load_gateway_config();

    let risk_addr: SocketAddr =
        env::var("RSX_RISK_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9101".into())
            .parse()
            .expect("invalid RSX_RISK_CMP_ADDR");
    let gw_addr: SocketAddr =
        env::var("RSX_GW_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9102".into())
            .parse()
            .expect("invalid RSX_GW_CMP_ADDR");
    let wal_dir = env::var("RSX_GW_WAL_DIR")
        .unwrap_or_else(|_| "./tmp/wal".into());

    // CMP/UDP: send orders to Risk
    let cmp_sender = CmpSender::new(
        risk_addr,
        0,
        &PathBuf::from(&wal_dir),
    )
    .expect("failed to create CMP sender");

    // CMP/UDP: receive responses from Risk
    let mut cmp_receiver = CmpReceiver::new(
        gw_addr, risk_addr, 0,
    )
    .expect("failed to bind CMP receiver");

    info!(
        "gateway started on {}",
        config.listen_addr,
    );

    let max_pending = config.max_pending;
    let circuit_threshold = config.circuit_threshold;
    let circuit_cooldown_ms = config.circuit_cooldown_ms;

    // Run monoio event loop
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .build()
    .expect("failed to build monoio runtime");

    let listen_addr = config.listen_addr.clone();
    rt.block_on(async move {
        let state = Rc::new(RefCell::new(
            GatewayState::new(
                max_pending,
                circuit_threshold,
                circuit_cooldown_ms,
            ),
        ));
        let sender =
            Rc::new(RefCell::new(cmp_sender));

        // Spawn WS accept loop
        let ws_addr = listen_addr;
        let state_accept = state.clone();
        let sender_accept = sender.clone();
        monoio::spawn(async move {
            if let Err(e) =
                rsx_gateway::ws::ws_accept_loop(
                    &ws_addr,
                    move |stream| {
                        let st = state_accept.clone();
                        let snd =
                            sender_accept.clone();
                        // TODO: authenticate user_id
                        // from handshake headers.
                        // For now use 0.
                        monoio::spawn(async move {
                            handle_connection(
                                stream, st, snd, 0,
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

        // CMP polling loop (yields to monoio)
        loop {
            while let Some((hdr, payload)) =
                cmp_receiver.try_recv()
            {
                match hdr.record_type {
                    RECORD_ORDER_RESPONSE
                        if payload.len()
                            >= std::mem::size_of::<
                                OrderResponse,
                            >() =>
                    {
                        let resp = unsafe {
                            std::ptr::read(
                                payload.as_ptr()
                                    as *const
                                        OrderResponse,
                            )
                        };
                        route_response(
                            &state, &resp,
                        );
                    }
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
                    _ => {}
                }
            }

            let _ = sender.borrow_mut().tick();
            cmp_receiver.tick();
            sender.borrow_mut().recv_control();

            // Yield to monoio scheduler
            monoio::time::sleep(
                std::time::Duration::from_micros(100),
            )
            .await;
        }
    });
}

fn oid_hex(hi: u64, lo: u64) -> String {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&hi.to_be_bytes());
    bytes[8..].copy_from_slice(&lo.to_be_bytes());
    order_id_to_hex(&bytes)
}

fn oid_bytes(hi: u64, lo: u64) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&hi.to_be_bytes());
    bytes[8..].copy_from_slice(&lo.to_be_bytes());
    bytes
}

fn route_response(
    state: &Rc<RefCell<GatewayState>>,
    resp: &OrderResponse,
) {
    match resp {
        OrderResponse::Accepted {
            user_id,
            margin_reserved: _,
            order_id_hi,
            order_id_lo,
        } => {
            let msg = serialize(
                &WsFrame::OrderUpdate {
                    order_id: oid_hex(
                        *order_id_hi,
                        *order_id_lo,
                    ),
                    status: 1, // accepted
                    filled_qty: 0,
                    remaining_qty: 0,
                    reason: 0,
                },
            );
            state
                .borrow_mut()
                .push_to_user(*user_id, msg);
        }
        OrderResponse::Rejected {
            user_id,
            reason,
            order_id_hi,
            order_id_lo,
        } => {
            let reason_code = match reason {
                rsx_risk::RejectReason::InsufficientMargin => 1,
                rsx_risk::RejectReason::UserInLiquidation => 2,
                rsx_risk::RejectReason::NotInShard => 3,
            };
            let msg = serialize(
                &WsFrame::OrderUpdate {
                    order_id: oid_hex(
                        *order_id_hi,
                        *order_id_lo,
                    ),
                    status: 3, // rejected
                    filled_qty: 0,
                    remaining_qty: 0,
                    reason: reason_code,
                },
            );
            let oid = oid_bytes(
                *order_id_hi,
                *order_id_lo,
            );
            let mut st = state.borrow_mut();
            st.push_to_user(*user_id, msg);
            st.pending.remove(&oid);
        }
    }
}

fn route_fill(
    state: &Rc<RefCell<GatewayState>>,
    rec: &FillRecord,
) {
    let taker_oid =
        oid_hex(rec.taker_order_id_hi, rec.taker_order_id_lo);
    let maker_oid =
        oid_hex(rec.maker_order_id_hi, rec.maker_order_id_lo);
    let msg = serialize(&WsFrame::Fill {
        taker_order_id: taker_oid.clone(),
        maker_order_id: maker_oid.clone(),
        price: rec.price,
        qty: rec.qty,
        timestamp_ns: rec.ts_ns,
        fee: 0,
    });
    let mut st = state.borrow_mut();
    st.push_to_user(rec.taker_user_id, msg.clone());
    st.push_to_user(rec.maker_user_id, msg);
}

fn route_order_done(
    state: &Rc<RefCell<GatewayState>>,
    rec: &OrderDoneRecord,
) {
    let oid = oid_hex(rec.order_id_hi, rec.order_id_lo);
    let msg = serialize(&WsFrame::OrderUpdate {
        order_id: oid,
        status: 2, // done
        filled_qty: rec.filled_qty,
        remaining_qty: rec.remaining_qty,
        reason: 0,
    });
    let oid_bytes_val =
        oid_bytes(rec.order_id_hi, rec.order_id_lo);
    let mut st = state.borrow_mut();
    st.push_to_user(rec.user_id, msg);
    st.pending.remove(&oid_bytes_val);
}

fn route_order_cancelled(
    state: &Rc<RefCell<GatewayState>>,
    rec: &OrderCancelledRecord,
) {
    let oid = oid_hex(rec.order_id_hi, rec.order_id_lo);
    let msg = serialize(&WsFrame::OrderUpdate {
        order_id: oid,
        status: 3, // cancelled
        filled_qty: 0,
        remaining_qty: rec.remaining_qty,
        reason: 0,
    });
    let oid_bytes_val =
        oid_bytes(rec.order_id_hi, rec.order_id_lo);
    let mut st = state.borrow_mut();
    st.push_to_user(rec.user_id, msg);
    st.pending.remove(&oid_bytes_val);
}
