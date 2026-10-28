use crate::records::parse_client_frame;
use crate::records::MdFrame;
use crate::records::MdParseError;
use crate::state::MarketDataState;
use monoio::net::TcpStream;
use rsx_ws::is_frame_buffered;
use rsx_ws::ws_read_frame_buf;
use rsx_ws::ws_upgrade;
use rsx_ws::ws_write_binary;
use rsx_ws::ws_write_frame;
use std::cell::RefCell;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use tracing::info;
use tracing::warn;

pub async fn handle_connection(
    mut stream: TcpStream,
    peer: SocketAddr,
    state: Rc<RefCell<MarketDataState>>,
    max_outbound: usize,
    snapshot_depth: u32,
) {
    let mut leftover = match ws_upgrade(&mut stream).await {
        Ok(leftover) => leftover,
        Err(e) => {
            warn!("handshake failed for {peer}: {e}");
            return;
        }
    };

    let conn_id = state.borrow_mut().add_connection();
    info!("md connection {} from {}", conn_id, peer);

    let egress = state
        .borrow()
        .connection_egress(conn_id)
        .expect("egress waker present for just-added connection");

    loop {
        let msgs = state.borrow_mut().drain_outbound(conn_id);
        for msg in msgs {
            if let Err(e) = ws_write_binary(&mut stream, &msg).await {
                warn!("write error conn {}: {e}", conn_id);
                state.borrow_mut().remove_connection(conn_id);
                return;
            }
        }

        // Cancel-safe: the egress arm only cancels `readable`, which reads
        // nothing; the frame read below runs un-raced.
        if !is_frame_buffered(&leftover) {
            monoio::select! {
                r = stream.readable(false) => {
                    if let Err(e) = r {
                        info!("conn {} closed: {e}", conn_id);
                        state.borrow_mut().remove_connection(conn_id);
                        return;
                    }
                }
                _ = egress.wait() => {
                    continue;
                }
            }
        }

        let (opcode, payload) = match ws_read_frame_buf(&mut stream, &mut leftover).await {
            Ok(f) => f,
            Err(e) => {
                info!("conn {} closed: {e}", conn_id);
                state.borrow_mut().remove_connection(conn_id);
                return;
            }
        };

        if opcode == 8 {
            state.borrow_mut().remove_connection(conn_id);
            return;
        }

        if opcode == 9 {
            let pong_payload = if payload.len() > 125 {
                &payload[..125]
            } else {
                &payload
            };
            if let Err(e) = ws_write_frame(&mut stream, 0xA, pong_payload).await {
                warn!("md: pong write failed conn {}: {e}", conn_id);
            }
            continue;
        }

        if opcode != 1 {
            continue;
        }

        let text = match std::str::from_utf8(&payload) {
            Ok(s) => s,
            Err(_) => continue,
        };

        match parse_client_frame(text) {
            Ok(MdFrame::Subscribe {
                symbol_id,
                channels,
            }) => {
                let mut st = state.borrow_mut();
                st.subscribe(conn_id, symbol_id, channels, snapshot_depth);
                if (channels & 2) != 0 {
                    // No book is created here: the first ORDER_INSERTED
                    // anchors it at a real price.
                    st.send_snapshot_to_client(conn_id, symbol_id, snapshot_depth, max_outbound);
                }
            }
            Ok(MdFrame::Unsubscribe {
                symbol_id,
                channels: _,
            }) => {
                if symbol_id == 0 {
                    state.borrow_mut().unsubscribe_all(conn_id);
                } else {
                    state.borrow_mut().unsubscribe(conn_id, symbol_id);
                }
            }
            Ok(MdFrame::Heartbeat { timestamp_ms }) => {
                let mut st = state.borrow_mut();
                st.update_heartbeat(conn_id);
                let echo: Arc<[u8]> = crate::wire::encode_heartbeat(timestamp_ms).into();
                st.push_to_client(conn_id, echo, max_outbound);
            }
            Err(MdParseError::InvalidJson) => continue,
            Err(_) => continue,
        }
    }
}
