use crate::state::GatewayState;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpStream;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tracing::warn;

/// Split a request line into `(method, path)`.
/// Returns None if the request line is malformed.
fn parse_request_line(request: &str) -> Option<(&str, &str)> {
    let line = request.lines().next()?;
    let mut parts = line.splitn(3, ' ');
    let method = parts.next()?;
    let path = parts.next()?;
    Some((method, path))
}

fn http_response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\n\
Content-Type: application/json\r\n\
Content-Length: {}\r\n\
Connection: close\r\n\
\r\n\
{body}",
        body.len(),
    )
    .into_bytes()
}

/// An error response with the WebSocket `{"E":[code,msg]}` body.
fn http_error(code: u16, reason: &str, message: &str) -> Vec<u8> {
    let body = format!("{{\"E\":[{code},\"{message}\"]}}");
    http_response(&format!("{code} {reason}"), &body)
}

fn health_body(state: &Rc<RefCell<GatewayState>>) -> String {
    let uptime_sec = state.borrow().started_at.elapsed().as_secs();
    let ts_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!(
        "{{\"status\":\"ok\",\"version\":\"{}\",\"uptime_sec\":{uptime_sec},\"ts\":{ts_ns}}}",
        env!("CARGO_PKG_VERSION"),
    )
}

fn symbols_body(state: &Rc<RefCell<GatewayState>>) -> String {
    let symbols: Vec<String> = state
        .borrow()
        .symbol_configs
        .iter()
        .map(|cfg| {
            format!(
                concat!(
                    "{{\"id\":{},",
                    "\"tick_size\":{},",
                    "\"lot_size\":{},",
                    "\"price_decimals\":{},",
                    "\"qty_decimals\":{}}}"
                ),
                cfg.symbol_id, cfg.tick_size, cfg.lot_size, cfg.price_decimals, cfg.qty_decimals,
            )
        })
        .collect();
    format!("{{\"symbols\":[{}]}}", symbols.join(","))
}

/// Handle a REST HTTP request. Writes response and
/// returns. The connection should be closed after.
pub async fn handle_rest(stream: &mut TcpStream, request: &str, state: &Rc<RefCell<GatewayState>>) {
    let response = match parse_request_line(request) {
        Some(("GET", path)) => match path.split('?').next().unwrap_or(path) {
            "/health" => http_response("200 OK", &health_body(state)),
            "/v1/symbols" => http_response("200 OK", &symbols_body(state)),
            _ => http_error(404, "Not Found", "not found"),
        },
        _ => http_error(400, "Bad Request", "bad request"),
    };
    let (res, _) = stream.write_all(response).await;
    if let Err(e) = res {
        warn!("rest write failed: {e}");
    }
}
