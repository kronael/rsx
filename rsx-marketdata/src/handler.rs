use crate::protocol::parse_client_frame;
use crate::protocol::MdFrame;
use crate::protocol::MdParseError;
use crate::state::MarketDataState;
use crate::ws::ws_handshake;
use crate::ws::ws_read_frame;
use crate::ws::ws_write_raw;
use crate::ws::ws_write_text;
use monoio::net::TcpStream;
use std::cell::RefCell;
use std::rc::Rc;
use tracing::info;
use tracing::warn;

pub async fn handle_connection(
    mut stream: TcpStream,
    state: Rc<RefCell<MarketDataState>>,
    max_outbound: usize,
    snapshot_depth: u32,
) {
    if let Err(e) = ws_handshake(&mut stream).await {
        warn!("handshake failed: {e}");
        return;
    }

    let conn_id = state.borrow_mut().add_connection();
    info!("md connection {}", conn_id);

    loop {
        let msgs = state.borrow_mut().drain_outbound(conn_id);
        for msg in msgs {
            if let Err(e) = ws_write_text(&mut stream, msg.as_bytes()).await {
                warn!("write error conn {}: {e}", conn_id);
                state.borrow_mut().remove_connection(conn_id);
                return;
            }
        }

        let (opcode, payload) = match ws_read_frame(&mut stream).await {
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
            let mut pong = vec![0x8A, 0x00];
            if !payload.is_empty() {
                pong[1] = payload.len() as u8;
                pong.extend_from_slice(&payload);
            }
            let _ = ws_write_raw(&mut stream, &pong).await;
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
                let is_new = st.subscribe(
                    conn_id,
                    symbol_id,
                    channels,
                    snapshot_depth,
                );
                if is_new && (channels & 2) != 0 {
                    if let Some(snapshot) = st.snapshot_msg(
                        symbol_id,
                        snapshot_depth,
                    ) {
                        st.push_to_client(
                            conn_id,
                            snapshot,
                            max_outbound,
                        );
                    }
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
                let echo = format!("{{\"H\":[{}]}}", timestamp_ms);
                st.push_to_client(
                    conn_id,
                    echo,
                    max_outbound,
                );
            }
            Err(MdParseError::InvalidJson) => continue,
            Err(_) => continue,
        }
    }
}
