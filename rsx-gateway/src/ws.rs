use monoio::io::AsyncReadRent;
use monoio::io::AsyncReadRentExt;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpListener;
use monoio::net::TcpStream;
use sha1::Digest;
use sha1::Sha1;
use std::io;
use tracing::info;
use tracing::warn;

use crate::jwt::validate_jwt;

const WS_MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const UNAUTHORIZED: &[u8] = b"HTTP/1.1 401 Unauthorized\r\nConnection: close\r\n\r\n";
const MAX_FRAME_LEN: usize = 4096;

/// Accept WebSocket connections on the given address.
/// Calls `handler` for each accepted connection.
pub async fn ws_accept_loop<F>(addr: &str, handler: F) -> io::Result<()>
where
    F: Fn(TcpStream, std::net::SocketAddr) + 'static,
{
    let listener = TcpListener::bind(addr)?;
    info!("ws listening on {}", addr);
    loop {
        let (stream, peer) = listener.accept().await?;
        // Nagle plus delayed ACK adds ms-scale tail to the small order frames.
        if let Err(e) = stream.set_nodelay(true) {
            warn!("set_nodelay failed for {peer}: {e}");
        }
        info!("ws connection from {}", peer);
        handler(stream, peer);
    }
}

/// Read the initial HTTP request from a TcpStream.
pub async fn read_http_request(stream: &mut TcpStream) -> io::Result<(String, Vec<u8>)> {
    let buf = vec![0u8; 4096];
    let (res, buf) = stream.read(buf).await;
    let n = res?;
    if n == 0 {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionReset,
            "connection closed during handshake",
        ));
    }
    let data = &buf[..n];
    let header_end = data
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(n);
    let request = String::from_utf8_lossy(&data[..header_end]).into_owned();
    let leftover = data[header_end..].to_vec();
    Ok((request, leftover))
}

/// Returns true if the request is a WS upgrade.
pub fn is_ws_upgrade(request: &str) -> bool {
    header_value(request, "sec-websocket-key").is_some()
}

/// Perform the WS handshake on an already-read request. Returns
/// `(Sec-WebSocket-Key, user_id)` once the 101 is written; a missing or
/// invalid JWT is answered with a 401 and an error.
pub async fn ws_handshake_from_request(
    stream: &mut TcpStream,
    request: &str,
    jwt_secret: &str,
) -> io::Result<(String, u32)> {
    let key = header_value(request, "sec-websocket-key")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Sec-WebSocket-Key"))?
        .to_string();

    let user_id = match extract_user(request, jwt_secret) {
        Ok(user_id) => user_id,
        Err(reason) => {
            let (res, _) = stream.write_all(UNAUTHORIZED.to_vec()).await;
            if let Err(e) = res {
                warn!("gateway: 401 response write failed: {e}");
            }
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, reason));
        }
    };

    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
Upgrade: websocket\r\n\
Connection: Upgrade\r\n\
Sec-WebSocket-Accept: {}\r\n\
\r\n",
        compute_accept_key(&key),
    );
    let (res, _) = stream.write_all(response.into_bytes()).await;
    res?;

    Ok((key, user_id))
}

/// Perform the WS handshake on a raw TcpStream. Returns
/// `(Sec-WebSocket-Key, user_id, bytes read past the request)`.
pub async fn ws_handshake(
    stream: &mut TcpStream,
    jwt_secret: &str,
) -> io::Result<(String, u32, Vec<u8>)> {
    let (request, leftover) = read_http_request(stream).await?;
    let (key, uid) = ws_handshake_from_request(stream, &request, jwt_secret).await?;
    Ok((key, uid, leftover))
}

/// Validate the Bearer JWT and return its user id.
fn extract_user(request: &str, jwt_secret: &str) -> Result<u32, &'static str> {
    let value = header_value(request, "authorization").ok_or("missing authorization header")?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .ok_or("bearer scheme missing")?;
    validate_jwt(token.trim(), jwt_secret).map_err(|_| "jwt invalid")
}

/// Trimmed value of the first header called `name` (case-insensitive).
fn header_value<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn compute_accept_key(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(WS_MAGIC.as_bytes());
    let result = hasher.finalize();
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(result)
}

fn big_endian_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .fold(0, |len, byte| (len << 8) | usize::from(*byte))
}

/// Read one client frame, consuming from `buf` before the stream. Returns
/// `(opcode, unmasked payload)`. Dropping the future mid-frame loses the
/// bytes already taken, so the caller awaits readability first and then
/// runs this to the end. Fragmented, unmasked and over-long frames are
/// errors.
pub async fn ws_read_frame_buf(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
) -> io::Result<(u8, Vec<u8>)> {
    macro_rules! read_n {
        ($n:expr) => {{
            let n = $n;
            let mut out = Vec::with_capacity(n);
            let take = n.min(buf.len());
            out.extend_from_slice(&buf[..take]);
            buf.drain(..take);
            if out.len() < n {
                let tmp = vec![0u8; n - out.len()];
                let (res, tmp) = stream.read_exact(tmp).await;
                res?;
                out.extend_from_slice(&tmp);
            }
            out
        }};
    }

    let head = read_n!(2);
    if head[0] & 0x80 == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "fragmented frame (FIN=0) not supported",
        ));
    }
    if head[1] & 0x80 == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unmasked client frame rejected",
        ));
    }
    let opcode = head[0] & 0x0F;
    let len1 = usize::from(head[1] & 0x7F);

    let payload_len = match len1 {
        126 => big_endian_len(&read_n!(2)),
        127 => big_endian_len(&read_n!(8)),
        _ => len1,
    };
    if payload_len > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds 4096 bytes",
        ));
    }

    let mask = read_n!(4);
    let mut payload = read_n!(payload_len);
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[i % 4];
    }

    Ok((opcode, payload))
}

/// Write a WebSocket text frame.
pub async fn ws_write_text(stream: &mut TcpStream, data: &[u8]) -> io::Result<()> {
    ws_write_frame(stream, 0x1, data).await
}

/// Write a WebSocket frame with provided opcode.
pub async fn ws_write_frame(stream: &mut TcpStream, opcode: u8, data: &[u8]) -> io::Result<()> {
    let mut frame = Vec::with_capacity(10 + data.len());
    frame.push(0x80 | (opcode & 0x0F)); // FIN + opcode

    if data.len() <= 125 {
        frame.push(data.len() as u8);
    } else if data.len() <= 65535 {
        frame.push(126);
        frame.extend_from_slice(&(data.len() as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(data.len() as u64).to_be_bytes());
    }
    frame.extend_from_slice(data);

    let (res, _) = stream.write_all(frame).await;
    res?;
    Ok(())
}
