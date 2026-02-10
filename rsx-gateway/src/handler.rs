use crate::convert::validate_lot_alignment;
use crate::convert::validate_tick_alignment;
use crate::order_id::generate_order_id;
use crate::order_id::hex_to_order_id;
use crate::pending::PendingOrder;
use crate::protocol::parse;
use crate::protocol::serialize;
use crate::protocol::CancelKey;
use crate::protocol::WsFrame;
use crate::rate_limit::per_ip;
use crate::rate_limit::per_user;
use crate::state::GatewayState;
use crate::ws::ws_handshake;
use crate::ws::ws_read_frame;
use crate::ws::ws_write_text;
use monoio::net::TcpStream;
use rsx_dxs::cmp::CmpSender;
use rsx_dxs::records::CancelRequest;
use rsx_dxs::records::RECORD_CANCEL_REQUEST;
use rsx_dxs::records::RECORD_ORDER_REQUEST;
use rsx_risk::types::OrderRequest;
use rsx_types::time::time_ms;
use rsx_types::time::time_ns;
use std::cell::RefCell;
use std::net::SocketAddr;
use std::rc::Rc;
use tracing::info;
use tracing::warn;

pub async fn handle_connection(
    mut stream: TcpStream,
    peer: SocketAddr,
    state: Rc<RefCell<GatewayState>>,
    cmp_sender: Rc<RefCell<CmpSender>>,
    jwt_secret: &str,
) {
    let user_id = match ws_handshake(
        &mut stream,
        jwt_secret,
    )
    .await
    {
        Ok((_key, uid)) => uid,
        Err(e) => {
            warn!("handshake failed: {e}");
            return;
        }
    };

    let conn_id =
        state.borrow_mut().add_connection(user_id);
    state.borrow_mut().touch_connection(
        conn_id,
        time_ns(),
    );
    info!("connection {} user {}", conn_id, user_id);

    loop {
        let msgs =
            state.borrow_mut().drain_outbound(conn_id);
        for msg in msgs {
            if let Err(e) =
                ws_write_text(&mut stream, msg.as_bytes())
                    .await
            {
                warn!("write error conn {}: {e}", conn_id);
                state
                    .borrow_mut()
                    .remove_connection(conn_id);
                return;
            }
        }

        let (opcode, payload) =
            match ws_read_frame(&mut stream).await {
                Ok(f) => f,
                Err(e) => {
                    info!(
                        "conn {} closed: {e}",
                        conn_id
                    );
                    state
                        .borrow_mut()
                        .remove_connection(conn_id);
                    return;
                }
            };

        state.borrow_mut().touch_connection(
            conn_id,
            time_ns(),
        );

        if opcode == 8 {
            state
                .borrow_mut()
                .remove_connection(conn_id);
            return;
        }

        if opcode == 9 {
            let mut pong = vec![0x8A, 0x00];
            if !payload.is_empty() {
                pong[1] = payload.len() as u8;
                pong.extend_from_slice(&payload);
            }
            let _ = ws_write_text(
                &mut stream,
                &pong,
            )
            .await;
            continue;
        }

        if opcode != 1 {
            continue;
        }

        let text = match std::str::from_utf8(&payload)
        {
            Ok(s) => s,
            Err(_) => {
                let err = serialize(
                    &WsFrame::Error {
                        code: 1001,
                        message: "invalid utf8"
                            .to_string(),
                    },
                );
                let _ = ws_write_text(
                    &mut stream,
                    err.as_bytes(),
                )
                .await;
                continue;
            }
        };

        let frame = match parse(text) {
            Ok(f) => f,
            Err(e) => {
                let err = serialize(
                    &WsFrame::Error {
                        code: 1002,
                        message: e.to_string(),
                    },
                );
                let _ = ws_write_text(
                    &mut stream,
                    err.as_bytes(),
                )
                .await;
                continue;
            }
        };

        match frame {
            WsFrame::NewOrder {
                symbol_id,
                side,
                price,
                qty,
                client_order_id,
                tif,
                reduce_only,
            } => {
                {
                    let mut st = state.borrow_mut();
                    let ip_limiter = st
                        .ip_limiters
                        .entry(peer.ip())
                        .or_insert_with(per_ip);
                    if !ip_limiter.try_consume() {
                        drop(st);
                        send_error(
                            &mut stream,
                            1006,
                            "rate limited",
                        )
                        .await;
                        continue;
                    }
                }

                {
                    let mut st = state.borrow_mut();
                    let limiter = st
                        .user_limiters
                        .entry(user_id)
                        .or_insert_with(per_user);
                    if !limiter.try_consume() {
                        drop(st);
                        send_error(
                            &mut stream,
                            1006,
                            "rate limited",
                        )
                        .await;
                        continue;
                    }
                }

                {
                    let mut st = state.borrow_mut();
                    if !st.circuit.allow() {
                        drop(st);
                        send_error(
                            &mut stream,
                            5,
                            "overloaded",
                        )
                        .await;
                        continue;
                    }
                }

                // Tick/lot validation
                {
                    let st = state.borrow();
                    let sid = symbol_id as usize;
                    if sid >= st.symbol_configs.len() {
                        drop(st);
                        send_error(
                            &mut stream,
                            1007,
                            "unknown symbol",
                        )
                        .await;
                        continue;
                    }
                    let cfg = &st.symbol_configs[sid];
                    if !validate_tick_alignment(
                        price,
                        cfg.tick_size,
                    ) {
                        drop(st);
                        send_error(
                            &mut stream,
                            1008,
                            "price not tick aligned",
                        )
                        .await;
                        continue;
                    }
                    if !validate_lot_alignment(
                        qty, cfg.lot_size,
                    ) {
                        drop(st);
                        send_error(
                            &mut stream,
                            1009,
                            "qty not lot aligned",
                        )
                        .await;
                        continue;
                    }
                }

                let oid = generate_order_id();
                let now_ns = time_ns();

                let mut cid_bytes = [0u8; 20];
                let src = client_order_id.as_bytes();
                let len = src.len().min(20);
                cid_bytes[..len]
                    .copy_from_slice(&src[..len]);

                // SAFETY: oid is [u8; 16], slices are exact
                let oid_hi = u64::from_be_bytes(
                    oid[0..8].try_into().unwrap(),
                );
                let oid_lo = u64::from_be_bytes(
                    oid[8..16].try_into().unwrap(),
                );

                let order = OrderRequest {
                    seq: 0,
                    user_id,
                    symbol_id,
                    price,
                    qty,
                    order_id_hi: oid_hi,
                    order_id_lo: oid_lo,
                    timestamp_ns: now_ns,
                    side,
                    tif,
                    reduce_only,
                    is_liquidation: false,
                    _pad: [0; 4],
                };

                let pending = PendingOrder {
                    order_id: oid,
                    user_id,
                    symbol_id,
                    client_order_id: cid_bytes,
                    timestamp_ns: now_ns,
                };
                {
                    let mut st = state.borrow_mut();
                    if !st.pending.push(pending) {
                        let err = serialize(
                            &WsFrame::Error {
                                code: 1003,
                                message:
                                    "pending queue full"
                                        .to_string(),
                            },
                        );
                        let _ = ws_write_text(
                            &mut stream,
                            err.as_bytes(),
                        )
                        .await;
                        continue;
                    }
                }

                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        &order as *const OrderRequest
                            as *const u8,
                        std::mem::size_of::<
                            OrderRequest,
                        >(),
                    )
                };
                let _ = cmp_sender.borrow_mut().send_raw(
                    RECORD_ORDER_REQUEST,
                    bytes,
                );
                state
                    .borrow_mut()
                    .circuit
                    .record_success();

                let _ = (oid, qty);
            }
            WsFrame::Cancel { key } => {
                let st = state.borrow();
                match key {
                    CancelKey::OrderId(ref hex) => {
                        let oid_bytes =
                            match hex_to_order_id(hex) {
                                Some(b) => b,
                                None => {
                                    drop(st);
                                    let err = serialize(
                                        &WsFrame::Error {
                                            code: 1005,
                                            message:
                                                "invalid order id"
                                                    .to_string(),
                                        },
                                    );
                                    let _ = ws_write_text(
                                        &mut stream,
                                        err.as_bytes(),
                                    )
                                    .await;
                                    continue;
                                }
                            };
                        let found = st
                            .pending
                            .find_by_order_id(&oid_bytes);
                        if let Some(p) = found {
                            let cancel = build_cancel(
                                user_id,
                                p.symbol_id,
                                &p.order_id,
                            );
                            drop(st);
                            send_cancel(
                                &cmp_sender, &cancel,
                            );
                        } else {
                            drop(st);
                            send_error(
                                &mut stream,
                                1005,
                                "order not found",
                            )
                            .await;
                        }
                    }
                    CancelKey::ClientOrderId(
                        ref cid_str,
                    ) => {
                        let mut cid = [0u8; 20];
                        let src = cid_str.as_bytes();
                        let len = src.len().min(20);
                        cid[..len]
                            .copy_from_slice(&src[..len]);
                        let found = st
                            .pending
                            .find_by_client_order_id(&cid);
                        if let Some(p) = found {
                            let cancel = build_cancel(
                                user_id,
                                p.symbol_id,
                                &p.order_id,
                            );
                            drop(st);
                            send_cancel(
                                &cmp_sender, &cancel,
                            );
                        } else {
                            drop(st);
                            send_error(
                                &mut stream,
                                1005,
                                "order not found",
                            )
                            .await;
                        }
                    }
                }
            }
            WsFrame::Heartbeat {..} => {
                let now_ms = time_ms();
                let resp = serialize(
                    &WsFrame::Heartbeat {
                        timestamp_ms: now_ms,
                    },
                );
                let _ = ws_write_text(
                    &mut stream,
                    resp.as_bytes(),
                )
                .await;
            }
            _ => {
                let err = serialize(
                    &WsFrame::Error {
                        code: 1004,
                        message: "unsupported"
                            .to_string(),
                    },
                );
                let _ = ws_write_text(
                    &mut stream,
                    err.as_bytes(),
                )
                .await;
            }
        }
    }
}

fn build_cancel(
    user_id: u32,
    symbol_id: u32,
    order_id: &[u8; 16],
) -> CancelRequest {
    // SAFETY: order_id is &[u8; 16], slices are exact
    let oid_hi =
        u64::from_be_bytes(order_id[0..8].try_into().unwrap());
    let oid_lo =
        u64::from_be_bytes(order_id[8..16].try_into().unwrap());
    CancelRequest {
        seq: 0,
        ts_ns: time_ns(),
        user_id,
        symbol_id,
        order_id_hi: oid_hi,
        order_id_lo: oid_lo,
        _pad: [0; 24],
    }
}

fn send_cancel(
    cmp_sender: &Rc<RefCell<CmpSender>>,
    cancel: &CancelRequest,
) {
    let bytes = unsafe {
        std::slice::from_raw_parts(
            cancel as *const CancelRequest as *const u8,
            std::mem::size_of::<CancelRequest>(),
        )
    };
    let _ = cmp_sender
        .borrow_mut()
        .send_raw(RECORD_CANCEL_REQUEST, bytes);
}

async fn send_error(
    stream: &mut TcpStream,
    code: u32,
    message: &str,
) {
    let err = serialize(&WsFrame::Error {
        code,
        message: message.to_string(),
    });
    let _ =
        ws_write_text(stream, err.as_bytes()).await;
}
