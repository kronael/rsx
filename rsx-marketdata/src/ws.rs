use monoio::io::AsyncReadRent;
use monoio::io::AsyncReadRentExt;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpListener;
use monoio::net::TcpStream;
use rsx_types::time_utils::WS_UPGRADE_DEADLINE;
use sha1::Digest;
use sha1::Sha1;
use std::io;
use tracing::info;
use tracing::warn;

const WS_MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const MAX_FRAME_LEN: usize = 4096;

/// Accept WebSocket connections on the given address.
/// Calls `handler` for each accepted connection.
pub async fn ws_accept_loop<F>(addr: &str, handler: F) -> io::Result<()>
where
    F: Fn(TcpStream) + 'static,
{
    let listener = TcpListener::bind(addr)?;
    info!("ws listening on {}", addr);
    loop {
        let (stream, peer) = listener.accept().await?;
        // Disable Nagle: coalescing + delayed-ACK adds ms-scale tail
        // latency to the small binary frames fanned out to subscribers.
        if let Err(e) = stream.set_nodelay(true) {
            warn!("set_nodelay failed for {peer}: {e}");
        }
        info!("ws connection from {}", peer);
        handler(stream);
    }
}

/// Perform WebSocket upgrade handshake on a raw TcpStream. Returns Ok(key) if
/// upgrade succeeded; a request not received within `WS_UPGRADE_DEADLINE` is a
/// `TimedOut` error.
pub async fn ws_handshake(stream: &mut TcpStream) -> io::Result<String> {
    let buf = vec![0u8; 4096];
    let (res, buf) = monoio::time::timeout(WS_UPGRADE_DEADLINE, stream.read(buf))
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "upgrade request not received in time",
            )
        })?;
    let n = res?;
    if n == 0 {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionReset,
            "connection closed during handshake",
        ));
    }

    let request = String::from_utf8_lossy(&buf[..n]);

    let key = extract_ws_key(&request)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Sec-WebSocket-Key"))?;

    let accept = compute_accept_key(&key);

    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
Upgrade: websocket\r\n\
Connection: Upgrade\r\n\
Sec-WebSocket-Accept: {}\r\n\r\n",
        accept,
    );
    let resp_bytes = response.into_bytes();
    let (res, _) = stream.write_all(resp_bytes).await;
    res?;

    Ok(key)
}

fn extract_ws_key(request: &str) -> Option<String> {
    for line in request.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("sec-websocket-key:") {
            let val = line.split_once(':').map(|(_, v)| v.trim().to_string());
            return val;
        }
    }
    None
}

fn compute_accept_key(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(WS_MAGIC.as_bytes());
    let result = hasher.finalize();
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(result)
}

/// Read one client frame. Returns `(opcode, unmasked payload)`. Fragmented,
/// unmasked and over-long frames are `InvalidData` errors.
pub async fn ws_read_frame(stream: &mut TcpStream) -> io::Result<(u8, Vec<u8>)> {
    let head = read_bytes(stream, 2).await?;
    if head[0] & 0x80 == 0 {
        return Err(invalid_frame("fragmented frame (FIN=0) not supported"));
    }
    if head[1] & 0x80 == 0 {
        return Err(invalid_frame("unmasked client frame rejected"));
    }
    let opcode = head[0] & 0x0F;
    let payload_len = match head[1] & 0x7F {
        127 => return Err(invalid_frame("frame exceeds 4096 bytes")),
        126 => {
            let ext = read_bytes(stream, 2).await?;
            usize::from(u16::from_be_bytes([ext[0], ext[1]]))
        }
        len => usize::from(len),
    };
    if payload_len > MAX_FRAME_LEN {
        return Err(invalid_frame("frame exceeds 4096 bytes"));
    }

    let mut mask = read_bytes(stream, 4 + payload_len).await?;
    let mut payload = mask.split_off(4);
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[i % 4];
    }

    Ok((opcode, payload))
}

async fn read_bytes(stream: &mut TcpStream, len: usize) -> io::Result<Vec<u8>> {
    let (res, bytes) = stream.read_exact(vec![0u8; len]).await;
    res?;
    Ok(bytes)
}

fn invalid_frame(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

/// Write a WebSocket BINARY frame. The public feed is protobuf, so the
/// fanned-out payloads are binary `MdFrame` bytes (not UTF-8 text).
pub async fn ws_write_binary(stream: &mut TcpStream, data: &[u8]) -> io::Result<()> {
    let mut frame = Vec::with_capacity(10 + data.len());
    frame.push(0x82);

    if data.len() <= 125 {
        frame.push(data.len() as u8);
    } else if data.len() <= 65535 {
        frame.push(126);
        frame.push((data.len() >> 8) as u8);
        frame.push((data.len() & 0xFF) as u8);
    } else {
        frame.push(127);
        let len = data.len() as u64;
        frame.extend_from_slice(&len.to_be_bytes());
    }
    frame.extend_from_slice(data);

    let (res, _) = stream.write_all(frame).await;
    res?;
    Ok(())
}

/// Write a raw WebSocket frame payload (already framed).
pub async fn ws_write_raw(stream: &mut TcpStream, data: &[u8]) -> io::Result<()> {
    let (res, _) = stream.write_all(data.to_vec()).await;
    res?;
    Ok(())
}

#[cfg(test)]
#[path = "ws_test.rs"]
mod ws_test;
