use crate::state::GatewayState;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpStream;
use std::cell::RefCell;
use std::rc::Rc;

/// Parse the request path from an HTTP request line.
/// Returns None if the request line is malformed.
fn parse_path(request: &str) -> Option<&str> {
    let line = request.lines().next()?;
    let mut parts = line.splitn(3, ' ');
    parts.next(); // method
    parts.next()  // path
}

fn http_200(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\n\
Content-Type: application/json\r\n\
Content-Length: {}\r\n\
Connection: close\r\n\
\r\n\
{}",
        body.len(),
        body,
    )
    .into_bytes()
}

fn http_404() -> Vec<u8> {
    let body = r#"{"error":"not found"}"#;
    format!(
        "HTTP/1.1 404 Not Found\r\n\
Content-Type: application/json\r\n\
Content-Length: {}\r\n\
Connection: close\r\n\
\r\n\
{}",
        body.len(),
        body,
    )
    .into_bytes()
}

async fn send_bytes(
    stream: &mut TcpStream,
    bytes: Vec<u8>,
) {
    let (_, _) = stream.write_all(bytes).await;
}

/// Handle a REST HTTP request. Writes response and
/// returns. The connection should be closed after.
pub async fn handle_rest(
    stream: &mut TcpStream,
    request: &str,
    state: &Rc<RefCell<GatewayState>>,
) {
    let path = match parse_path(request) {
        Some(p) => p,
        None => {
            send_bytes(stream, http_404()).await;
            return;
        }
    };
    // Strip query string
    let path = path.split('?').next().unwrap_or(path);

    match path {
        "/health" => {
            let body = r#"{"status":"ok"}"#;
            send_bytes(stream, http_200(body)).await;
        }
        "/v1/symbols" => {
            let configs = state
                .borrow()
                .symbol_configs
                .clone();
            let mut parts = Vec::new();
            for cfg in &configs {
                parts.push(format!(
                    concat!(
                        "{{\"id\":{},",
                        "\"tick_size\":{},",
                        "\"lot_size\":{},",
                        "\"price_decimals\":{},",
                        "\"qty_decimals\":{}}}"
                    ),
                    cfg.symbol_id,
                    cfg.tick_size,
                    cfg.lot_size,
                    cfg.price_decimals,
                    cfg.qty_decimals,
                ));
            }
            let body =
                format!("{{\"symbols\":[{}]}}", parts.join(","));
            send_bytes(stream, http_200(&body)).await;
        }
        _ => {
            send_bytes(stream, http_404()).await;
        }
    }
}
