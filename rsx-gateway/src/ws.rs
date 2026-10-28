use crate::jwt::validate_jwt;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpStream;
use rsx_ws::header_value;
use rsx_ws::read_http_request;
use rsx_ws::write_upgrade_response;
use std::io;
use tracing::warn;

const UNAUTHORIZED: &[u8] = b"HTTP/1.1 401 Unauthorized\r\nConnection: close\r\n\r\n";

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

    write_upgrade_response(stream, &key).await?;
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
