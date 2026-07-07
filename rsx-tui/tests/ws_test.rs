//! Loopback proof for the WS client transport (webproto-49).
//!
//! Stands up a `tokio-tungstenite` server on `127.0.0.1:0`, connects a
//! real `WsConn` to it, submits one order, asserts the server
//! received a well-formed `{N:[...]}` text frame, then has the server
//! reply with a `U` (accept) frame followed by an `F` (fill) frame —
//! and asserts the client's `poll_event` folds them into `Accepted`
//! then `Fill`. Mirrors the shape of `tests/quic_test.rs`; no
//! cluster, pure loopback within the test process.

use futures_util::SinkExt;
use futures_util::StreamExt;
use rsx_tui::conn::GatewayConn;
use rsx_tui::conn::GwEvent;
use rsx_tui::conn::OrderReq;
use rsx_tui::conn::Side;
use rsx_tui::conn::Tif;
use rsx_tui::ws::mint_jwt;
use rsx_tui::ws::WsConn;
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::handshake::server::Request;
use tokio_tungstenite::tungstenite::handshake::server::Response;
use tokio_tungstenite::tungstenite::Message;

/// The oid this server always answers with: 32 hex chars, low 16 of
/// which decode to `7` — matches `WsConn`'s `oid_to_u64` (last 16 hex
/// chars = order_id_lo, see rsx-tui/src/ws.rs).
const TAKER_OID: &str = "00000000000000010000000000000007";
const MAKER_OID: &str = "00000000000000000000000000000000";

/// Accept one connection, check the `Authorization` header made it
/// through the handshake, read one `{N:[...]}` order frame, report
/// it, then echo a `U` (resting) + `F` (fill) frame pair. Holds the
/// socket open briefly after so the client has time to observe both.
async fn run_server(
    listener: TcpListener,
    got_auth: mpsc::Sender<Option<String>>,
    got_frame: mpsc::Sender<Value>,
) {
    let Ok((sock, _addr)) = listener.accept().await else {
        return;
    };
    let auth_header = std::sync::Mutex::new(None::<String>);
    // Err variant (tungstenite ErrorResponse) is fixed by the accept_hdr_async
    // callback signature, not ours.
    #[allow(clippy::result_large_err)]
    let callback = |req: &Request, resp: Response| {
        let auth = req
            .headers()
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        *auth_header.lock().expect("lock") = auth;
        Ok(resp)
    };
    let mut ws = match accept_hdr_async(sock, callback).await {
        Ok(ws) => ws,
        Err(_) => return,
    };
    let _ = got_auth.send(auth_header.lock().expect("lock").clone());

    let Some(Ok(Message::Text(text))) = ws.next().await else {
        return;
    };
    let frame: Value = serde_json::from_str(&text).expect("valid json frame");
    let _ = got_frame.send(frame);

    let accept = serde_json::json!({
        "U": [TAKER_OID, 1, 0, 5, 0],
    });
    if ws.send(Message::Text(accept.to_string())).await.is_err() {
        return;
    }
    let fill = serde_json::json!({
        "F": [TAKER_OID, MAKER_OID, 10_001, 5, 1_700_000_000_000_000_000i64, 0],
    });
    if ws.send(Message::Text(fill.to_string())).await.is_err() {
        return;
    }
    // Hold the connection open until the client closes.
    let _ = ws.next().await;
}

#[test]
fn ws_loopback_roundtrips_order_and_fill() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build test runtime");

    let listener = rt.block_on(async {
        let addr: SocketAddr = "127.0.0.1:0".parse().expect("addr");
        TcpListener::bind(addr).await.expect("bind test listener")
    });
    let addr = listener.local_addr().expect("bound addr");

    let (auth_tx, auth_rx) = mpsc::channel::<Option<String>>();
    let (frame_tx, frame_rx) = mpsc::channel::<Value>();
    rt.spawn(run_server(listener, auth_tx, frame_tx));

    let url = format!("ws://{addr}");
    let token = mint_jwt(1, "test-secret");
    let mut conn = WsConn::connect(url, token.clone(), 10).expect("connect WsConn");

    let want = OrderReq {
        side: Side::Buy,
        price: 10_001,
        qty: 5,
        tif: Tif::Ioc,
    };
    conn.submit(want).expect("submit order");

    let mut saw_connected = false;
    let mut saw_accepted: Option<GwEvent> = None;
    let mut saw_fill: Option<GwEvent> = None;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline
        && (!saw_connected || saw_accepted.is_none() || saw_fill.is_none())
    {
        while let Some(ev) = conn.poll_event() {
            match ev {
                GwEvent::Connected => saw_connected = true,
                GwEvent::Accepted { .. } => saw_accepted = Some(ev),
                GwEvent::Fill { .. } => saw_fill = Some(ev),
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(20));
    }

    assert!(saw_connected, "client never observed Connected");
    assert_eq!(saw_accepted, Some(GwEvent::Accepted { oid: 7 }));
    assert_eq!(
        saw_fill,
        Some(GwEvent::Fill {
            oid: 7,
            px: 10_001,
            qty: 5,
            side: Side::Buy
        }),
    );

    let auth = auth_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("server observed the handshake");
    assert_eq!(auth, Some(format!("Bearer {token}")));

    let frame = frame_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("server received the order frame");
    let arr = frame
        .get("N")
        .and_then(Value::as_array)
        .expect("frame has an N array");
    assert_eq!(arr.len(), 8, "N frame has 8 positional fields");
    assert_eq!(arr[0], Value::from(10), "sym");
    assert_eq!(arr[1], Value::from(0), "side (0 = BUY)");
    assert_eq!(arr[2], Value::from(10_001), "px");
    assert_eq!(arr[3], Value::from(5), "qty");
    let cid = arr[4].as_str().expect("cid is a string");
    assert_eq!(cid.len(), 20, "cid is a fixed 20-char string");
    assert_eq!(arr[5], Value::from(1), "tif (1 = IOC)");
    assert_eq!(arr[6], Value::from(0), "ro defaults to 0");
    assert_eq!(arr[7], Value::from(0), "po defaults to 0");
}
