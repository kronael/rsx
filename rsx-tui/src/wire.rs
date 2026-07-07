//! webproto-49-over-QUIC frame codec.
//!
//! Length-delimited frames: a 4-byte big-endian length prefix followed
//! by a JSON body. Client→server frames carry an `OrderReq`; server→
//! client frames carry a `GwEvent`. This is the webproto-49 message
//! shape (order in, events out) carried over a QUIC bidirectional
//! stream instead of a WebSocket.
//!
//! Byte-level interop with the live gateway's webproto framing is a
//! follow-up: the gateway currently speaks webproto over WebSocket, and
//! the gateway QUIC listener is out of scope here. This module fixes the
//! client↔server shape so the TUI can be built and tested against a
//! loopback QUIC server today; swapping the body encoding for the
//! gateway's exact wire bytes is a localized change here.

use crate::conn::GwEvent;
use crate::conn::OrderReq;
use quinn::RecvStream;
use quinn::SendStream;
use std::io;

/// Reject frames larger than this (guards a corrupt/hostile length
/// prefix from triggering a huge allocation). 1 MiB is far above any
/// legitimate order or event frame.
const MAX_FRAME: usize = 1 << 20;

/// Map any error carrying a message into an `io::Error`.
fn to_io<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(e.to_string())
}

async fn write_frame(send: &mut SendStream, body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?;
    send.write_all(&len.to_be_bytes()).await.map_err(to_io)?;
    send.write_all(body).await.map_err(to_io)?;
    Ok(())
}

async fn read_frame(recv: &mut RecvStream) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await.map_err(to_io)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame length exceeds MAX_FRAME",
        ));
    }
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body).await.map_err(to_io)?;
    Ok(body)
}

/// Client→server: encode and send one order frame.
pub async fn write_order(send: &mut SendStream, order: &OrderReq) -> io::Result<()> {
    let body = serde_json::to_vec(order).map_err(to_io)?;
    write_frame(send, &body).await
}

/// Server→client on the read side (or a test server): decode one order
/// frame.
pub async fn read_order(recv: &mut RecvStream) -> io::Result<OrderReq> {
    let body = read_frame(recv).await?;
    serde_json::from_slice(&body).map_err(to_io)
}

/// Server→client: encode and send one event frame.
pub async fn write_event(send: &mut SendStream, ev: &GwEvent) -> io::Result<()> {
    let body = serde_json::to_vec(ev).map_err(to_io)?;
    write_frame(send, &body).await
}

/// Client side: decode one event frame into a `GwEvent`.
pub async fn read_event(recv: &mut RecvStream) -> io::Result<GwEvent> {
    let body = read_frame(recv).await?;
    serde_json::from_slice(&body).map_err(to_io)
}
