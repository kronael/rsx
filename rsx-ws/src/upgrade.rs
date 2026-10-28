use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use monoio::io::AsyncReadRent;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpStream;
use sha1::Digest;
use sha1::Sha1;
use std::io;
use std::time::Duration;
use tracing::warn;

/// Total time a client has to deliver a complete HTTP upgrade request.
pub const WS_UPGRADE_DEADLINE: Duration = Duration::from_secs(5);

const WS_MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const HEADER_TOO_LARGE: &[u8] =
    b"HTTP/1.1 431 Request Header Fields Too Large\r\nConnection: close\r\n\r\n";
const MAX_REQUEST_LEN: usize = 8192;

/// Read the initial HTTP request: up to and including the blank line, plus
/// whatever arrived past it. A request head over 8 KiB is answered `431` and
/// refused; one not complete within `WS_UPGRADE_DEADLINE` is a `TimedOut`
/// error.
pub async fn read_http_request(stream: &mut TcpStream) -> io::Result<(String, Vec<u8>)> {
    monoio::time::timeout(WS_UPGRADE_DEADLINE, read_request_head(stream))
        .await
        .unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "upgrade request not received in time",
            ))
        })
}

async fn read_request_head(stream: &mut TcpStream) -> io::Result<(String, Vec<u8>)> {
    let mut data = Vec::new();
    let header_end = loop {
        let (res, buf) = stream.read(vec![0u8; 4096]).await;
        let n = res?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "connection closed during handshake",
            ));
        }
        data.extend_from_slice(&buf[..n]);
        let blank_line = data.windows(4).position(|w| w == b"\r\n\r\n");
        match blank_line {
            Some(pos) if pos + 4 <= MAX_REQUEST_LEN => break pos + 4,
            Some(_) => {}
            None if data.len() < MAX_REQUEST_LEN => continue,
            None => {}
        }
        let (res, _) = stream.write_all(HEADER_TOO_LARGE.to_vec()).await;
        if let Err(e) = res {
            warn!("ws: 431 response write failed: {e}");
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "request head over 8 KiB",
        ));
    };
    let request = String::from_utf8_lossy(&data[..header_end]).into_owned();
    let leftover = data[header_end..].to_vec();
    Ok((request, leftover))
}

/// True if the request carries a `Sec-WebSocket-Key`.
pub fn is_ws_upgrade(request: &str) -> bool {
    header_value(request, "sec-websocket-key").is_some()
}

/// Trimmed value of the first header called `name` (case-insensitive).
pub fn header_value<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

/// Write the `101 Switching Protocols` reply for the client's
/// `Sec-WebSocket-Key`.
pub async fn write_upgrade_response(stream: &mut TcpStream, key: &str) -> io::Result<()> {
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
Upgrade: websocket\r\n\
Connection: Upgrade\r\n\
Sec-WebSocket-Accept: {}\r\n\
\r\n",
        compute_accept_key(key),
    );
    let (res, _) = stream.write_all(response.into_bytes()).await;
    res?;
    Ok(())
}

/// Unauthenticated upgrade: read the request, answer 101, and return the
/// bytes that arrived past the request head.
pub async fn ws_upgrade(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let (request, leftover) = read_http_request(stream).await?;
    let key = header_value(&request, "sec-websocket-key")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Sec-WebSocket-Key"))?;
    write_upgrade_response(stream, key).await?;
    Ok(leftover)
}

fn compute_accept_key(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(WS_MAGIC.as_bytes());
    STANDARD.encode(hasher.finalize())
}

#[cfg(test)]
#[path = "upgrade_test.rs"]
mod upgrade_test;
