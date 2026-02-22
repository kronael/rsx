use monoio::io::AsyncReadRent;
use monoio::io::AsyncReadRentExt;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpListener;
use monoio::net::TcpStream;
use sha1::Digest;
use sha1::Sha1;
use std::io;
use tracing::info;

const WS_MAGIC: &str =
    "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Accept WebSocket connections on the given address.
/// Calls `handler` for each accepted connection.
pub async fn ws_accept_loop<F>(
    addr: &str,
    handler: F,
) -> io::Result<()>
where
    F: Fn(TcpStream) + 'static,
{
    let listener = TcpListener::bind(addr)?;
    info!("ws listening on {}", addr);
    loop {
        let (stream, peer) = listener.accept().await?;
        info!("ws connection from {}", peer);
        handler(stream);
    }
}

/// Perform WebSocket upgrade handshake on a raw
/// TcpStream. Returns Ok(key) if upgrade succeeded.
pub async fn ws_handshake(
    stream: &mut TcpStream,
) -> io::Result<String> {
    let buf = vec![0u8; 4096];
    let (res, buf) = stream.read(buf).await;
    let n = res?;
    if n == 0 {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionReset,
            "connection closed during handshake",
        ));
    }

    let request = String::from_utf8_lossy(&buf[..n]);

    let key = extract_ws_key(&request).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "missing Sec-WebSocket-Key",
        )
    })?;

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
            let val = line
                .split_once(':')
                .map(|(_, v)| v.trim().to_string());
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
    base64::engine::general_purpose::STANDARD
        .encode(result)
}

/// Read a single WebSocket frame from the stream.
/// Returns (opcode, payload).
/// Only handles frames up to 64KB.
pub async fn ws_read_frame(
    stream: &mut TcpStream,
) -> io::Result<(u8, Vec<u8>)> {
    let preamble = vec![0u8; 2];
    let (res, preamble) = stream.read_exact(preamble).await;
    res?;

    let opcode = preamble[0] & 0x0F;
    let masked = (preamble[1] & 0x80) != 0;
    let len1 = (preamble[1] & 0x7F) as usize;

    let payload_len = if len1 <= 125 {
        len1
    } else if len1 == 126 {
        let ext = vec![0u8; 2];
        let (res, ext) = stream.read_exact(ext).await;
        res?;
        ((ext[0] as usize) << 8) | (ext[1] as usize)
    } else {
        let ext = vec![0u8; 8];
        let (res, ext) = stream.read_exact(ext).await;
        res?;
        // SAFETY: ext is exactly 8 bytes from read_exact
        usize::from_be_bytes(
            ext[..8].try_into().unwrap(),
        )
    };

    const MAX_PAYLOAD: usize = 1_048_576; // 1MB
    if payload_len > MAX_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds max payload size",
        ));
    }

    let mask_key = if masked {
        let mk = vec![0u8; 4];
        let (res, mk) = stream.read_exact(mk).await;
        res?;
        Some([mk[0], mk[1], mk[2], mk[3]])
    } else {
        None
    };

    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        let (res, p) =
            stream.read_exact(payload).await;
        res?;
        payload = p;
    }

    if let Some(mask) = mask_key {
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[i % 4];
        }
    }

    Ok((opcode, payload))
}

/// Write a WebSocket text frame.
pub async fn ws_write_text(
    stream: &mut TcpStream,
    data: &[u8],
) -> io::Result<()> {
    let mut frame =
        Vec::with_capacity(10 + data.len());
    frame.push(0x81); // FIN + text opcode

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
pub async fn ws_write_raw(
    stream: &mut TcpStream,
    data: &[u8],
) -> io::Result<()> {
    let (res, _) = stream.write_all(data.to_vec()).await;
    res?;
    Ok(())
}
