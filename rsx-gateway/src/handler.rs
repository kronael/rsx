use crate::convert::validate_lot_alignment;
use crate::convert::validate_tick_alignment;
use crate::order_id::generate_order_id;
use crate::order_id::hex_to_order_id;
use crate::order_id::order_id_to_hex;
use crate::order_id::split_order_id;
use crate::pending::PendingOrder;
use crate::pending::PendingStage;
use crate::rate_limit::RateLimiter;
use crate::rest::handle_rest;
use crate::route::update;
use crate::state::GatewayState;
use crate::ws::is_ws_upgrade;
use crate::ws::read_http_request;
use crate::ws::ws_handshake_from_request;
use crate::ws::ws_read_frame_buf;
use crate::ws::ws_write_frame;
use crate::ws::ws_write_text;
use monoio::net::TcpStream;
use rsx_cast::as_bytes;
use rsx_cast::cast::CastSender;
use rsx_messages::parse;
use rsx_messages::serialize;
use rsx_messages::CancelKey;
use rsx_messages::CancelRequest;
use rsx_messages::OrderRequest;
use rsx_messages::WsFrame;
use rsx_messages::RECORD_CANCEL_REQUEST;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_types::time_utils::time_ms;
use rsx_types::time_utils::time_ns;
use rsx_types::FailureReason;
use std::cell::RefCell;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::rc::Rc;
use tracing::info;
use tracing::warn;

/// An order refusal: the reason it is counted under, and the `E[code,
/// message]` the client gets.
type Rejection = (FailureReason, u32, &'static str);

const RATE_LIMITED: Rejection = (FailureReason::RateLimit, 1006, "rate limited");
const OVERLOADED: Rejection = (FailureReason::Overloaded, 5, "overloaded");
const NETWORK_ERROR: Rejection = (FailureReason::NetworkError, 8, "network error");

/// Serve one connection: HTTP request, then for a WebSocket upgrade a
/// session that ends when the client closes, a protocol violation occurs,
/// or the gateway removes the connection from `state` (idle reap, full
/// outbound queue), which drops whatever the session is awaiting and
/// closes the socket.
pub async fn handle_connection(
    mut stream: TcpStream,
    peer: SocketAddr,
    state: Rc<RefCell<GatewayState>>,
    cast_sender: Rc<RefCell<CastSender>>,
    jwt_secret: &str,
) {
    let (request, leftover) = match read_http_request(&mut stream).await {
        Ok(read) => read,
        Err(e) => {
            warn!("read request failed: {e}");
            return;
        }
    };

    if !is_ws_upgrade(&request) {
        handle_rest(&mut stream, &request, &state).await;
        return;
    }

    let user_id = match ws_handshake_from_request(&mut stream, &request, jwt_secret).await {
        Ok((_, user_id)) => user_id,
        Err(e) => {
            warn!("handshake failed: {e}");
            return;
        }
    };

    let conn_id = match state.borrow_mut().add_connection(user_id) {
        Ok(id) => id,
        Err(e) => {
            warn!("rejected connection: {e}");
            return;
        }
    };
    state.borrow_mut().touch_connection(conn_id, time_ns());
    state.borrow_mut().queue_heartbeat(conn_id, time_ms());
    info!("connection {} user {}", conn_id, user_id);

    let Some(egress) = state.borrow().connection_egress(conn_id) else {
        return;
    };
    monoio::select! {
        _ = serve_session(&mut stream, leftover, conn_id, user_id, peer, &state, &cast_sender) => {}
        _ = egress.closed() => {
            info!("conn {} closed by gateway", conn_id);
        }
    }
    state.borrow_mut().remove_connection(conn_id);
}

async fn serve_session(
    stream: &mut TcpStream,
    mut leftover: Vec<u8>,
    conn_id: u64,
    user_id: u32,
    peer: SocketAddr,
    state: &Rc<RefCell<GatewayState>>,
    cast_sender: &Rc<RefCell<CastSender>>,
) {
    let Some(egress) = state.borrow().connection_egress(conn_id) else {
        return;
    };

    loop {
        let msgs = state.borrow_mut().drain_outbound(conn_id);
        for msg in msgs {
            if let Err(e) = ws_write_text(stream, msg.as_bytes()).await {
                warn!("write error conn {}: {e}", conn_id);
                return;
            }
        }

        // Cancel-safe: the egress arm only cancels `readable`, which reads
        // nothing; the frame read below runs un-raced.
        monoio::select! {
            r = stream.readable(false) => {
                if let Err(e) = r {
                    info!("conn {} readable error: {e}", conn_id);
                    return;
                }
            }
            _ = egress.wait() => {
                continue;
            }
        }

        let (opcode, payload) = match ws_read_frame_buf(stream, &mut leftover).await {
            Ok(frame) => frame,
            Err(e) => {
                info!("conn {} closed: {e}", conn_id);
                return;
            }
        };

        state.borrow_mut().touch_connection(conn_id, time_ns());

        match opcode {
            8 => return,
            9 => {
                if let Err(e) = ws_write_frame(stream, 0xA, &payload).await {
                    warn!("ws write failed conn {}: {e}", conn_id);
                }
                continue;
            }
            0xA => continue,
            1 => {}
            _ => {
                info!("conn {} frame rejected opcode={}", conn_id, opcode);
                return;
            }
        }

        let Ok(text) = std::str::from_utf8(&payload) else {
            send_error(stream, 1001, "invalid utf8").await;
            continue;
        };

        let frame = match parse(text) {
            Ok(frame) => frame,
            Err(e) => {
                send_error(stream, 1002, &e.to_string()).await;
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
                post_only,
            } => {
                let cid = cid_bytes(&client_order_id);
                let admitted = admit_order(
                    &mut state.borrow_mut(),
                    peer.ip(),
                    user_id,
                    symbol_id,
                    price,
                    qty,
                    &cid,
                );
                match admitted {
                    Ok(Admission::New) => {}
                    Ok(Admission::Duplicate(order_id)) => {
                        answer_duplicate(stream, state, &order_id).await;
                        continue;
                    }
                    Err(rejection) => {
                        reject_order(stream, state, rejection).await;
                        continue;
                    }
                }

                let oid = generate_order_id();
                let (order_id_hi, order_id_lo) = split_order_id(&oid);
                let now_ns = time_ns();

                rsx_log::latency_sample!("gateway_in", order_id_hi, order_id_lo, now_ns);

                let order = OrderRequest {
                    seq: cast_sender.borrow().next_seq(),
                    user_id,
                    symbol_id,
                    price,
                    qty,
                    order_id_hi,
                    order_id_lo,
                    timestamp_ns: now_ns,
                    side,
                    tif,
                    reduce_only,
                    post_only,
                    is_liquidation: false,
                    _pad: [0; 3],
                };

                if !forward(cast_sender, RECORD_ORDER_REQUEST, as_bytes(&order)) {
                    state.borrow_mut().circuit.record_failure();
                    reject_order(stream, state, NETWORK_ERROR).await;
                    continue;
                }
                let mut st = state.borrow_mut();
                st.pending.push(PendingOrder {
                    order_id: oid,
                    user_id,
                    symbol_id,
                    client_order_id: cid,
                    timestamp_ns: now_ns,
                    stage: PendingStage::Submitted,
                });
                st.client_ids.insert(user_id, cid, oid, now_ns);
                st.circuit.record_success();
                st.metrics.orders_processed.add(1);
            }
            WsFrame::Cancel { key } => {
                let found = match key {
                    CancelKey::OrderId(hex) => match hex_to_order_id(&hex) {
                        Some(order_id) => state
                            .borrow()
                            .pending
                            .find_by_order_id(user_id, &order_id)
                            .map(|order| (order.symbol_id, order.order_id)),
                        None => {
                            send_error(stream, 1005, "invalid order id").await;
                            continue;
                        }
                    },
                    CancelKey::ClientOrderId(cid) => state
                        .borrow()
                        .pending
                        .find_by_client_order_id(user_id, &cid_bytes(&cid))
                        .map(|order| (order.symbol_id, order.order_id)),
                };
                let Some((symbol_id, order_id)) = found else {
                    send_error(stream, 1005, "order not found").await;
                    continue;
                };
                let (order_id_hi, order_id_lo) = split_order_id(&order_id);
                let cancel = CancelRequest {
                    seq: cast_sender.borrow().next_seq(),
                    ts_ns: time_ns(),
                    user_id,
                    symbol_id,
                    order_id_hi,
                    order_id_lo,
                    _pad: [0; 24],
                };
                if !forward(cast_sender, RECORD_CANCEL_REQUEST, as_bytes(&cancel)) {
                    send_error(stream, 8, "network error").await;
                }
            }
            WsFrame::Heartbeat { .. } => {
                let heartbeat = WsFrame::Heartbeat {
                    timestamp_ms: time_ms(),
                };
                send_frame(stream, &heartbeat).await;
            }
            _ => send_error(stream, 1004, "unsupported").await,
        }
    }
}

/// What `admit_order` decided for a frame that passed the checks.
enum Admission {
    /// Forward a new order.
    New,
    /// The client order id already has this order id: forward nothing.
    Duplicate([u8; 16]),
}

/// Run a new order through the gateway's checks in order: per-IP, per-user
/// and per-instance rate limits, a repeated client order id, symbol, tick and
/// lot, pending capacity, circuit breaker. A repeat is answered from the
/// client-id map, so it needs no capacity and takes no breaker slot. The
/// breaker slot is taken last: a half-open circuit admits one probe, and the
/// probe must be an order that is sent.
fn admit_order(
    state: &mut GatewayState,
    ip: IpAddr,
    user_id: u32,
    symbol_id: u32,
    price: i64,
    qty: i64,
    cid: &[u8; 20],
) -> Result<Admission, Rejection> {
    if !state.ip_limiter_for(ip).try_consume() {
        return Err(RATE_LIMITED);
    }
    let cap = state.rate_limit_per_user;
    let user_limiter = state
        .user_limiters
        .entry(user_id)
        .or_insert_with(|| RateLimiter::new(cap, cap));
    if !user_limiter.try_consume() {
        return Err(RATE_LIMITED);
    }
    if !state.instance_limiter.try_consume() {
        return Err(OVERLOADED);
    }
    if let Some(order_id) = state.client_ids.get(user_id, cid) {
        return Ok(Admission::Duplicate(order_id));
    }
    let Some(config) = state.symbol_configs.get(symbol_id as usize) else {
        return Err((FailureReason::SymbolNotFound, 1007, "unknown symbol"));
    };
    if !validate_tick_alignment(price, config.tick_size) {
        return Err((
            FailureReason::InvalidTickSize,
            1008,
            "price not tick aligned",
        ));
    }
    if !validate_lot_alignment(qty, config.lot_size) {
        return Err((FailureReason::InvalidLotSize, 1009, "qty not lot aligned"));
    }
    if state.pending.is_full() || !state.circuit.allow() {
        return Err(OVERLOADED);
    }
    Ok(Admission::New)
}

/// `cid` zero-padded to the wire's 20 bytes.
fn cid_bytes(cid: &str) -> [u8; 20] {
    let mut bytes = [0u8; 20];
    let src = cid.as_bytes();
    let len = src.len().min(bytes.len());
    bytes[..len].copy_from_slice(&src[..len]);
    bytes
}

/// Send a record to risk. The seq advances only after `send_raw` returns
/// Ok, so a failed send leaves no gap in the NAK ring or the peer's
/// expected seq. Returns false on a failed send.
fn forward(cast_sender: &Rc<RefCell<CastSender>>, record_type: u16, bytes: &[u8]) -> bool {
    let mut sender = cast_sender.borrow_mut();
    match sender.send_raw(record_type, bytes) {
        Ok(_) => {
            sender.advance_seq();
            true
        }
        Err(e) => {
            warn!("gateway: forward record {record_type} to risk failed: {e}");
            false
        }
    }
}

/// Count an order rejected for its reason and answer the client with
/// `E[code, message]`.
async fn reject_order(
    stream: &mut TcpStream,
    state: &Rc<RefCell<GatewayState>>,
    (reason, code, message): Rejection,
) {
    state.borrow().metrics.count_reject(reason);
    send_error(stream, code, message).await;
}

/// Answer a repeated client order id with the `U` the ME gives a duplicate
/// order id (status 3, reason DUPLICATE_ORDER_ID), naming the order already
/// minted. The pending entry and the first order's own updates are untouched.
async fn answer_duplicate(
    stream: &mut TcpStream,
    state: &Rc<RefCell<GatewayState>>,
    order_id: &[u8; 16],
) {
    state
        .borrow()
        .metrics
        .count_reject(FailureReason::DuplicateOrderId);
    let reason = FailureReason::DuplicateOrderId as u8;
    send_frame(stream, &update(order_id_to_hex(order_id), 3, 0, 0, reason)).await;
}

async fn send_error(stream: &mut TcpStream, code: u32, message: &str) {
    let error = WsFrame::Error {
        code,
        message: message.to_string(),
    };
    send_frame(stream, &error).await;
}

async fn send_frame(stream: &mut TcpStream, frame: &WsFrame) {
    if let Err(e) = ws_write_text(stream, serialize(frame).as_bytes()).await {
        warn!("ws write failed: {e}");
    }
}
