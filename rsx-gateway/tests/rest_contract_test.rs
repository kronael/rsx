/// Contract tests for REST/WS proxy handlers:
/// path rewriting, response headers, WS upgrade
/// detection. Tests run against local in-process
/// listeners (no external deps).
use monoio::io::AsyncReadRent;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpStream;
use rsx_gateway::rest::handle_rest;
use rsx_gateway::state::GatewayState;
use rsx_gateway::ws::is_ws_upgrade;
use rsx_types::SymbolConfig;
use std::cell::RefCell;
use std::rc::Rc;

// ── helpers ───────────────────────────────────────

fn empty_state() -> Rc<RefCell<GatewayState>> {
    Rc::new(RefCell::new(GatewayState::new(
        64, 10, 1000, vec![],
    )))
}

fn state_with_symbols() -> Rc<RefCell<GatewayState>> {
    let syms = vec![
        SymbolConfig {
            symbol_id: 0,
            tick_size: 10,
            lot_size: 1,
            price_decimals: 2,
            qty_decimals: 0,
        },
        SymbolConfig {
            symbol_id: 1,
            tick_size: 100,
            lot_size: 10,
            price_decimals: 3,
            qty_decimals: 1,
        },
    ];
    Rc::new(RefCell::new(GatewayState::new(
        64, 10, 1000, syms,
    )))
}

/// Send a raw request to handle_rest via loopback.
/// Returns the full response string.
async fn round_trip(
    request: &str,
    state: Rc<RefCell<GatewayState>>,
) -> String {
    let listener =
        monoio::net::TcpListener::bind("127.0.0.1:0")
            .unwrap();
    let addr = listener.local_addr().unwrap();
    let req = request.to_string();

    monoio::spawn(async move {
        let (mut stream, _) =
            listener.accept().await.unwrap();
        handle_rest(&mut stream, &req, &state).await;
    });

    monoio::time::sleep(
        std::time::Duration::from_millis(5),
    )
    .await;

    let mut client =
        TcpStream::connect(addr).await.unwrap();
    monoio::time::sleep(
        std::time::Duration::from_millis(20),
    )
    .await;

    let buf = vec![0u8; 4096];
    let (res, buf): (_, Vec<u8>) =
        client.read(buf).await;
    let n = res.unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

// ── is_ws_upgrade ─────────────────────────────────

#[test]
fn ws_upgrade_detected_with_key_header() {
    let req = "GET /ws HTTP/1.1\r\n\
        Host: localhost\r\n\
        Upgrade: websocket\r\n\
        Sec-WebSocket-Key: abc123==\r\n\
        \r\n";
    assert!(is_ws_upgrade(req));
}

#[test]
fn ws_upgrade_detected_case_insensitive() {
    let req = "GET /ws HTTP/1.1\r\n\
        Host: localhost\r\n\
        SEC-WEBSOCKET-KEY: abc123==\r\n\
        \r\n";
    assert!(is_ws_upgrade(req));
}

#[test]
fn ws_upgrade_not_detected_for_plain_http() {
    let req = "GET /health HTTP/1.1\r\n\
        Host: localhost\r\n\
        \r\n";
    assert!(!is_ws_upgrade(req));
}

#[test]
fn ws_upgrade_not_detected_missing_key() {
    let req = "GET /ws HTTP/1.1\r\n\
        Host: localhost\r\n\
        Upgrade: websocket\r\n\
        \r\n";
    assert!(!is_ws_upgrade(req));
}

// ── /health route ─────────────────────────────────

#[test]
fn health_returns_200_ok() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = empty_state();
        let req = "GET /health HTTP/1.1\r\n\
            Host: localhost\r\n\
            \r\n";
        let resp = round_trip(req, state).await;
        assert!(
            resp.starts_with("HTTP/1.1 200 OK"),
            "expected 200, got: {resp}"
        );
    });
}

#[test]
fn health_body_is_status_ok() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = empty_state();
        let req = "GET /health HTTP/1.1\r\n\
            Host: localhost\r\n\
            \r\n";
        let resp = round_trip(req, state).await;
        assert!(
            resp.contains(r#"{"status":"ok"}"#),
            "body missing status ok: {resp}"
        );
    });
}

#[test]
fn health_has_json_content_type() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = empty_state();
        let req = "GET /health HTTP/1.1\r\n\
            Host: localhost\r\n\
            \r\n";
        let resp = round_trip(req, state).await;
        assert!(
            resp.contains(
                "Content-Type: application/json"
            ),
            "missing json content-type: {resp}"
        );
    });
}

#[test]
fn health_has_content_length_header() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = empty_state();
        let req = "GET /health HTTP/1.1\r\n\
            Host: localhost\r\n\
            \r\n";
        let resp = round_trip(req, state).await;
        assert!(
            resp.contains("Content-Length:"),
            "missing content-length: {resp}"
        );
    });
}

#[test]
fn health_has_connection_close() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = empty_state();
        let req = "GET /health HTTP/1.1\r\n\
            Host: localhost\r\n\
            \r\n";
        let resp = round_trip(req, state).await;
        assert!(
            resp.contains("Connection: close"),
            "missing connection close: {resp}"
        );
    });
}

// ── path rewriting: query string stripped ─────────

#[test]
fn health_query_string_stripped() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = empty_state();
        let req = "GET /health?foo=bar HTTP/1.1\r\n\
            Host: localhost\r\n\
            \r\n";
        let resp = round_trip(req, state).await;
        assert!(
            resp.starts_with("HTTP/1.1 200 OK"),
            "query string broke routing: {resp}"
        );
    });
}

// ── /v1/symbols route ─────────────────────────────

#[test]
fn symbols_returns_200() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = state_with_symbols();
        let req = "GET /v1/symbols HTTP/1.1\r\n\
            Host: localhost\r\n\
            \r\n";
        let resp = round_trip(req, state).await;
        assert!(
            resp.starts_with("HTTP/1.1 200 OK"),
            "expected 200, got: {resp}"
        );
    });
}

#[test]
fn symbols_body_contains_symbols_key() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = state_with_symbols();
        let req = "GET /v1/symbols HTTP/1.1\r\n\
            Host: localhost\r\n\
            \r\n";
        let resp = round_trip(req, state).await;
        assert!(
            resp.contains("\"symbols\""),
            "missing symbols key: {resp}"
        );
    });
}

#[test]
fn symbols_body_has_correct_count() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = state_with_symbols();
        let req = "GET /v1/symbols HTTP/1.1\r\n\
            Host: localhost\r\n\
            \r\n";
        let resp = round_trip(req, state).await;
        // Two symbol entries: count "\"id\":" occurrences
        let count =
            resp.matches("\"id\":").count();
        assert_eq!(
            count, 2,
            "expected 2 symbols in body: {resp}"
        );
    });
}

#[test]
fn symbols_body_contains_tick_and_lot() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = state_with_symbols();
        let req = "GET /v1/symbols HTTP/1.1\r\n\
            Host: localhost\r\n\
            \r\n";
        let resp = round_trip(req, state).await;
        assert!(
            resp.contains("\"tick_size\""),
            "missing tick_size: {resp}"
        );
        assert!(
            resp.contains("\"lot_size\""),
            "missing lot_size: {resp}"
        );
    });
}

#[test]
fn symbols_empty_state_returns_empty_array() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = empty_state();
        let req = "GET /v1/symbols HTTP/1.1\r\n\
            Host: localhost\r\n\
            \r\n";
        let resp = round_trip(req, state).await;
        assert!(
            resp.contains("\"symbols\":[]"),
            "expected empty array: {resp}"
        );
    });
}

#[test]
fn symbols_query_string_stripped() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = state_with_symbols();
        let req =
            "GET /v1/symbols?active=true HTTP/1.1\r\n\
            Host: localhost\r\n\
            \r\n";
        let resp = round_trip(req, state).await;
        assert!(
            resp.starts_with("HTTP/1.1 200 OK"),
            "query string broke symbols route: {resp}"
        );
    });
}

// ── unknown path → 404 ────────────────────────────

#[test]
fn unknown_path_returns_404() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = empty_state();
        let req = "GET /nonexistent HTTP/1.1\r\n\
            Host: localhost\r\n\
            \r\n";
        let resp = round_trip(req, state).await;
        assert!(
            resp.starts_with("HTTP/1.1 404 Not Found"),
            "expected 404, got: {resp}"
        );
    });
}

#[test]
fn unknown_path_404_body_is_json() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = empty_state();
        let req = "GET /v2/orders HTTP/1.1\r\n\
            Host: localhost\r\n\
            \r\n";
        let resp = round_trip(req, state).await;
        assert!(
            resp.contains(
                "Content-Type: application/json"
            ),
            "404 missing json content-type: {resp}"
        );
        assert!(
            resp.contains("\"error\""),
            "404 body missing error key: {resp}"
        );
    });
}

#[test]
fn malformed_request_returns_404() {
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let state = empty_state();
        // No space-separated path in request line.
        let req = "BADREQUEST\r\n\r\n";
        let resp = round_trip(req, state).await;
        assert!(
            resp.starts_with("HTTP/1.1 404 Not Found"),
            "expected 404 for malformed: {resp}"
        );
    });
}

// ── WS upgrade response headers ───────────────────

#[test]
fn ws_handshake_responds_101() {
    use rsx_gateway::ws::ws_handshake;
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let listener =
            monoio::net::TcpListener::bind("127.0.0.1:0")
                .unwrap();
        let addr = listener.local_addr().unwrap();

        monoio::spawn(async move {
            let (mut stream, _) =
                listener.accept().await.unwrap();
            // empty secret = allow X-User-Id fallback
            let _ = ws_handshake(&mut stream, "").await;
        });

        monoio::time::sleep(
            std::time::Duration::from_millis(5),
        )
        .await;

        let mut client =
            TcpStream::connect(addr).await.unwrap();

        let req =
            "GET /ws/private HTTP/1.1\r\n\
            Host: localhost\r\n\
            Upgrade: websocket\r\n\
            Connection: Upgrade\r\n\
            Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
            X-User-Id: 1\r\n\
            Sec-WebSocket-Version: 13\r\n\
            \r\n";
        let (res, _) = client
            .write_all(req.as_bytes().to_vec())
            .await;
        res.unwrap();

        monoio::time::sleep(
            std::time::Duration::from_millis(20),
        )
        .await;

        let buf = vec![0u8; 4096];
        let (res, buf): (_, Vec<u8>) =
            client.read(buf).await;
        let n = res.unwrap_or(0);
        let resp =
            String::from_utf8_lossy(&buf[..n])
                .into_owned();

        assert!(
            resp.starts_with(
                "HTTP/1.1 101 Switching Protocols"
            ),
            "expected 101, got: {resp}"
        );
        assert!(
            resp.contains("Upgrade: websocket"),
            "missing upgrade header: {resp}"
        );
        assert!(
            resp.contains("Sec-WebSocket-Accept:"),
            "missing accept header: {resp}"
        );
    });
}

#[test]
fn ws_handshake_401_without_auth() {
    use rsx_gateway::ws::ws_handshake;
    let mut rt = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();
    rt.block_on(async {
        let listener =
            monoio::net::TcpListener::bind("127.0.0.1:0")
                .unwrap();
        let addr = listener.local_addr().unwrap();

        monoio::spawn(async move {
            let (mut stream, _) =
                listener.accept().await.unwrap();
            // Non-empty secret = JWT required,
            // X-User-Id fallback disabled.
            let _ =
                ws_handshake(&mut stream, "secret").await;
        });

        monoio::time::sleep(
            std::time::Duration::from_millis(5),
        )
        .await;

        let mut client =
            TcpStream::connect(addr).await.unwrap();

        // No Authorization or X-User-Id header.
        let req =
            "GET /ws/private HTTP/1.1\r\n\
            Host: localhost\r\n\
            Upgrade: websocket\r\n\
            Connection: Upgrade\r\n\
            Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
            Sec-WebSocket-Version: 13\r\n\
            \r\n";
        let (res, _) = client
            .write_all(req.as_bytes().to_vec())
            .await;
        res.unwrap();

        monoio::time::sleep(
            std::time::Duration::from_millis(20),
        )
        .await;

        let buf = vec![0u8; 4096];
        let (res, buf): (_, Vec<u8>) =
            client.read(buf).await;
        let n = res.unwrap_or(0);
        let resp =
            String::from_utf8_lossy(&buf[..n])
                .into_owned();

        assert!(
            resp.starts_with("HTTP/1.1 401"),
            "expected 401, got: {resp}"
        );
    });
}
